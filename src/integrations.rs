#[cfg(feature = "log")]
pub mod log {
    use crate::core::EventKind;

    pub struct EventlineLogger;

    static LOGGER: EventlineLogger = EventlineLogger;

    pub fn init(level: ::log::LevelFilter) -> Result<(), ::log::SetLoggerError> {
        ::log::set_logger(&LOGGER)?;
        ::log::set_max_level(level);
        Ok(())
    }

    impl ::log::Log for EventlineLogger {
        fn enabled(&self, metadata: &::log::Metadata<'_>) -> bool {
            metadata.level() <= ::log::max_level()
        }

        fn log(&self, record: &::log::Record<'_>) {
            if !self.enabled(record.metadata()) {
                return;
            }

            crate::runtime::emit(
                map_level(record.level()),
                record.args().to_string(),
                crate::journal::Fields::new(),
            );
        }

        fn flush(&self) {
            let _ = crate::runtime::flush();
        }
    }

    fn map_level(level: ::log::Level) -> EventKind {
        match level {
            ::log::Level::Error => EventKind::Error,
            ::log::Level::Warn => EventKind::Warning,
            ::log::Level::Info => EventKind::Info,
            ::log::Level::Debug | ::log::Level::Trace => EventKind::Debug,
        }
    }
}

#[cfg(feature = "tracing")]
pub mod tracing {
    use crate::core::EventKind;
    use crate::journal::Fields;
    use ::tracing::field::{Field, Visit};
    use ::tracing::{Event, Level, Subscriber};
    use ::tracing_subscriber::Layer;
    use ::tracing_subscriber::layer::Context;

    #[derive(Debug, Clone, Copy, Default)]
    pub struct EventlineLayer;

    impl<S> Layer<S> for EventlineLayer
    where
        S: Subscriber,
    {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut visitor = EventVisitor::default();
            event.record(&mut visitor);

            let message = visitor
                .message
                .unwrap_or_else(|| event.metadata().name().to_string());

            crate::runtime::emit(map_level(event.metadata().level()), message, visitor.fields);
        }
    }

    #[derive(Default)]
    struct EventVisitor {
        message: Option<String>,
        fields: Fields,
    }

    impl Visit for EventVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.message = Some(format!("{value:?}"));
            } else {
                self.fields.insert(field.name(), format!("{value:?}"));
            }
        }

        fn record_i64(&mut self, field: &Field, value: i64) {
            self.record_value(field, value);
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.record_value(field, value);
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.record_value(field, value);
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "message" {
                self.message = Some(value.to_string());
            } else {
                self.fields.insert(field.name(), value);
            }
        }
    }

    impl EventVisitor {
        fn record_value(&mut self, field: &Field, value: impl Into<crate::core::Value>) {
            if field.name() == "message" {
                self.message = Some(value.into().to_string());
            } else {
                self.fields.insert(field.name(), value);
            }
        }
    }

    fn map_level(level: &Level) -> EventKind {
        match *level {
            Level::ERROR => EventKind::Error,
            Level::WARN => EventKind::Warning,
            Level::INFO => EventKind::Info,
            Level::DEBUG | Level::TRACE => EventKind::Debug,
        }
    }
}
