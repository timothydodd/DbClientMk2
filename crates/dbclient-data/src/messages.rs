//! Captures server info messages (SQL Server `PRINT`, informational RAISERROR) for the query
//! that produced them. tiberius only logs those tokens at DEBUG level through `tracing`, so a
//! global subscriber layer picks them up and files them under the task-local sink of the query
//! being polled.

use std::sync::{Arc, Mutex, Once};
use tracing::field::{Field, Visit};
use tracing::Subscriber;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;

pub type MessageSink = Arc<Mutex<Vec<String>>>;

tokio::task_local! {
    static CURRENT_SINK: MessageSink;
}

static INIT: Once = Once::new();

/// Installs the capturing subscriber once. A subscriber set by someone else is left alone.
pub fn install() {
    INIT.call_once(|| {
        let _ = tracing_subscriber::registry().with(InfoLayer).try_init();
    });
}

/// Runs `fut` with `sink` receiving the info messages tiberius logs while it is polled.
pub async fn with_sink<F: std::future::Future>(sink: MessageSink, fut: F) -> F::Output {
    CURRENT_SINK.scope(sink, fut).await
}

struct InfoLayer;

impl<S: Subscriber + for<'a> LookupSpan<'a>> Layer<S> for InfoLayer {
    fn enabled(&self, metadata: &tracing::Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        metadata.target().starts_with("tiberius::tds::stream::token") && *metadata.level() == tracing::Level::DEBUG
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let Some(message) = visitor.message else { return };
        // Only the Info token line is a bare message; login/feature acks carry other text.
        if message.is_empty() || message.contains(" version ") {
            return;
        }
        let _ = CURRENT_SINK.try_with(|sink| {
            if let Ok(mut v) = sink.lock() {
                v.push(message.clone());
            }
        });
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: Option<String>,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}"));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn captures_tiberius_info_events_for_the_current_task() {
        install();
        let sink: MessageSink = Default::default();
        with_sink(sink.clone(), async {
            tracing::event!(target: "tiberius::tds::stream::token", tracing::Level::DEBUG, "{}", "hello from PRINT");
            tracing::event!(target: "other", tracing::Level::DEBUG, "{}", "ignored");
        })
        .await;
        assert_eq!(sink.lock().unwrap().as_slice(), ["hello from PRINT"]);
    }
}
