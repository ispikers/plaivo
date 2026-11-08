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

    pub fn save(&self) {
        store::write_json(&path(), self);
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
}
