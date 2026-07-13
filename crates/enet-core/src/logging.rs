//! Logging initialization with file + stdout sinks.

use crate::config::LogLevel;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initialize structured logging. Keep the returned guard alive for the process lifetime.
pub fn init_logging(level: LogLevel, log_dir: impl AsRef<Path>) -> anyhow::Result<WorkerGuard> {
    let log_dir = log_dir.as_ref();
    std::fs::create_dir_all(log_dir)?;
    let file_appender = tracing_appender::rolling::daily(log_dir, "enet-gateway.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level.as_filter()));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_writer(std::io::stdout))
        .with(fmt::layer().with_ansi(false).with_writer(non_blocking))
        .try_init()
        .ok(); // ignore if already initialized in tests

    Ok(guard)
}
