//! Structured session logs and crash-report primitives.
//!
//! The module never initializes global logging. The composition root owns its
//! lifetime and can send records from the primary event loop in P1.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A unique identifier for one Horyzond process session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionId(String);

impl SessionId {
    /// Generates an identifier from the wall clock, process ID, and an atomic sequence.
    #[must_use]
    pub fn generate() -> Self {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let milliseconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(format!("{milliseconds}-{}-{sequence}", std::process::id()))
    }

    /// Returns a filesystem-safe textual representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Severity attached to a structured diagnostic record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// One active session log at `<config-root>/latest.log`.
#[derive(Debug)]
pub struct SessionLogger {
    session_id: SessionId,
    latest_log: PathBuf,
    crashes_dir: PathBuf,
    active_marker: PathBuf,
    writer: BufWriter<File>,
    started_at: std::time::Instant,
    sequence: u64,
}

impl SessionLogger {
    /// Archives an existing active log and opens a new structured session log.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when required directories cannot be created, an
    /// existing log cannot be archived, or the new log cannot be opened.
    pub fn open(config_root: &Path, session_id: SessionId) -> Result<Self, DiagnosticsError> {
        fs::create_dir_all(config_root.join("logs"))?;
        let crashes_dir = config_root.join("crashes");
        fs::create_dir_all(&crashes_dir)?;
        let latest_log = config_root.join("latest.log");
        let active_marker = config_root.join(".session-active");
        let previous_session_unclean = active_marker.exists();
        if previous_session_unclean {
            fs::remove_file(&active_marker)?;
        }
        let mut marker = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&active_marker)?;
        writeln!(marker, "session_id={}", session_id.as_str())?;
        marker.sync_all()?;
        if latest_log.exists() {
            let archive = unique_path(&config_root.join("logs"), "previous", "log");
            fs::rename(&latest_log, archive)?;
        }
        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&latest_log)?;
        let mut logger = Self {
            session_id,
            latest_log,
            crashes_dir,
            active_marker,
            writer: BufWriter::new(file),
            started_at: std::time::Instant::now(),
            sequence: 0,
        };
        logger.record(LogLevel::Info, "diagnostics", "session_started")?;
        if previous_session_unclean {
            logger.record(LogLevel::Warn, "diagnostics", "previous_session_unclean")?;
        }
        Ok(logger)
    }

    /// Returns this logger's session ID.
    #[must_use]
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Returns the active `latest.log` path.
    #[must_use]
    pub fn latest_log_path(&self) -> &Path {
        &self.latest_log
    }

    /// Writes and flushes one newline-delimited JSON diagnostic event.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the record cannot be written or flushed.
    pub fn record(
        &mut self,
        level: LogLevel,
        module: &str,
        message: &str,
    ) -> Result<(), DiagnosticsError> {
        self.sequence += 1;
        let elapsed = self.started_at.elapsed().as_micros();
        writeln!(
            self.writer,
            "{{\"session_id\":\"{}\",\"sequence\":{},\"elapsed_us\":{},\"level\":\"{}\",\"module\":\"{}\",\"message\":\"{}\"}}",
            escape_json(self.session_id.as_str()),
            self.sequence,
            elapsed,
            level.as_str(),
            escape_json(module),
            escape_json(message)
        )?;
        self.writer.flush()?;
        Ok(())
    }

    /// Writes a crash report correlated with the active session.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the report cannot be created or written.
    pub fn write_crash_report(&mut self, summary: &str) -> Result<PathBuf, DiagnosticsError> {
        self.record(LogLevel::Error, "diagnostics", summary)?;
        let path = unique_path(&self.crashes_dir, self.session_id.as_str(), "json");
        let mut report = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;
        writeln!(
            report,
            "{{\"session_id\":\"{}\",\"summary\":\"{}\",\"latest_log\":\"{}\"}}",
            escape_json(self.session_id.as_str()),
            escape_json(summary),
            escape_json(&self.latest_log.display().to_string())
        )?;
        report.flush()?;
        Ok(path)
    }

    /// Marks a clean shutdown before the logger is dropped.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the closing record cannot be persisted.
    pub fn close(mut self) -> Result<(), DiagnosticsError> {
        self.record(LogLevel::Info, "diagnostics", "session_stopped")?;
        fs::remove_file(self.active_marker)?;
        Ok(())
    }
}

