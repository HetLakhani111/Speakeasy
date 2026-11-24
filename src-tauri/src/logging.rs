use std::fs;
use std::path::PathBuf;

use tauri::{AppHandle, Manager};
use tracing_subscriber::EnvFilter;

use crate::error::{io_error, Error, Result};

/// Holds the non-blocking writer for the log file. Dropping it flushes any
/// buffered lines, so it lives in application state until exit.
pub struct Logging {
    pub dir: PathBuf,
    pub guard: tracing_appender::non_blocking::WorkerGuard,
}

pub fn init(app: &AppHandle) -> Result<Logging> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|err| Error::Path(format!("log directory: {err}")))?;

    fs::create_dir_all(&dir).map_err(|err| io_error(&dir, err))?;

    let appender = tracing_appender::rolling::daily(&dir, "speakeasy.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("speakeasy=info,tauri=warn,wry=warn"));

    // A second init attempt (e.g. after a reload in development) is harmless.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .try_init();

    Ok(Logging { dir, guard })
}
