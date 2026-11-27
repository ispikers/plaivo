use crate::first;
use crate::plex::{Hub, Item, ItemKind, MarkerKind};
use crate::prefs::{LanguagePrefs, LanguageRule, SubtitleMode};
use crate::store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const RECENT_SEARCHES: usize = 8;
const RECENT_ROOMS: usize = 5;
pub const COUNTDOWN_MAX: u32 = 30;
pub const DENSITY_MAX: u32 = 5;
pub const DENSITY_LABELS: [&str; DENSITY_MAX as usize] =
    ["Smallest", "Small", "Medium", "Large", "Largest"];
pub const SKIP_DELAY_MAX: i32 = 15;
pub const AUTO_SKIP_NOTICE: f64 = 7.0;
pub const SUBTITLE_COLORS: [(&str, &str); 4] = [
    ("FFFFFF", "White"),
    ("FFFF00", "Yellow"),
    ("00FFFF", "Cyan"),
    ("00FF00", "Green"),
];

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
    pub stop_on_close: bool,
    pub skip_intro: SkipMode,
    pub skip_credits: SkipMode,
    pub skip_intro_delay: i32,
    pub skip_credits_delay: i32,
    pub seek_small: u32,
    pub seek_large: u32,
    pub shader: String,
    pub library_order: Vec<String>,
    pub hidden_libraries: Vec<String>,
    pub hidden_servers: Vec<String>,
    pub density: u32,
    pub theme: Theme,
    pub view_mode: ViewMode,
    pub grid_spacing: GridSpacing,
    pub episode_poster: EpisodePoster,
    pub episode_numbers: bool,
    pub season_posters: bool,
    pub hide_spoilers: bool,
    pub watched_indicators: bool,
    pub language_rules: Vec<LanguageRule>,
    pub playback_rules: Vec<PlaybackRule>,
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
    pub watch_relay: String,
    pub watch_rooms: Vec<String>,
    pub check_updates: bool,
    pub home_clock: HomeClock,
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
            stop_on_close: false,
            skip_intro: SkipMode::Button,
            skip_credits: SkipMode::Button,
            skip_intro_delay: 0,
            skip_credits_delay: 0,
            seek_small: 10,
            seek_large: 30,
            shader: String::new(),
            library_order: Vec::new(),
            hidden_libraries: Vec::new(),
            hidden_servers: Vec::new(),
            density: 3,
            theme: Theme::Dark,
            view_mode: ViewMode::Grid,
            grid_spacing: GridSpacing::Normal,
            episode_poster: EpisodePoster::Thumbnail,
            episode_numbers: true,
            season_posters: false,
            hide_spoilers: false,
            watched_indicators: true,
            language_rules: Vec::new(),
            playback_rules: Vec::new(),
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
            watch_relay: String::new(),
            watch_rooms: Vec::new(),
            check_updates: true,
            home_clock: HomeClock::Fullscreen,
        }
    }
}

pub fn seek_cancels_auto_skip(to: f64, start: f64, end: f64, auto_at: f64) -> bool {
    to >= start.min(auto_at) && to < end
}

