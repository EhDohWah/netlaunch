//! Log capture for the "no password in logs" check.
//!
//! The log level is set in code from `--log-level`, never from `RUST_LOG`.
//! The same rule applies to the real app: playwright-rs writes the text it
//! types into fields to the log at `trace` level.

use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::FmtSpan;

/// Keeps every log line in memory and also writes it to a file.
#[derive(Clone)]
pub struct Capture {
    buffer: Arc<Mutex<Vec<u8>>>,
    file: Arc<Mutex<File>>,
}

pub struct CaptureWriter(Capture);

impl Write for CaptureWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.buffer.lock().unwrap().extend_from_slice(buf);
        self.0.file.lock().unwrap().write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.file.lock().unwrap().flush()
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = CaptureWriter;

    fn make_writer(&'a self) -> Self::Writer {
        CaptureWriter(self.clone())
    }
}

impl Capture {
    pub fn contains(&self, needle: &str) -> bool {
        let buffer = self.buffer.lock().unwrap();
        String::from_utf8_lossy(&buffer).contains(needle)
    }
}

/// Installs the global subscriber at `level` and returns the capture.
/// Span open/close events are logged too, so span fields (selectors, URLs)
/// are part of what the check searches.
pub fn install(level: &str, log_file: &Path) -> std::io::Result<Capture> {
    let capture = Capture {
        buffer: Arc::new(Mutex::new(Vec::new())),
        file: Arc::new(Mutex::new(File::create(log_file)?)),
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(level))
        .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .init();
    Ok(capture)
}
