//! Debug plumbing for the app: stderr logger, trace file sink and the
//! machine tracer built from CLI flags.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use dosy_core::debug::{TraceSink, Tracer};

/// Instructions kept for fault reports when tracing is on.
pub const RING_DEPTH: usize = 64;

/// `--trace[=FILE]` and `--log-int=..` settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebugSetup {
    /// Keep the last [`RING_DEPTH`] instructions for fault reports.
    pub trace: bool,
    /// Also write every instruction here.
    pub trace_file: Option<PathBuf>,
    /// Log `INT n` calls for these vectors.
    pub log_ints: Vec<u8>,
}

impl DebugSetup {
    /// The machine tracer, or `None` when nothing is enabled (zero cost).
    pub fn tracer(&self) -> std::io::Result<Option<Box<Tracer>>> {
        if !self.trace && self.log_ints.is_empty() {
            return Ok(None);
        }
        let mut t = Tracer::new(if self.trace { RING_DEPTH } else { 0 });
        if let Some(p) = &self.trace_file {
            t = t.with_sink(Box::new(FileSink(BufWriter::new(File::create(p)?))));
        }
        for &v in &self.log_ints {
            t.log_int(v);
        }
        Ok(Some(Box::new(t)))
    }
}

/// Parses `21,10,16` (hex vectors).
pub fn parse_int_list(s: &str) -> Result<Vec<u8>, String> {
    s.split(',')
        .map(|v| {
            let v = v.trim();
            let v = v.strip_suffix(['h', 'H']).unwrap_or(v);
            u8::from_str_radix(v, 16).map_err(|_| format!("bad interrupt number {v:?}"))
        })
        .collect()
}

struct FileSink(BufWriter<File>);

impl TraceSink for FileSink {
    fn line(&mut self, line: &str) {
        // Tracing is best effort; a full disk must not stop emulation.
        let _ = writeln!(self.0, "{line}");
    }
}

/// Minimal stderr logger. Level from `DOSY_LOG` (error/warn/info/debug/
/// trace, default warn); `--log-int` enables info for `dosy::int`.
struct StderrLogger {
    level: log::LevelFilter,
    int_calls: bool,
}

impl log::Log for StderrLogger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= self.level || (self.int_calls && m.target() == "dosy::int")
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("dosy: {} {}: {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

pub fn init_logger(int_calls: bool) {
    let level = std::env::var("DOSY_LOG")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(log::LevelFilter::Warn);
    let max = if int_calls {
        level.max(log::LevelFilter::Info)
    } else {
        level
    };
    let logger: &'static StderrLogger = Box::leak(Box::new(StderrLogger { level, int_calls }));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn int_lists_and_tracer_setup() {
        assert_eq!(parse_int_list("21,10,16h"), Ok(vec![0x21, 0x10, 0x16]));
        assert!(parse_int_list("21,zz").is_err());
        assert!(DebugSetup::default().tracer().unwrap().is_none());
        let t = DebugSetup {
            log_ints: vec![0x21],
            ..Default::default()
        };
        assert!(t.tracer().unwrap().is_some());
    }
}
