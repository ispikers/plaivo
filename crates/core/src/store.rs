use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

static DIRS: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

pub fn set_dirs(config: PathBuf, cache: PathBuf) {
    let _ = DIRS.set((config, cache));
}

pub fn config_dir() -> PathBuf {
    if let Some((config, _)) = DIRS.get() {
        return config.clone();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let base: Option<PathBuf> = None;
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    base.unwrap_or_else(|| ".".into()).join("Plaivo")
}

pub fn cache_dir() -> PathBuf {
    if let Some((_, cache)) = DIRS.get() {
        return cache.clone();
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let base: Option<PathBuf> = None;
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("Plaivo"));
    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Caches/Plaivo"));
    base.unwrap_or_else(|| config_dir().join("cache"))
        .join("cache")
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let parsed = match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        r => r
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())),
    };
    parsed
        .inspect_err(|e| {
            let mut bad = path.as_os_str().to_owned();
            bad.push(".bad");
            let bad = PathBuf::from(bad);
            log::warn!(
                "could not read {}: {e}; keeping a copy at {}",
                path.display(),
                bad.display()
            );
            if let Err(e) = std::fs::copy(path, &bad) {
                log::error!("could not back up {}: {e}", path.display());
            }
        })
        .ok()
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::File::create(&tmp)
        .and_then(|mut f| {
            f.write_all(bytes)?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> bool {
    let result = serde_json::to_vec_pretty(value)
        .map_err(std::io::Error::other)
        .and_then(|b| write_atomic(path, &b));
    if let Err(e) = &result {
        log::error!("could not write {}: {e}", path.display());
    }
    result.is_ok()
}

pub(crate) fn fnv1a(key: &str) -> u64 {
    key.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

pub fn cache_path(kind: &str, key: &str) -> PathBuf {
    cache_dir().join(kind).join(format!("{:016x}", fnv1a(key)))
}

// Pruning goes by modified time, so a cache hit refreshes it (at most daily) to keep used files.
pub fn touch(path: &Path) {
    let now = std::time::SystemTime::now();
    let stale = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > TOUCH_AFTER));
    if stale && let Ok(f) = std::fs::File::options().write(true).open(path) {
        let _ = f.set_modified(now);
    }
}

pub fn prune_cache(max_bytes: u64) {
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = std::fs::read_dir(cache_dir())
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|d| std::fs::read_dir(d.path()).into_iter().flatten().flatten())
        .filter_map(|f| {
            let m = f.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), f.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= max_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

pub fn clear_cache() {
    for kind in ["img", "meta"] {
        let _ = std::fs::remove_dir_all(cache_dir().join(kind));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keys_are_stable_and_distinct() {
        assert_eq!(fnv1a("abc"), 0xe71fa2190541574b);
        assert_ne!(cache_path("meta", "a"), cache_path("meta", "b"));
    }

    #[test]
    fn touch_refreshes_only_old_files() {
        let p = std::env::temp_dir().join(format!("plaivo-touch-test-{}", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        let old = std::time::SystemTime::now() - 2 * TOUCH_AFTER;
        let f = std::fs::File::options().write(true).open(&p).unwrap();
        f.set_modified(old).unwrap();
        drop(f);
        touch(&p);
        let after = std::fs::metadata(&p).unwrap().modified().unwrap();
        assert!(after > old + TOUCH_AFTER);
        assert_eq!(std::fs::read(&p).unwrap(), b"x");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn atomic_write_round_trips() {
        let p = std::env::temp_dir()
            .join(format!("plaivo-store-test-{}", std::process::id()))
            .join("x.json");
        assert!(write_json(&p, &vec![1, 2, 3]));
        assert_eq!(read_json::<Vec<i32>>(&p), Some(vec![1, 2, 3]));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn corrupt_files_are_kept_aside() {
        let dir = std::env::temp_dir().join(format!("plaivo-store-bad-{}", std::process::id()));
        let p = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p), None);
        assert!(!dir.join("config.json.bad").exists());
        std::fs::write(&p, b"{\"token\": ").unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p), None);
        assert_eq!(
            std::fs::read(dir.join("config.json.bad")).unwrap(),
            b"{\"token\": "
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
