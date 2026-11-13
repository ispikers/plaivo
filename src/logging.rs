use crate::store;
use log::{Level, LevelFilter, Log, Metadata, Record};
use plaivo_core::redact;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_BYTES: usize = 1 << 20;
const MAX_LINE: usize = 16 << 10;

static LINES: Mutex<(VecDeque<String>, usize)> = Mutex::new((VecDeque::new(), 0));

struct Logger {
    console: bool,
}

fn log_dir() -> PathBuf {
    store::cache_dir().with_file_name("logs")
}

pub fn len() -> usize {
    LINES.lock().map_or(0, |l| l.0.len())
}

pub fn lines(range: std::ops::Range<usize>) -> Vec<String> {
    LINES
        .lock()
        .map(|l| {
            let n = l.0.len();
            l.0.range(range.start.min(n)..range.end.min(n))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Writes the in-memory log to a text file in the log folder and returns its path.
pub fn export() -> std::io::Result<PathBuf> {
    let dir = log_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("plaivo-log.txt");
    let mut text = format!(
        "Plaivo {} on {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS
    );
    for line in lines(0..usize::MAX) {
        text.push_str(&line);
        text.push('\n');
    }
    std::fs::write(&path, text)?;
    Ok(path)
}

fn push(mut line: String) {
    if line.len() > MAX_LINE {
        let mut end = MAX_LINE;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
        line.push('…');
    }
    if let Ok(mut l) = LINES.lock() {
        let (lines, bytes) = &mut *l;
        *bytes += line.len();
        lines.push_back(line);
        while *bytes > MAX_BYTES
            && let Some(old) = lines.pop_front()
        {
            *bytes -= old.len();
        }
    }
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    s.trim().parse().ok()
}

fn default_level(debug: bool) -> LevelFilter {
    std::env::var("PLAIVO_LOG")
        .ok()
        .and_then(|v| parse_level(&v))
        .unwrap_or(if debug || cfg!(debug_assertions) {
            LevelFilter::Debug
        } else {
            LevelFilter::Info
        })
}

pub fn set_debug(debug: bool) {
    let level = default_level(debug);
    log::set_max_level(level);
    log::info!("log level {level}");
}

pub fn init(debug: bool) {
    let level = default_level(debug);
    for old in ["plaivo.log", "plaivo.old.log"] {
        let _ = std::fs::remove_file(log_dir().join(old));
    }
    let logger = Logger {
        console: cfg!(debug_assertions),
    };
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(level);
    }
    log::info!(
        "Plaivo {} starting, log level {level}",
        env!("CARGO_PKG_VERSION")
    );
}

fn clock() -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let s = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        s / 3600 % 24,
        s / 60 % 60,
        s % 60,
        ms % 1000
    )
}

impl Log for Logger {
    fn enabled(&self, m: &Metadata) -> bool {
        let ours = m.target().starts_with("plaivo") || m.target() == "mpv";
        m.level()
            <= if ours {
                log::max_level()
            } else {
                log::max_level().min(LevelFilter::Warn)
            }
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = redact(&format!(
            "{} {:5} {}: {}",
            clock(),
            r.level(),
            r.target(),
            r.args()
        ));
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
        push(line);
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_parse_case_insensitively() {
        assert_eq!(parse_level("TRACE"), Some(LevelFilter::Trace));
        assert_eq!(parse_level(" warn "), Some(LevelFilter::Warn));
        assert_eq!(parse_level("loud"), None);
    }

    #[test]
    fn buffer_stays_within_its_byte_budget() {
        let line = "é".repeat(MAX_LINE);
        for _ in 0..MAX_BYTES / MAX_LINE * 2 {
            push(line.clone());
        }
        push("newest".into());
        let all = lines(0..usize::MAX);
        assert!(all.iter().map(String::len).sum::<usize>() <= MAX_BYTES);
        assert!(all[0].len() <= MAX_LINE + '…'.len_utf8());
        assert_eq!(all.last().unwrap(), "newest");
        assert_eq!(lines(len() - 1..len() + 9), ["newest"]);
    }
}
