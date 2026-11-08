use crate::prefs::{LanguagePrefs, SubtitleMode};
use crate::store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const RECENT_SEARCHES: usize = 8;
pub const COUNTDOWN_MAX: u32 = 30;
pub const DENSITY_MAX: u32 = 5;
pub const SUBTITLE_COLORS: [(&str, &str); 4] = [("FFFFFF", "White"), ("FFFF00", "Yellow"), ("00FFFF", "Cyan"), ("00FF00", "Green")];

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub sidebar_compact: bool,
    pub home_server: String,
    pub recent_searches: Vec<String>,
    pub quality: u32,
    pub scale: Scale,
    pub audio_device: String,
    pub autoplay_next: bool,
    pub autoplay_countdown: u32,
    pub skip_intro: SkipMode,
    pub skip_credits: SkipMode,
    pub seek_small: u32,
    pub seek_large: u32,
    pub shader: String,
    pub library_order: Vec<String>,
    pub hidden_libraries: Vec<String>,
    pub density: u32,
    pub debug_logging: bool,
    pub audio_language: String,
    pub subtitle_language: String,
    pub subtitle_mode: SubtitleMode,
    pub subtitle_size: u32,
    pub subtitle_color: String,
    pub subtitle_outline: u32,
    pub subtitle_background: u32,
    pub subtitle_position: u32,
    pub download_dir: String,
    pub auto_remove_watched: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sidebar_compact: false,
            home_server: String::new(),
            recent_searches: Vec::new(),
            quality: 0,
            scale: Scale::Fit,
            audio_device: String::new(),
            autoplay_next: true,
            autoplay_countdown: 5,
            skip_intro: SkipMode::Button,
            skip_credits: SkipMode::Button,
            seek_small: 10,
            seek_large: 30,
            shader: String::new(),
            library_order: Vec::new(),
            hidden_libraries: Vec::new(),
            density: 3,
            debug_logging: false,
            audio_language: String::new(),
            subtitle_language: String::new(),
            subtitle_mode: SubtitleMode::ForeignAudio,
            subtitle_size: 100,
            subtitle_color: SUBTITLE_COLORS[0].0.into(),
            subtitle_outline: 3,
            subtitle_background: 0,
            subtitle_position: 100,
            download_dir: String::new(),
            auto_remove_watched: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum SkipMode {
    Off,
    #[default]
    Button,
    Auto,
}

impl SkipMode {
    pub const ALL: [SkipMode; 3] = [SkipMode::Off, SkipMode::Button, SkipMode::Auto];

    pub fn label(self) -> &'static str {
        match self {
            SkipMode::Off => "Off",
            SkipMode::Button => "Button",
            SkipMode::Auto => "Auto",
        }
    }
}

pub fn library_key(server: &str, section: &str) -> String {
    format!("{server}/{section}")
}

