use crate::store;
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_BYTES: u64 = 4 << 20;

struct Logger {
    level: LevelFilter,
    console: bool,
    file: Option<Mutex<File>>,
}

pub fn log_path() -> PathBuf {
    store::cache_dir().with_file_name("logs").join("plaivo.log")
}

fn open_log() -> Option<File> {
    let path = log_path();
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, path.with_extension("old.log"));
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn redact(line: &str) -> String {
    const KEY: &str = "X-Plex-Token";
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(KEY) {
        let after = &rest[i + KEY.len()..];
        let Some(sep) = after.chars().next().filter(|c| *c == '=' || *c == ':') else {
            out.push_str(&rest[..i + KEY.len()]);
            rest = after;
            continue;
        };
        let value = after[1..].trim_start();
        let skipped = after.len() - value.len();
        let end = value.find(|c: char| c == '&' || c == '"' || c == '\'' || c.is_whitespace()).unwrap_or(value.len());
        out.push_str(&rest[..i + KEY.len()]);
        out.push(sep);
        out.push_str("<redacted>");
        rest = &after[skipped + end..];
    }
    out.push_str(rest);
    out
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    s.trim().parse().ok()
}

pub fn init() {
    let level = std::env::var("PLAIVO_LOG").ok().and_then(|v| parse_level(&v)).unwrap_or(if cfg!(debug_assertions) { LevelFilter::Debug } else { LevelFilter::Info });
    let logger = Logger { level, console: cfg!(debug_assertions), file: open_log().map(Mutex::new) };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    log::info!("Plaivo {} starting, log level {level}, file {}", env!("CARGO_PKG_VERSION"), log_path().display());
}

fn clock() -> String {
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}.{:03}Z", s / 3600 % 24, s / 60 % 60, s % 60, ms % 1000)
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        let ours = m.target().starts_with("plaivo") || m.target() == "mpv";
        m.level() <= if ours { self.level } else { self.level.min(LevelFilter::Warn) }
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = redact(&format!("{} {:5} {}: {}", clock(), r.level(), r.target(), r.args()));
        if self.console {
            let color = match r.level() {
                Level::Error => "31",
                Level::Warn => "33",
                Level::Info => "32",
                Level::Debug => "36",
                Level::Trace => "90",
            };
            eprintln!("\x1b[{color}m{line}\x1b[0m");
        }
        if let Some(f) = &self.file
            && let Ok(mut f) = f.lock()
        {
            let _ = writeln!(f, "{line}");
        }
    }

    fn flush(&self) {
        if let Some(f) = &self.file
            && let Ok(mut f) = f.lock()
        {
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_never_reach_the_log() {
        assert_eq!(redact("GET http://h/a?X-Plex-Token=abc123&b=1 ok"), "GET http://h/a?X-Plex-Token=<redacted>&b=1 ok");
        assert_eq!(redact("hdr X-Plex-Token: s3cr3t done"), "hdr X-Plex-Token:<redacted> done");
        assert_eq!(redact("no secrets here"), "no secrets here");
    }

    #[test]
    fn levels_parse_case_insensitively() {
        assert_eq!(parse_level("TRACE"), Some(LevelFilter::Trace));
        assert_eq!(parse_level(" warn "), Some(LevelFilter::Warn));
        assert_eq!(parse_level("loud"), None);
    }
}