#[derive(Debug)]
struct PanicContext {
    crashes_dir: PathBuf,
    session_id: SessionId,
    latest_log: PathBuf,
}

static PANIC_CONTEXT: OnceLock<PanicContext> = OnceLock::new();

/// Installs the one process-wide panic hook used to write a crash report.
///
/// Repeated calls leave the first session context in place. Fatal signals and
/// abrupt termination cannot run this hook; P1 records those limits in the
/// documentation and relies on the unclean-session marker in later work.
pub fn install_panic_hook(config_root: &Path, session_id: SessionId) {
    let context = PanicContext {
        crashes_dir: config_root.join("crashes"),
        latest_log: config_root.join("latest.log"),
        session_id,
    };
    if PANIC_CONTEXT.set(context).is_err() {
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        let Some(context) = PANIC_CONTEXT.get() else {
            return;
        };
        let summary = info.to_string();
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();
        let path = unique_path(&context.crashes_dir, context.session_id.as_str(), "json");
        if let Ok(mut report) = OpenOptions::new().create_new(true).write(true).open(path) {
            let _ = writeln!(
                report,
                "{{\"session_id\":\"{}\",\"summary\":\"{}\",\"backtrace\":\"{}\",\"latest_log\":\"{}\"}}",
                escape_json(context.session_id.as_str()),
                escape_json(&summary),
                escape_json(&backtrace),
                escape_json(&context.latest_log.display().to_string())
            );
            let _ = report.flush();
        }
    }));
}

fn unique_path(directory: &Path, prefix: &str, extension: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    for attempt in 0_u32..10_000 {
        let path = directory.join(format!(
            "{prefix}-{millis}-{}-{attempt}.{extension}",
            std::process::id()
        ));
        if !path.exists() {
            return path;
        }
    }
    directory.join(format!(
        "{prefix}-{millis}-{}-overflow.{extension}",
        std::process::id()
    ))
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// A diagnostics I/O failure.
#[derive(Debug)]
pub struct DiagnosticsError(io::Error);

impl From<io::Error> for DiagnosticsError {
    fn from(value: io::Error) -> Self {
        Self(value)
    }
}
impl fmt::Display for DiagnosticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl std::error::Error for DiagnosticsError {}

#[cfg(test)]
mod tests {
    use super::{LogLevel, SessionId, SessionLogger};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "horyzond-diagnostics-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn archives_previous_log_and_writes_crash_report() {
        let root = temporary_root();
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("latest.log"), "previous session\n").expect("previous log");
        let mut logger = SessionLogger::open(&root, SessionId::generate()).expect("open logger");
        logger
            .record(LogLevel::Trace, "test", "event")
            .expect("record");
        let report = logger.write_crash_report("test crash").expect("report");
        assert!(
            root.join("logs")
                .read_dir()
                .expect("archives")
                .next()
                .is_some()
        );
        assert!(report.exists());
        assert!(
            fs::read_to_string(logger.latest_log_path())
                .expect("latest")
                .contains("session_started")
        );
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn marks_an_unclean_previous_session() {
        let root = temporary_root();
        let abandoned =
            SessionLogger::open(&root, SessionId::generate()).expect("open abandoned logger");
        drop(abandoned);
        let logger =
            SessionLogger::open(&root, SessionId::generate()).expect("open recovery logger");
        let contents = fs::read_to_string(logger.latest_log_path()).expect("latest log");
        assert!(contents.contains("previous_session_unclean"));
        logger.close().expect("clean close");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