fn library_server(key: &str) -> &str {
    key.rsplit_once('/').map_or(key, |(server, _)| server)
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

    pub fn skip_mode(&self, marker: &str) -> SkipMode {
        match marker {
            "intro" => self.skip_intro,
            "credits" => self.skip_credits,
            _ => SkipMode::Button,
        }
    }

    pub fn up_next_in(&self, time: f64, duration: f64, credits: Option<f64>, anchor: &mut Option<f64>) -> Option<f64> {
        let countdown = self.autoplay_countdown.min(COUNTDOWN_MAX) as f64;
        let start = credits.filter(|c| *c > 0.0).map_or(duration, |c| c.min(duration)).min(duration - countdown);
        if !self.autoplay_next || duration <= countdown || time < start {
            *anchor = None;
            return None;
        }
        let since = *anchor.get_or_insert(time);
        Some(countdown - (time - since))
    }

    pub fn languages(&self) -> LanguagePrefs {
        LanguagePrefs { audio: self.audio_language.clone(), subtitles: self.subtitle_language.clone(), mode: self.subtitle_mode }
    }

    pub fn set_languages(&mut self, p: &LanguagePrefs) {
        self.audio_language = p.audio.clone();
        self.subtitle_language = p.subtitles.clone();
        self.subtitle_mode = p.mode;
    }

    pub fn card_scale(&self) -> f32 {
        1.0 + (self.density.clamp(1, DENSITY_MAX) as f32 - 3.0) * 0.15
    }

    pub fn library_rank(&self, key: &str) -> usize {
        self.library_order.iter().position(|k| k == key).unwrap_or(usize::MAX)
    }

    pub fn library_hidden(&self, key: &str) -> bool {
        self.hidden_libraries.iter().any(|k| k == key)
    }

    pub fn set_library_hidden(&mut self, key: &str, hidden: bool) {
        self.hidden_libraries.retain(|k| k != key);
        if hidden {
            self.hidden_libraries.push(key.to_owned());
        }
    }

    pub fn move_library(&mut self, arranged: &[String], key: &str, step: isize) {
        let Some(i) = arranged.iter().position(|k| k == key) else { return };
        let server = library_server(key);
        let j = if step < 0 {
            arranged[..i].iter().rposition(|k| library_server(k) == server)
        } else {
            arranged[i + 1..].iter().position(|k| library_server(k) == server).map(|j| i + 1 + j)
        };
        let Some(j) = j else { return };
        let mut order = arranged.to_vec();
        order.swap(i, j);
        order.extend(self.library_order.iter().filter(|k| !arranged.contains(k)).cloned());
        self.library_order = order;
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

    #[test]
    fn new_fields_default_when_missing() {
        let s: Settings = serde_json::from_str(r#"{"quality":8000,"skip_intro":"auto"}"#).unwrap();
        assert!(s.autoplay_next);
        assert_eq!((s.autoplay_countdown, s.seek_small, s.seek_large, s.density), (5, 10, 30, 3));
        assert_eq!((s.skip_intro, s.skip_credits), (SkipMode::Auto, SkipMode::Button));
        assert_eq!((s.subtitle_mode, s.subtitle_size, s.subtitle_color.as_str(), s.subtitle_position), (SubtitleMode::ForeignAudio, 100, "FFFFFF", 100));
        assert!(s.download_dir.is_empty() && !s.auto_remove_watched);
        let s: Settings = serde_json::from_str(r#"{"audio_language":"jpn","subtitle_mode":"always"}"#).unwrap();
        assert_eq!(s.languages(), LanguagePrefs { audio: "jpn".into(), subtitles: String::new(), mode: SubtitleMode::Always });
    }

    #[test]
    fn skip_mode_follows_marker_kind() {
        let s = Settings { skip_intro: SkipMode::Auto, skip_credits: SkipMode::Off, ..Default::default() };
        assert_eq!(s.skip_mode("intro"), SkipMode::Auto);
        assert_eq!(s.skip_mode("credits"), SkipMode::Off);
        assert_eq!(s.skip_mode("commercial"), SkipMode::Button);
    }

    #[test]
    fn up_next_counts_down_from_the_first_tick_in_the_window() {
        let s = Settings { autoplay_countdown: 10, ..Default::default() };
        let run = |s: &Settings, credits: Option<f64>, times: &[f64]| {
            let mut anchor = None;
            times.iter().map(|&t| s.up_next_in(t, 1300.0, credits, &mut anchor)).collect::<Vec<_>>()
        };
        assert_eq!(run(&s, None, &[1000.0, 1290.0, 1297.5]), [None, Some(10.0), Some(2.5)]);
        assert_eq!(run(&s, None, &[1299.0, 1300.0]), [Some(10.0), Some(9.0)]);
        assert_eq!(run(&s, Some(1200.0), &[1199.0, 1204.0, 1211.0, 1215.0]), [None, Some(10.0), Some(3.0), Some(-1.0)]);
        assert_eq!(run(&s, Some(1250.0), &[1251.0, 1100.0, 1252.0]), [Some(10.0), None, Some(10.0)]);
        assert_eq!(run(&s, Some(1296.0), &[1289.0, 1290.0]), [None, Some(10.0)]);
        assert_eq!(run(&s, Some(0.0), &[1295.0]), [Some(10.0)]);
        let zero = Settings { autoplay_countdown: 0, ..s.clone() };
        assert_eq!(run(&zero, Some(1200.0), &[1199.0, 1200.5]), [None, Some(0.0)]);
        assert_eq!(run(&zero, None, &[1299.0]), [None]);
        assert_eq!(run(&Settings { autoplay_next: false, ..s.clone() }, None, &[1299.0]), [None]);
        assert_eq!(run(&Settings { autoplay_countdown: 99, ..s.clone() }, None, &[1270.0]), [Some(30.0)]);
        assert_eq!(s.up_next_in(5.0, 0.0, None, &mut None), None);
    }

    #[test]
    fn density_scales_cards_around_the_default() {
        let at = |density| Settings { density, ..Default::default() }.card_scale();
        assert_eq!(at(3), 1.0);
        assert!(at(1) < at(2) && at(2) < at(3) && at(3) < at(4) && at(4) < at(5));
        assert_eq!((at(0), at(9)), (at(1), at(5)));
    }

    #[test]
    fn libraries_reorder_within_their_server_and_hide() {
        let keys = |v: &[&str]| v.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        let arranged = |s: &Settings, mut v: Vec<String>| {
            v.sort_by_key(|k| s.library_rank(k));
            v
        };
        let mut s = Settings::default();
        let all = keys(&["a/1", "a/2", "b/1", "a/3"]);
        assert_eq!(arranged(&s, all.clone()), all);
        s.move_library(&all, "a/3", -1);
        assert_eq!(arranged(&s, all.clone()), keys(&["a/1", "a/3", "b/1", "a/2"]));
        let now = arranged(&s, all.clone());
        s.move_library(&now, "a/1", -1);
        s.move_library(&now, "b/1", 1);
        assert_eq!(arranged(&s, all.clone()), now);
        s.move_library(&now, "a/1", 1);
        assert_eq!(arranged(&s, keys(&["a/1", "a/2", "b/1", "a/3", "c/9"])), keys(&["a/3", "a/1", "b/1", "a/2", "c/9"]));

        s.library_order.push("gone/1".into());
        let now = arranged(&s, all.clone());
        s.move_library(&now, "a/2", -1);
        assert_eq!(s.library_order.last().map(String::as_str), Some("gone/1"));

        s.set_library_hidden("b/1", true);
        s.set_library_hidden("b/1", true);
        assert!(s.library_hidden("b/1") && s.hidden_libraries.len() == 1);
        s.set_library_hidden("b/1", false);
        assert!(!s.library_hidden("b/1"));
        assert_eq!(library_key("srv", "4"), "srv/4");
    }
}