pub fn countdown_left(seconds: u32, time: f64, anchor: &mut Option<f64>) -> f64 {
    let since = *anchor.insert(anchor.filter(|a| *a <= time).unwrap_or(time));
    seconds.min(COUNTDOWN_MAX) as f64 - (time - since)
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct PlaybackRule {
    pub libraries: Vec<String>,
    pub shader: Option<String>,
    pub quality: Option<u32>,
}

impl PlaybackRule {
    pub fn matches(&self, library: &str) -> bool {
        self.libraries.iter().any(|l| l == library)
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

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Light,
    #[default]
    Dark,
    Oled,
}

impl Theme {
    pub const ALL: [Theme; 4] = [Theme::System, Theme::Light, Theme::Dark, Theme::Oled];

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
            Theme::Oled => "OLED",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    Grid,
    List,
}

impl ViewMode {
    pub const ALL: [ViewMode; 2] = [ViewMode::Grid, ViewMode::List];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Grid => "Grid",
            ViewMode::List => "List",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum GridSpacing {
    Tight,
    #[default]
    Normal,
    Spacious,
}

impl GridSpacing {
    pub const ALL: [GridSpacing; 3] = [
        GridSpacing::Tight,
        GridSpacing::Normal,
        GridSpacing::Spacious,
    ];

    pub fn label(self) -> &'static str {
        match self {
            GridSpacing::Tight => "Tight",
            GridSpacing::Normal => "Normal",
            GridSpacing::Spacious => "Spacious",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum EpisodePoster {
    Series,
    Season,
    #[default]
    Thumbnail,
}

impl EpisodePoster {
    pub const ALL: [EpisodePoster; 3] = [
        EpisodePoster::Series,
        EpisodePoster::Season,
        EpisodePoster::Thumbnail,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EpisodePoster::Series => "Series poster",
            EpisodePoster::Season => "Season poster",
            EpisodePoster::Thumbnail => "Thumbnail",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum HomeClock {
    Never,
    #[default]
    Fullscreen,
    Always,
}

impl HomeClock {
    pub const ALL: [HomeClock; 3] = [HomeClock::Never, HomeClock::Fullscreen, HomeClock::Always];

    pub fn label(self) -> &'static str {
        match self {
            HomeClock::Never => "Never",
            HomeClock::Fullscreen => "Fullscreen",
            HomeClock::Always => "Always",
        }
    }

    pub fn shown(self, fullscreen: bool) -> bool {
        self == HomeClock::Always || (self == HomeClock::Fullscreen && fullscreen)
    }
}

pub fn rank_in(order: &[String], key: &str) -> usize {
    order.iter().position(|k| k == key).unwrap_or(usize::MAX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardPrefs {
    pub poster: EpisodePoster,
    pub numbers: bool,
    pub spoilers: bool,
}

impl CardPrefs {
    pub fn wide_hub(&self, h: &Hub) -> bool {
        h.is_wide() && (self.poster == EpisodePoster::Thumbnail || h.kind == ItemKind::Clip)
    }

    pub fn art<'a>(&self, it: &'a Item, wide: bool) -> &'a str {
        if it.kind == ItemKind::Episode && !wide && self.poster == EpisodePoster::Season {
            first(&[&it.parent_thumb, &it.grandparent_thumb, &it.thumb])
        } else {
            it.card_art(wide)
        }
    }

    pub fn text(&self, it: &Item) -> (String, String) {
        let (title, subtitle) = it.card_text();
        if it.kind == ItemKind::Episode && !self.numbers {
            (title, it.title.clone())
        } else {
            (title, subtitle)
        }
    }

    pub fn is_spoiler(&self, it: &Item) -> bool {
        self.spoilers && it.kind == ItemKind::Episode && !it.is_watched()
    }

    pub fn should_blur(&self, it: &Item, art: &str) -> bool {
        self.is_spoiler(it) && !art.is_empty() && art == it.thumb
    }
}

pub const BLUR_PARAM: &str = "&blur=20";

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

static WRITES: store::Writes = store::Writes::new();

fn path() -> PathBuf {
    store::config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        store::read_json(&path()).unwrap_or_default()
    }

    pub fn save(&self) -> bool {
        WRITES.now(&path(), self)
    }

    pub fn save_later(&self) -> impl FnOnce() + Send + 'static {
        WRITES.later(path(), self.clone())
    }

    pub fn flush(&self) {
        WRITES.flush(&path(), self);
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

    pub fn remember_room(&mut self, code: &str) {
        self.watch_rooms.retain(|c| c != code);
        self.watch_rooms.insert(0, code.to_owned());
        self.watch_rooms.truncate(RECENT_ROOMS);
    }

    pub fn forget_room(&mut self, code: &str) {
        self.watch_rooms.retain(|c| c != code);
    }

    fn skip_rule(&self, marker: MarkerKind) -> (SkipMode, i32) {
        match marker {
            MarkerKind::Intro => (self.skip_intro, self.skip_intro_delay),
            MarkerKind::Credits => (self.skip_credits, self.skip_credits_delay),
            MarkerKind::Commercial | MarkerKind::Other => (SkipMode::Button, 0),
        }
    }

    pub fn skip_mode(&self, marker: MarkerKind) -> SkipMode {
        self.skip_rule(marker).0
    }

    pub fn auto_skip_at(&self, marker: MarkerKind, start: f64) -> f64 {
        let delay = self.skip_rule(marker).1;
        (start + delay.clamp(-SKIP_DELAY_MAX, SKIP_DELAY_MAX) as f64).max(0.0)
    }

    pub fn up_next_in(
        &self,
        time: f64,
        duration: f64,
        credits: Option<f64>,
        anchor: &mut Option<f64>,
    ) -> Option<f64> {
        let countdown = self.autoplay_countdown.min(COUNTDOWN_MAX) as f64;
        let start = credits
            .filter(|c| *c > 0.0)
            .map_or(duration, |c| c.min(duration))
            .min(duration - countdown);
        if !self.autoplay_next || duration <= countdown || time < start {
            *anchor = None;
            return None;
        }
        Some(countdown_left(self.autoplay_countdown, time, anchor).min(duration - time))
    }

    pub fn languages(&self) -> LanguagePrefs {
        LanguagePrefs {
            audio: self.audio_language.clone(),
            subtitles: self.subtitle_language.clone(),
            mode: self.subtitle_mode,
        }
    }

    pub fn set_languages(&mut self, p: &LanguagePrefs) {
        self.audio_language = p.audio.clone();
        self.subtitle_language = p.subtitles.clone();
        self.subtitle_mode = p.mode;
    }

    pub fn card_prefs(&self) -> CardPrefs {
        CardPrefs {
            poster: self.episode_poster,
            numbers: self.episode_numbers,
            spoilers: self.hide_spoilers,
        }
    }

    pub fn card_pad(&self) -> f32 {
        match self.grid_spacing {
            GridSpacing::Tight => 0.,
            GridSpacing::Normal => 3.,
            GridSpacing::Spacious => 6.,
        }
    }

    pub fn density_label(&self) -> &'static str {
        DENSITY_LABELS[self.density.clamp(1, DENSITY_MAX) as usize - 1]
    }

    pub fn card_scale(&self) -> f32 {
        1.0 + (self.density.clamp(1, DENSITY_MAX) as f32 - 3.0) * 0.15
    }

    pub fn density_fraction(&self) -> f32 {
        (self.density.clamp(1, DENSITY_MAX) - 1) as f32 / (DENSITY_MAX - 1) as f32
    }

    pub fn list_poster_width(&self) -> f32 {
        70. + 50. * self.density_fraction()
    }

    pub fn list_summary_lines(&self) -> u32 {
        self.density.clamp(2, DENSITY_MAX)
    }

    pub fn library_rank(&self, key: &str) -> usize {
        rank_in(&self.library_order, key)
    }

    fn playback_rules_for(&self, library: &str) -> impl Iterator<Item = &PlaybackRule> {
        self.playback_rules
            .iter()
            .filter(move |r| r.matches(library))
    }

    pub fn quality_for(&self, library: &str) -> u32 {
        self.playback_rules_for(library)
            .find_map(|r| r.quality)
            .unwrap_or(self.quality)
    }

    pub fn shader_for(&self, library: &str) -> &str {
        self.playback_rules_for(library)
            .find_map(|r| r.shader.as_deref())
            .unwrap_or(&self.shader)
    }

    pub fn set_quality_for(&mut self, library: &str, kbps: u32) {
        match self
            .playback_rules
            .iter_mut()
            .find(|r| r.matches(library) && r.quality.is_some())
        {
            Some(r) => r.quality = Some(kbps),
            None => self.quality = kbps,
        }
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

    pub fn item_hidden(&self, server: &str, it: &Item) -> bool {
        !it.library_section_id.is_empty()
            && self.library_hidden(&library_key(server, &it.library_section_id))
    }

    pub fn server_hidden(&self, id: &str) -> bool {
        self.hidden_servers.iter().any(|k| k == id)
    }

    pub fn set_server_hidden(&mut self, server: &str, hidden: bool) {
        self.hidden_servers.retain(|k| k != server);
        self.hidden_libraries
            .retain(|k| library_server(k) != server);
        if hidden {
            self.hidden_servers.push(server.to_owned());
        }
    }

    pub fn sync_server_hidden<'a>(
        &mut self,
        server: &str,
        libraries: impl IntoIterator<Item = &'a str>,
    ) -> bool {
        let keys: Vec<&str> = libraries.into_iter().collect();
        // A server hidden before its libraries were ever listed keeps hiding them once they are.
        if self.server_hidden(server) && !keys.iter().any(|k| self.library_hidden(k)) {
            for k in keys {
                self.set_library_hidden(k, true);
            }
            return false;
        }
        let hidden = !keys.is_empty() && keys.iter().all(|k| self.library_hidden(k));
        if hidden == self.server_hidden(server) {
            return false;
        }
        self.hidden_servers.retain(|k| k != server);
        if hidden {
            self.hidden_servers.push(server.to_owned());
        }
        true
    }

    pub fn move_library(&mut self, arranged: &[String], key: &str, step: isize) {
        let Some(i) = arranged.iter().position(|k| k == key) else {
            return;
        };
        let server = library_server(key);
        let j = if step < 0 {
            arranged[..i]
                .iter()
                .rposition(|k| library_server(k) == server)
        } else {
            arranged[i + 1..]
                .iter()
                .position(|k| library_server(k) == server)
                .map(|j| i + 1 + j)
        };
        let Some(j) = j else { return };
        let mut order = arranged.to_vec();
        order.swap(i, j);
        order.extend(
            self.library_order
                .iter()
                .filter(|k| !arranged.contains(k))
                .cloned(),
        );
        self.library_order = order;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_clock_visibility() {
        let shown = |c: HomeClock| [c.shown(false), c.shown(true)];
        assert_eq!(shown(HomeClock::Never), [false, false]);
        assert_eq!(shown(HomeClock::Fullscreen), [false, true]);
        assert_eq!(shown(HomeClock::Always), [true, true]);
        let old: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(old.home_clock, HomeClock::Fullscreen);
    }

    #[test]
    fn card_prefs_shape_episode_cards() {
        let ep = Item {
            kind: ItemKind::Episode,
            title: "Pilot".into(),
            grandparent_title: "Show".into(),
            parent_index: Some(1),
            index: Some(2),
            thumb: "/ep".into(),
            parent_thumb: "/season".into(),
            grandparent_thumb: "/series".into(),
            ..Default::default()
        };
        let hub = Hub {
            kind: ItemKind::Episode,
            ..Default::default()
        };
        let thumbs = Settings::default().card_prefs();
        assert!(thumbs.wide_hub(&hub));
        assert_eq!(
            (thumbs.art(&ep, true), thumbs.art(&ep, false)),
            ("/ep", "/series")
        );
        assert_eq!(thumbs.text(&ep).1, "S1 · E2 · Pilot");
        let season = CardPrefs {
            poster: EpisodePoster::Season,
            numbers: false,
            spoilers: true,
        };
        assert!(!season.wide_hub(&hub));
        assert!(!season.wide_hub(&Hub {
            id: "home.continue".into(),
            kind: ItemKind::Mixed,
            ..Default::default()
        }));
        assert_eq!(season.art(&ep, false), "/season");
        assert_eq!(season.text(&ep), ("Show".to_owned(), "Pilot".to_owned()));
        assert!(season.should_blur(&ep, "/ep") && !season.should_blur(&ep, "/season"));
        assert!(!season.is_spoiler(&Item {
            view_count: Some(1),
            ..ep.clone()
        }));
        assert!(!thumbs.should_blur(&ep, "/ep"));
    }

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
        let s: Settings =
            serde_json::from_str(r#"{"sidebar_compact":true,"future_option":1}"#).unwrap();
        assert!(s.sidebar_compact && s.home_server.is_empty() && s.scale == Scale::Fit);
        let s: Settings = serde_json::from_str(r#"{"scale":"stretch"}"#).unwrap();
        assert_eq!(s.scale.next(), Scale::Fit);
    }

    #[test]
    fn new_fields_default_when_missing() {
        let s: Settings = serde_json::from_str(r#"{"quality":8000,"skip_intro":"auto"}"#).unwrap();
        assert!(s.autoplay_next);
        assert_eq!(
            (s.autoplay_countdown, s.seek_small, s.seek_large, s.density),
            (5, 10, 30, 3)
        );
        assert_eq!(
            (s.skip_intro, s.skip_credits),
            (SkipMode::Auto, SkipMode::Button)
        );
        assert_eq!(
            (
                s.subtitle_mode,
                s.subtitle_size,
                s.subtitle_color.as_str(),
                s.subtitle_position
            ),
            (SubtitleMode::ForeignAudio, 100, "FFFFFF", 100)
        );
        assert!(s.download_dir.is_empty() && !s.auto_remove_watched);
        assert!(s.watch_relay.is_empty() && s.watch_rooms.is_empty());
        let s: Settings =
            serde_json::from_str(r#"{"audio_language":"jpn","subtitle_mode":"always"}"#).unwrap();
        assert_eq!(
            s.languages(),
            LanguagePrefs {
                audio: "jpn".into(),
                subtitles: String::new(),
                mode: SubtitleMode::Always
            }
        );
    }

    #[test]
    fn recent_rooms_move_to_front_and_cap() {
        let mut s = Settings::default();
        for c in [
            "AAAAAA", "BBBBBB", "AAAAAA", "CCCCCC", "DDDDDD", "EEEEEE", "FFFFFF",
        ] {
            s.remember_room(c);
        }
        assert_eq!(
            s.watch_rooms,
            ["FFFFFF", "EEEEEE", "DDDDDD", "CCCCCC", "AAAAAA"]
        );
        s.forget_room("DDDDDD");
        s.forget_room("ZZZZZZ");
        assert_eq!(s.watch_rooms, ["FFFFFF", "EEEEEE", "CCCCCC", "AAAAAA"]);
    }

    #[test]
    fn skip_mode_follows_marker_kind() {
        let s = Settings {
            skip_intro: SkipMode::Auto,
            skip_credits: SkipMode::Off,
            ..Default::default()
        };
        assert_eq!(s.skip_mode(MarkerKind::Intro), SkipMode::Auto);
        assert_eq!(s.skip_mode(MarkerKind::Credits), SkipMode::Off);
        assert_eq!(s.skip_mode(MarkerKind::Commercial), SkipMode::Button);
        let s = Settings {
            skip_intro_delay: -3,
            skip_credits_delay: 99,
            ..s
        };
        assert_eq!(s.auto_skip_at(MarkerKind::Intro, 60.0), 57.0);
        assert_eq!(s.auto_skip_at(MarkerKind::Intro, 1.0), 0.0);
        assert_eq!(s.auto_skip_at(MarkerKind::Credits, 1200.0), 1215.0);
        assert_eq!(s.auto_skip_at(MarkerKind::Commercial, 60.0), 60.0);
    }

    #[test]
    fn seeking_into_a_marker_or_its_auto_point_cancels_auto_skip() {
        let intro = |to| seek_cancels_auto_skip(to, 60.0, 90.0, 57.0);
        assert!(!intro(52.0));
        assert!(intro(58.0));
        assert!(intro(75.0));
        assert!(!intro(90.0));
        let credits = |to| seek_cancels_auto_skip(to, 1200.0, 1300.0, 1215.0);
        assert!(!credits(1199.0));
        assert!(credits(1205.0));
    }

    #[test]
    fn countdown_restarts_when_seeking_back_past_its_start() {
        let mut anchor = None;
        let left: Vec<_> = [100.0, 103.0, 99.0, 101.0, 110.0]
            .iter()
            .map(|&t| countdown_left(5, t, &mut anchor))
            .collect();
        assert_eq!(left, [5.0, 2.0, 5.0, 3.0, -6.0]);
        assert_eq!(countdown_left(99, 0.0, &mut None), COUNTDOWN_MAX as f64);
    }

    #[test]
    fn up_next_counts_down_from_the_first_tick_in_the_window() {
        let s = Settings {
            autoplay_countdown: 10,
            ..Default::default()
        };
        let run = |s: &Settings, credits: Option<f64>, times: &[f64]| {
            let mut anchor = None;
            times
                .iter()
                .map(|&t| s.up_next_in(t, 1300.0, credits, &mut anchor))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            run(&s, None, &[1000.0, 1290.0, 1297.5]),
            [None, Some(10.0), Some(2.5)]
        );
        assert_eq!(run(&s, None, &[1299.0, 1300.0]), [Some(1.0), Some(0.0)]);
        assert_eq!(
            run(&s, Some(1200.0), &[1199.0, 1204.0, 1211.0, 1215.0]),
            [None, Some(10.0), Some(3.0), Some(-1.0)]
        );
        assert_eq!(
            run(&s, Some(1250.0), &[1251.0, 1100.0, 1252.0]),
            [Some(10.0), None, Some(10.0)]
        );
        assert_eq!(run(&s, Some(1296.0), &[1289.0, 1290.0]), [None, Some(10.0)]);
        assert_eq!(run(&s, Some(0.0), &[1295.0]), [Some(5.0)]);
        let zero = Settings {
            autoplay_countdown: 0,
            ..s.clone()
        };
        assert_eq!(
            run(&zero, Some(1200.0), &[1199.0, 1200.5]),
            [None, Some(0.0)]
        );
        assert_eq!(run(&zero, None, &[1299.0]), [None]);
        assert_eq!(
            run(
                &Settings {
                    autoplay_next: false,
                    ..s.clone()
                },
                None,
                &[1299.0]
            ),
            [None]
        );
        assert_eq!(
            run(
                &Settings {
                    autoplay_countdown: 99,
                    ..s.clone()
                },
                None,
                &[1270.0]
            ),
            [Some(30.0)]
        );
        assert_eq!(s.up_next_in(5.0, 0.0, None, &mut None), None);
    }

    #[test]
    fn density_scales_cards_around_the_default() {
        let at = |density| {
            Settings {
                density,
                ..Default::default()
            }
            .card_scale()
        };
        assert_eq!(at(3), 1.0);
        assert!(at(1) < at(2) && at(2) < at(3) && at(3) < at(4) && at(4) < at(5));
        assert_eq!((at(0), at(9)), (at(1), at(5)));
    }

    #[test]
    fn list_rows_grow_with_density() {
        let at = |density| {
            let s = Settings {
                density,
                ..Default::default()
            };
            (s.list_poster_width(), s.list_summary_lines())
        };
        assert_eq!(at(0), (70., 2));
        assert_eq!(at(2), (82.5, 2));
        assert_eq!(at(3), (95., 3));
        assert_eq!(at(DENSITY_MAX), (120., DENSITY_MAX));
        assert_eq!(at(9), at(DENSITY_MAX));
    }

    #[test]
    fn density_labels_clamp() {
        let label = |density| {
            Settings {
                density,
                ..Default::default()
            }
            .density_label()
        };
        assert_eq!(
            (label(0), label(3), label(9)),
            ("Smallest", "Medium", "Largest")
        );
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
        assert_eq!(
            arranged(&s, all.clone()),
            keys(&["a/1", "a/3", "b/1", "a/2"])
        );
        let now = arranged(&s, all.clone());
        s.move_library(&now, "a/1", -1);
        s.move_library(&now, "b/1", 1);
        assert_eq!(arranged(&s, all.clone()), now);
        s.move_library(&now, "a/1", 1);
        assert_eq!(
            arranged(&s, keys(&["a/1", "a/2", "b/1", "a/3", "c/9"])),
            keys(&["a/3", "a/1", "b/1", "a/2", "c/9"])
        );

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

        s.set_library_hidden("a/1", true);
        assert!(!s.sync_server_hidden("a", ["a/1", "a/2"]) && !s.server_hidden("a"));
        s.set_library_hidden("a/2", true);
        assert!(s.sync_server_hidden("a", ["a/1", "a/2"]) && s.server_hidden("a"));
        assert!(!s.sync_server_hidden("a", ["a/1", "a/2"]));
        assert!(s.sync_server_hidden("a", ["a/1", "a/2", "a/3"]) && !s.server_hidden("a"));
        assert!(!s.sync_server_hidden("z", []) && !s.server_hidden("z"));

        s.set_server_hidden("off", true);
        assert!(!s.sync_server_hidden("off", []) && s.server_hidden("off"));
        assert!(!s.sync_server_hidden("off", ["off/1", "off/2"]) && s.server_hidden("off"));
        assert!(s.library_hidden("off/1") && s.library_hidden("off/2"));
        s.set_server_hidden("off", false);
        assert!(!s.server_hidden("off") && !s.library_hidden("off/1") && s.library_hidden("a/1"));

        s.set_library_hidden("part/1", true);
        s.set_server_hidden("part", true);
        assert!(!s.sync_server_hidden("part", ["part/1", "part/2"]) && s.server_hidden("part"));
        assert!(s.library_hidden("part/1") && s.library_hidden("part/2"));
    }

    #[test]
    fn playback_rules_override_each_setting_independently() {
        let mut s = Settings {
            quality: 8000,
            shader: "fsr".into(),
            playback_rules: vec![
                PlaybackRule {
                    libraries: vec!["a/1".into()],
                    shader: Some("anime4k".into()),
                    quality: None,
                },
                PlaybackRule {
                    libraries: vec!["a/1".into(), "b/2".into()],
                    shader: Some(String::new()),
                    quality: Some(2000),
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            (s.shader_for("a/1"), s.quality_for("a/1")),
            ("anime4k", 2000)
        );
        assert_eq!((s.shader_for("b/2"), s.quality_for("b/2")), ("", 2000));
        assert_eq!((s.shader_for("c/3"), s.quality_for("c/3")), ("fsr", 8000));
        s.set_quality_for("a/1", 0);
        assert_eq!(
            (s.quality_for("b/2"), s.playback_rules[0].quality),
            (0, None)
        );
        s.set_quality_for("c/3", 4000);
        assert_eq!((s.quality, s.quality_for("a/1")), (4000, 0));
    }
}
