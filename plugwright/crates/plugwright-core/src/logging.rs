//! Guest log forwarding into `tracing`.

use std::sync::Arc;

use plugwright_abi::{LOG_DEBUG, LOG_ERROR, LOG_INFO, LOG_TRACE, LOG_WARN};

/// Receives `(level, message)` log lines emitted by a plugin.
pub type LogSink = Arc<dyn Fn(u32, &str) + Send + Sync>;

/// Target used for every log line that originates inside a plugin.
pub const GUEST_TARGET: &str = "plugwright::guest";

/// A sink that re-emits guest logs as `tracing` events tagged with the plugin id.
pub fn tracing_sink(id: &str) -> LogSink {
    let id: Arc<str> = id.into();
    Arc::new(move |level, msg| emit(&id, level, msg))
}

/// Re-emit one guest log line as a `tracing` event.
pub fn emit(id: &str, level: u32, msg: &str) {
    match level {
        LOG_ERROR => tracing::error!(target: GUEST_TARGET, plugin = id, "{msg}"),
        LOG_WARN => tracing::warn!(target: GUEST_TARGET, plugin = id, "{msg}"),
        LOG_INFO => tracing::info!(target: GUEST_TARGET, plugin = id, "{msg}"),
        LOG_DEBUG => tracing::debug!(target: GUEST_TARGET, plugin = id, "{msg}"),
        LOG_TRACE => tracing::trace!(target: GUEST_TARGET, plugin = id, "{msg}"),
        _ => tracing::info!(target: GUEST_TARGET, plugin = id, level, "{msg}"),
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::Mutex;

    use plugwright_abi::{LOG_DEBUG, LOG_ERROR, LOG_INFO, LOG_TRACE, LOG_WARN};
    use tracing::field::{Field, Visit};
    use tracing::{Event, Level, Subscriber};
    use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    struct Captured {
        level: Level,
        target: String,
        fields: Vec<(String, String)>,
    }

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Captured>>>);

    struct Fields(Vec<(String, String)>);

    impl Visit for Fields {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().into(), value.into()));
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.push((field.name().into(), format!("{value:?}")));
        }
    }

    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut fields = Fields(Vec::new());
            event.record(&mut fields);
            let meta = event.metadata();
            self.0.lock().unwrap().push(Captured {
                level: *meta.level(),
                target: meta.target().into(),
                fields: fields.0,
            });
        }
    }

    fn capture(f: impl FnOnce()) -> Vec<Captured> {
        let capture = Capture::default();
        let subscriber = tracing_subscriber::registry().with(capture.clone());
        tracing::subscriber::with_default(subscriber, f);
        capture.0.lock().unwrap().clone()
    }

    fn field<'a>(e: &'a Captured, name: &str) -> Option<&'a str> {
        e.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    #[test]
    fn guest_levels_map_to_tracing_levels() {
        let cases = [
            (LOG_ERROR, Level::ERROR),
            (LOG_WARN, Level::WARN),
            (LOG_INFO, Level::INFO),
            (LOG_DEBUG, Level::DEBUG),
            (LOG_TRACE, Level::TRACE),
        ];
        for (guest, level) in cases {
            let events = capture(|| emit("demo", guest, "hello"));
            assert_eq!(events.len(), 1, "level {guest}");
            let e = &events[0];
            assert_eq!(e.level, level, "level {guest}");
            assert_eq!(e.target, GUEST_TARGET);
            assert_eq!(field(e, "plugin"), Some("demo"));
            assert_eq!(field(e, "message"), Some("hello"));
            assert_eq!(field(e, "level"), None, "known levels carry no raw level field");
        }
    }

    #[test]
    fn unknown_guest_levels_log_at_info_with_the_raw_level() {
        for raw in [0, 6, u32::MAX] {
            let events = capture(|| emit("demo", raw, "odd"));
            assert_eq!(events.len(), 1);
            let e = &events[0];
            assert_eq!(e.level, Level::INFO);
            assert_eq!(e.target, GUEST_TARGET);
            assert_eq!(field(e, "level"), Some(raw.to_string().as_str()));
            assert_eq!(field(e, "message"), Some("odd"));
        }
    }

    #[test]
    fn messages_are_not_treated_as_format_strings() {
        let events = capture(|| emit("demo", LOG_INFO, "{} {0} %s"));
        assert_eq!(field(&events[0], "message"), Some("{} {0} %s"));
    }

    #[test]
    fn tracing_sink_tags_lines_with_the_plugin_id() {
        let sink = tracing_sink("my-plugin");
        let events = capture(|| {
            sink(LOG_WARN, "first");
            sink(LOG_ERROR, "second");
        });
        let got: Vec<_> = events.iter().map(|e| (e.level, field(e, "plugin"), field(e, "message"))).collect();
        assert_eq!(
            got,
            [(Level::WARN, Some("my-plugin"), Some("first")), (Level::ERROR, Some("my-plugin"), Some("second"))]
        );
    }

    #[test]
    fn guest_target_is_stable() {
        assert_eq!(GUEST_TARGET, "plugwright::guest");
    }
}
