use std::{
    env,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use reqwest::{Client, StatusCode};
use serde_json::json;
use tokio::{sync::Semaphore, task::JoinSet};
use uuid::Uuid;

#[derive(Debug)]
struct Config {
    url: String,
    api_key: String,
    requests: usize,
    concurrency: usize,
    max_error_rate: f64,
    max_p95_ms: u128,
}

impl Config {
    fn load() -> anyhow::Result<Self> {
        let requests = number("LOAD_REQUESTS", 200)?;
        let concurrency = number("LOAD_CONCURRENCY", 20)?;
        if requests == 0 || concurrency == 0 {
            bail!("request and concurrency values must be positive");
        }
        Ok(Self {
            url: env::var("LOAD_URL").unwrap_or_else(|_| {
                "http://127.0.0.1:8080/api/projects/00000000-0000-0000-0000-000000000001/store"
                    .into()
            }),
            api_key: env::var("LOAD_API_KEY").unwrap_or_else(|_| "dev-secret".into()),
            requests,
            concurrency,
            max_error_rate: env::var("LOAD_MAX_ERROR_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.01),
            max_p95_ms: number("LOAD_MAX_P95_MS", 500)?,
        })
    }
}

fn number<T: std::str::FromStr>(name: &str, default: T) -> anyhow::Result<T>
where
    T::Err: std::error::Error + Send + Sync + 'static,
{
    env::var(name)
        .map(|v| v.parse().with_context(|| format!("invalid {name}")))
        .unwrap_or(Ok(default))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::load()?;
    let client = Client::builder()
        .pool_max_idle_per_host(config.concurrency)
        .timeout(Duration::from_secs(10))
        .build()?;
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let started = Instant::now();
    let mut tasks = JoinSet::new();
    for sequence in 0..config.requests {
        let permit = semaphore.clone().acquire_owned().await?;
        let client = client.clone();
        let url = config.url.clone();
        let key = config.api_key.clone();
        tasks.spawn(async move { let _permit=permit; let before=Instant::now(); let response=client.post(url).header("x-sentry-auth",key).json(&json!({"event_id":Uuid::new_v4(),"level":"error","message":"production load acceptance","environment":"acceptance","tags":{"service":"load-smoke","sequence":sequence}})).send().await; (before.elapsed(),response.map(|r|r.status())) });
    }
    let mut latencies = Vec::with_capacity(config.requests);
    let mut accepted = 0usize;
    let mut failed = 0usize;
    while let Some(result) = tasks.join_next().await {
        let (latency, status) = result?;
        latencies.push(latency.as_millis());
        match status {
            Ok(StatusCode::ACCEPTED) => accepted += 1,
            _ => failed += 1,
        }
    }
    latencies.sort_unstable();
    let elapsed = started.elapsed();
    let p50 = percentile(&latencies, 0.50);
    let p95 = percentile(&latencies, 0.95);
    let p99 = percentile(&latencies, 0.99);
    let error_rate = failed as f64 / config.requests as f64;
    let throughput = config.requests as f64 / elapsed.as_secs_f64();
    println!(
        "requests={} accepted={} failed={} error_rate={:.4} throughput_rps={:.1} p50_ms={} p95_ms={} p99_ms={} elapsed_ms={}",
        config.requests,
        accepted,
        failed,
        error_rate,
        throughput,
        p50,
        p95,
        p99,
        elapsed.as_millis()
    );
    if error_rate > config.max_error_rate {
        bail!(
            "error rate {:.4} exceeds {:.4}",
            error_rate,
            config.max_error_rate
        );
    }
    if p95 > config.max_p95_ms {
        bail!("p95 {p95}ms exceeds {}ms", config.max_p95_ms);
    }
    Ok(())
}

fn percentile(sorted: &[u128], quantile: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) as f64 * quantile).ceil() as usize;
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn percentile_handles_boundaries() {
        let values = [1, 2, 3, 4, 100];
        assert_eq!(percentile(&values, 0.0), 1);
        assert_eq!(percentile(&values, 0.5), 3);
        assert_eq!(percentile(&values, 0.95), 100);
        assert_eq!(percentile(&[], 0.95), 0);
    }
}
