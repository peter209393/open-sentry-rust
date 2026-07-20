use std::{collections::BTreeMap, env, error::Error, fmt, panic, thread, time::Duration};

use sentry::{
    Level,
    protocol::{Attachment, Breadcrumb, Context, Event, User, Value},
};

const DEFAULT_DSN: &str = "http://dev-secret@127.0.0.1:8080/1";

#[derive(Debug)]
struct SmokeError;

impl fmt::Display for SmokeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("intentional sentry smoke-test error")
    }
}

impl Error for SmokeError {}

fn main() {
    let command = env::args().nth(1).unwrap_or_else(|| "all".into());
    if command == "help" || command == "--help" || command == "-h" {
        print_help();
        return;
    }

    let dsn = env::var("SENTRY_DSN").unwrap_or_else(|_| DEFAULT_DSN.into());
    let guard = sentry::init((
        dsn.as_str(),
        sentry::ClientOptions {
            release: Some("open-sentry-smoke@0.1.0".into()),
            environment: Some("local-development".into()),
            traces_sample_rate: 1.0,
            enable_logs: true,
            send_default_pii: true,
            shutdown_timeout: Duration::from_secs(10),
            debug: env::var_os("SENTRY_DEBUG").is_some(),
            ..Default::default()
        },
    ));
    if !guard.is_enabled() {
        eprintln!("Sentry client is disabled; check SENTRY_DSN");
        std::process::exit(2);
    }

    configure_common_scope();
    match command.as_str() {
        "all" => emit_all(),
        "message" => emit_message(),
        "error" => emit_error(),
        "event" => emit_structured_event(),
        "attachment" => emit_attachment(),
        "transaction" => emit_transaction(),
        "log" => emit_log(),
        "panic" => emit_panic(),
        unknown => {
            eprintln!("unknown command: {unknown}");
            print_help();
            std::process::exit(2);
        }
    }

    drop(guard);
    println!("Sentry smoke payloads flushed to {dsn}");
}

fn configure_common_scope() {
    sentry::configure_scope(|scope| {
        scope.set_tag("suite", "full-sdk-coverage");
        scope.set_tag("service", "checkout-api");
        scope.set_extra("command", Value::String("sentry-smoke".into()));
        scope.set_user(Some(User {
            id: Some("local-user-42".into()),
            username: Some("smoke-tester".into()),
            email: Some("smoke@example.test".into()),
            ip_address: Some("127.0.0.1".parse().expect("valid IP")),
            ..Default::default()
        }));
        scope.set_context(
            "smoke",
            Context::Other(BTreeMap::from([
                ("coverage".into(), Value::String("full".into())),
                ("local".into(), Value::Bool(true)),
            ])),
        );
    });
    sentry::add_breadcrumb(Breadcrumb {
        category: Some("smoke.lifecycle".into()),
        message: Some("smoke suite started".into()),
        level: Level::Info,
        ..Default::default()
    });
}

fn emit_all() {
    emit_message();
    emit_error();
    emit_structured_event();
    emit_attachment();
    emit_transaction();
    emit_log();
    emit_panic();
}

fn emit_message() {
    let id = sentry::capture_message("open-sentry SDK message smoke test", Level::Warning);
    println!("message event_id={id}");
}

fn emit_error() {
    let id = sentry::capture_error(&SmokeError);
    println!("error event_id={id}");
}

fn emit_structured_event() {
    let mut event = Event {
        message: Some("open-sentry structured event smoke test".into()),
        level: Level::Error,
        transaction: Some("checkout.submit".into()),
        ..Default::default()
    };
    event.tags.insert("component".into(), "cli".into());
    event.extra.insert("attempt".into(), Value::from(1));
    let id = sentry::capture_event(event);
    println!("structured event_id={id}");
}

fn emit_attachment() {
    let id = sentry::with_scope(
        |scope| {
            scope.add_attachment(Attachment {
                buffer: b"open-sentry attachment smoke payload\n".to_vec(),
                filename: "smoke.txt".into(),
                content_type: Some("text/plain".into()),
                ..Default::default()
            });
        },
        || sentry::capture_message("event carrying an attachment", Level::Info),
    );
    println!("attachment parent event_id={id}");
}

fn emit_transaction() {
    let transaction = sentry::start_transaction(sentry::TransactionContext::new(
        "smoke.transaction",
        "smoke.command",
    ));
    transaction.set_tag("component", "cli");
    transaction.set_data("records", Value::from(3));
    let span = transaction.start_child("db.query", "SELECT smoke coverage");
    thread::sleep(Duration::from_millis(5));
    span.finish();
    transaction.finish();
    println!("transaction emitted");
}

fn emit_log() {
    sentry::logger_info!(
        service.name = "checkout-api",
        suite = "full-sdk-coverage",
        "structured SDK log"
    );
    sentry::logger_warn!(
        service.name = "checkout-api",
        attempt = 1,
        "warning SDK log"
    );
    sentry::logger_error!(
        service.name = "checkout-api",
        recoverable = true,
        "error SDK log"
    );
    println!("logs emitted");
}

fn emit_panic() {
    let result = panic::catch_unwind(|| panic!("intentional caught smoke-test panic"));
    assert!(result.is_err());
    println!("caught panic emitted");
}

fn print_help() {
    println!(
        "Usage: cargo run --bin sentry-smoke -- [all|message|error|event|attachment|transaction|log|panic]\n\
         DSN defaults to {DEFAULT_DSN}; override it with SENTRY_DSN."
    );
}

#[cfg(test)]
mod tests {
    use sentry::protocol::{EnvelopeItem, ItemContainer};

    use super::*;

    fn test_options() -> sentry::ClientOptions {
        sentry::ClientOptions {
            traces_sample_rate: 1.0,
            enable_logs: true,
            ..Default::default()
        }
        .add_integration(sentry::integrations::panic::PanicIntegration::new())
    }

    #[test]
    fn all_emitters_build_the_expected_sdk_envelope_types() {
        let envelopes = sentry::test::with_captured_envelopes_options(
            || {
                configure_common_scope();
                emit_message();
                emit_error();
                emit_structured_event();
                emit_attachment();
                emit_transaction();
                emit_log();
            },
            test_options(),
        );

        let mut events = 0;
        let mut attachments = 0;
        let mut transactions = 0;
        let mut logs = 0;
        for item in envelopes.iter().flat_map(|envelope| envelope.items()) {
            match item {
                EnvelopeItem::Event(_) => events += 1,
                EnvelopeItem::Attachment(_) => attachments += 1,
                EnvelopeItem::Transaction(_) => transactions += 1,
                EnvelopeItem::ItemContainer(ItemContainer::Logs(_)) => logs += 1,
                _ => {}
            }
        }
        assert_eq!(events, 4);
        assert_eq!(attachments, 1);
        assert_eq!(transactions, 1);
        assert!(logs >= 1);
    }

    #[test]
    fn panic_integration_captures_a_caught_panic() {
        let envelopes = sentry::test::with_captured_envelopes_options(emit_panic, test_options());
        assert!(
            envelopes.iter().flat_map(|envelope| envelope.items()).any(
                |item| matches!(item, EnvelopeItem::Event(event) if !event.exception.is_empty())
            )
        );
    }
}
