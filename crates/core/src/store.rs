use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

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
    let base = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
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
    base.unwrap_or_else(|| config_dir().join("cache")).join("cache")
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("{}-{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) {
    let result = serde_json::to_vec_pretty(value).map_err(std::io::Error::other).and_then(|b| write_atomic(path, &b));
    if let Err(e) = result {
        log::error!("could not write {}: {e}", path.display());
    }
}

fn fnv1a(key: &str) -> u64 {
    key.bytes().fold(0xcbf29ce484222325, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

pub fn cache_path(kind: &str, key: &str) -> PathBuf {
    cache_dir().join(kind).join(format!("{:016x}", fnv1a(key)))
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
    let _ = std::fs::remove_dir_all(cache_dir());
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
    fn atomic_write_round_trips() {
        let p = std::env::temp_dir().join(format!("plaivo-store-test-{}", std::process::id())).join("x.json");
        write_json(&p, &vec![1, 2, 3]);
        assert_eq!(read_json::<Vec<i32>>(&p), Some(vec![1, 2, 3]));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
