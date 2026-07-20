use std::{sync::Arc, time::Duration};

use object::{Object, ObjectSymbol};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use crate::state::AppState;

pub fn validate_debug_file(kind: &str, payload: &[u8]) -> anyhow::Result<()> {
    match kind {
        "source_map" => {
            sourcemap::decode_slice(payload)?;
            Ok(())
        }
        "native_symbol"
            if payload.starts_with(b"\x7fELF")
                || payload.starts_with(&[0xfe, 0xed, 0xfa])
                || payload.starts_with(b"Microsoft C/C++") =>
        {
            Ok(())
        }
        "native_symbol" => anyhow::bail!("unrecognized ELF, Mach-O, or PDB symbol file"),
        _ => anyhow::bail!("kind must be source_map or native_symbol"),
    }
}

pub async fn process_event(db: &PgPool, event_id: Uuid) -> anyhow::Result<bool> {
    let (project_id, release, exception) = sqlx::query_as::<_, (Uuid, Option<String>, Value)>(
        "SELECT project_id,release,exception FROM events WHERE id=$1",
    )
    .bind(event_id)
    .fetch_one(db)
    .await?;
    let Some(release) = release else {
        anyhow::bail!("event has no release")
    };
    let maps = sqlx::query_as::<_, (String, Vec<u8>)>(
        r#"SELECT d.name,d.payload FROM debug_files d
        JOIN releases r ON r.id=d.release_id WHERE d.project_id=$1 AND r.version=$2
        AND d.kind='source_map' AND d.status='ready'"#,
    )
    .bind(project_id)
    .bind(&release)
    .fetch_all(db)
    .await?;
    let native = sqlx::query_as::<_, (String, Vec<u8>)>(r#"SELECT d.name,d.payload FROM debug_files d JOIN releases r ON r.id=d.release_id WHERE d.project_id=$1 AND r.version=$2 AND d.kind='native_symbol' AND d.status='ready'"#).bind(project_id).bind(&release).fetch_all(db).await?;
    if maps.is_empty() && native.is_empty() {
        anyhow::bail!("no debug file matches event release")
    }
    let mut output = exception;
    let frames = output
        .get_mut("values")
        .and_then(Value::as_array_mut)
        .and_then(|v| v.first_mut())
        .and_then(|v| v.get_mut("stacktrace"))
        .and_then(|v| v.get_mut("frames"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| anyhow::anyhow!("event has no stack frames"))?;
    let mut changed = false;
    for frame in frames {
        let filename = frame.get("filename").and_then(Value::as_str).unwrap_or("");
        let line = frame
            .get("lineno")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .saturating_sub(1) as u32;
        let col = frame
            .get("colno")
            .and_then(Value::as_u64)
            .unwrap_or(1)
            .saturating_sub(1) as u32;
        if let Some((_, payload)) = maps
            .iter()
            .find(|(name, _)| filename.ends_with(name.trim_end_matches(".map")))
        {
            let decoded = sourcemap::decode_slice(payload)?;
            if let Some(token) = decoded.lookup_token(line, col)
                && let Some(obj) = frame.as_object_mut()
            {
                if let Some(source) = token.get_source() {
                    obj.insert("filename".into(), source.into());
                }
                if let Some(name) = token.get_name() {
                    obj.insert("function".into(), name.into());
                }
                obj.insert("lineno".into(), ((token.get_src_line() + 1) as u64).into());
                obj.insert("colno".into(), ((token.get_src_col() + 1) as u64).into());
                changed = true;
            }
        }
        if let Some(address) = frame
            .get("instruction_addr")
            .and_then(Value::as_str)
            .and_then(|v| u64::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        {
            for (_, payload) in &native {
                let Ok(file) = object::File::parse(payload.as_slice()) else {
                    continue;
                };
                let symbol = file
                    .symbols()
                    .filter(|s| {
                        s.address() <= address
                            && s.address().saturating_add(s.size().max(1)) > address
                    })
                    .max_by_key(|s| s.address());
                if let Some(symbol) = symbol.and_then(|s| s.name().ok()) {
                    if let Some(obj) = frame.as_object_mut() {
                        obj.insert("function".into(), symbol.into());
                        changed = true;
                    }
                    break;
                }
            }
        }
    }
    if !changed {
        anyhow::bail!("no stack frame matched uploaded source maps")
    }
    sqlx::query(
        "UPDATE events SET symbolicated_exception=$2,symbolication_status='complete' WHERE id=$1",
    )
    .bind(event_id)
    .bind(output)
    .execute(db)
    .await?;
    Ok(true)
}

pub async fn run_worker(state: Arc<AppState>) {
    loop {
        if let Ok(Some(event_id))=sqlx::query_scalar::<_,Uuid>("UPDATE symbolication_jobs SET status='processing',attempts=attempts+1 WHERE id=(SELECT id FROM symbolication_jobs WHERE status IN ('pending','failed') AND available_at<=now() ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1) RETURNING event_id").fetch_optional(&state.db).await {
            match process_event(&state.db,event_id).await {
                Ok(_)=>{let _=sqlx::query("UPDATE symbolication_jobs SET status='complete',completed_at=now(),last_error=NULL WHERE event_id=$1").bind(event_id).execute(&state.db).await;}
                Err(error)=>{let _=sqlx::query("UPDATE symbolication_jobs SET status='failed',last_error=$2,available_at=now()+interval '5 minutes' WHERE event_id=$1").bind(event_id).bind(error.to_string()).execute(&state.db).await;}
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_source_map_and_rejects_random_native_file() {
        assert!(
            validate_debug_file(
                "source_map",
                br#"{"version":3,"sources":["src.ts"],"names":[],"mappings":"AAAA"}"#
            )
            .is_ok()
        );
        assert!(validate_debug_file("native_symbol", b"random").is_err());
    }
}
