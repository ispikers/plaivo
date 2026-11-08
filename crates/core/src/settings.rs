use crate::store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const RECENT_SEARCHES: usize = 8;

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub sidebar_compact: bool,
    pub home_server: String,
    pub recent_searches: Vec<String>,
    pub quality: u32,
    pub scale: Scale,
    pub audio_device: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
    #[default]
    Fit,
    Fill,
    Stretch,
}

impl Scale {
    pub fn next(self) -> Self {
        match self {
            Scale::Fit => Scale::Fill,
            Scale::Fill => Scale::Stretch,
            Scale::Stretch => Scale::Fit,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scale::Fit => "Letterbox",
            Scale::Fill => "Fill screen",
            Scale::Stretch => "Stretch",
        }
    }

    pub fn mpv(self) -> [(&'static str, &'static str); 2] {
        match self {
            Scale::Fit => [("keepaspect", "yes"), ("panscan", "0")],
            Scale::Fill => [("keepaspect", "yes"), ("panscan", "1")],
            Scale::Stretch => [("keepaspect", "no"), ("panscan", "0")],
        }
    }
}

fn path() -> PathBuf {
    store::config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        store::read_json(&path()).unwrap_or_default()
    }

    pub fn save(&self) -> bool {
        store::write_json(&path(), self)
    }

    pub fn remember_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() {
            return;
        }
        self.recent_searches.retain(|s| !s.eq_ignore_ascii_case(q));
        self.recent_searches.insert(0, q.to_owned());
        self.recent_searches.truncate(RECENT_SEARCHES);
    }

    pub fn migrate_android(&mut self, sidebar_compact: Option<bool>, home_server: Option<String>, quality: Option<i32>, scale: Option<&str>) {
        if let Some(v) = sidebar_compact {
            self.sidebar_compact = v;
        }
        if let Some(v) = home_server.filter(|s| !s.is_empty()) {
            self.home_server = v;
        }
        if let Some(v) = quality.and_then(|q| u32::try_from(q).ok()) {
            self.quality = v;
        }
        if let Some(v) = scale.and_then(|s| serde_json::from_value(serde_json::Value::String(s.to_ascii_lowercase())).ok()) {
            self.scale = v;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_searches_dedupe_and_cap() {
        let mut s = Settings::default();
        for q in ["a", "b", "A", " c ", ""] {
            s.remember_search(q);
        }
        assert_eq!(s.recent_searches, ["c", "A", "b"]);
        for i in 0..20 {
            s.remember_search(&i.to_string());
        }
        assert_eq!(s.recent_searches.len(), RECENT_SEARCHES);
    }

    #[test]
    fn unknown_and_missing_fields_fall_back_to_defaults() {
        let s: Settings = serde_json::from_str(r#"{"sidebar_compact":true,"future_option":1}"#).unwrap();
        assert!(s.sidebar_compact && s.home_server.is_empty() && s.scale == Scale::Fit);
        let s: Settings = serde_json::from_str(r#"{"scale":"stretch"}"#).unwrap();
        assert_eq!(s.scale.next(), Scale::Fit);
    }

    #[test]
    fn android_prefs_migrate_once_into_settings() {
        let mut s = Settings { recent_searches: vec!["x".into()], ..Default::default() };
        s.migrate_android(Some(true), Some("srv".into()), Some(8000), Some("Fill"));
        assert!(s.sidebar_compact);
        assert_eq!((s.home_server.as_str(), s.quality, s.scale), ("srv", 8000, Scale::Fill));
        assert_eq!(s.recent_searches, ["x"]);

        let mut s = Settings { home_server: "kept".into(), quality: 4000, ..Default::default() };
        s.migrate_android(None, Some(String::new()), Some(-1), Some("Zoom"));
        assert_eq!((s.home_server.as_str(), s.quality, s.scale, s.sidebar_compact), ("kept", 4000, Scale::Fit, false));
    }
}
