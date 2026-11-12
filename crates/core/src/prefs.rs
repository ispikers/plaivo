use crate::jellyfin;
use crate::plex::{Client, Item, Media, Picks, R, Server, Stream, drain, encode, err};
use crate::settings::library_key;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const PROFILE: &str = "https://clients.plex.tv/api/v2/user/profile";
const PLEX_KEY: &str = "plex";
const RETRY_AFTER: Duration = Duration::from_secs(300);

pub const LANGUAGES: &[(&str, &[&str])] = &[
    ("English", &["eng", "en"]),
    ("Japanese", &["jpn", "ja"]),
    ("Spanish", &["spa", "es"]),
    ("French", &["fre", "fra", "fr"]),
    ("German", &["ger", "deu", "de"]),
    ("Italian", &["ita", "it"]),
    ("Portuguese", &["por", "pt"]),
    ("Russian", &["rus", "ru"]),
    ("Chinese", &["chi", "zho", "zh"]),
    ("Korean", &["kor", "ko"]),
    ("Arabic", &["ara", "ar"]),
    ("Hindi", &["hin", "hi"]),
    ("Dutch", &["dut", "nld", "nl"]),
    ("Swedish", &["swe", "sv"]),
    ("Norwegian", &["nor", "no", "nob", "nb", "nno", "nn"]),
    ("Danish", &["dan", "da"]),
    ("Finnish", &["fin", "fi"]),
    ("Icelandic", &["ice", "isl", "is"]),
    ("Polish", &["pol", "pl"]),
    ("Czech", &["cze", "ces", "cs"]),
    ("Slovak", &["slo", "slk", "sk"]),
    ("Hungarian", &["hun", "hu"]),
    ("Romanian", &["rum", "ron", "ro"]),
    ("Greek", &["gre", "ell", "el"]),
    ("Turkish", &["tur", "tr"]),
    ("Hebrew", &["heb", "he", "iw"]),
    ("Persian", &["per", "fas", "fa"]),
    ("Ukrainian", &["ukr", "uk"]),
    ("Bulgarian", &["bul", "bg"]),
    ("Croatian", &["hrv", "hr"]),
    ("Serbian", &["srp", "sr"]),
    ("Slovenian", &["slv", "sl"]),
    ("Estonian", &["est", "et"]),
    ("Latvian", &["lav", "lv"]),
    ("Lithuanian", &["lit", "lt"]),
    ("Catalan", &["cat", "ca"]),
    ("Basque", &["baq", "eus", "eu"]),
    ("Galician", &["glg", "gl"]),
    ("Thai", &["tha", "th"]),
    ("Vietnamese", &["vie", "vi"]),
    ("Indonesian", &["ind", "id", "in"]),
    ("Malay", &["may", "msa", "ms"]),
    ("Tagalog", &["tgl", "tl", "fil"]),
    ("Bengali", &["ben", "bn"]),
    ("Tamil", &["tam", "ta"]),
    ("Telugu", &["tel", "te"]),
];

#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleMode {
    Off,
    #[default]
    ForeignAudio,
    Always,
}

impl SubtitleMode {
    pub const ALL: [SubtitleMode; 3] = [
        SubtitleMode::Off,
        SubtitleMode::ForeignAudio,
        SubtitleMode::Always,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SubtitleMode::Off => "Off",
            SubtitleMode::ForeignAudio => "Foreign audio only",
            SubtitleMode::Always => "Always",
        }
    }
}

pub fn two_letter(code: &str) -> Option<&'static str> {
    let code = code.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|(_, codes)| codes.contains(&code.as_str()))
        .and_then(|(_, codes)| codes.iter().copied().find(|c| c.len() == 2))
}

pub fn language_label(code: &str) -> Option<&'static str> {
    let code = code.to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|(_, codes)| codes.contains(&code.as_str()))
        .map(|(name, _)| *name)
}

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct LanguagePrefs {
    pub audio: String,
    pub subtitles: String,
    pub mode: SubtitleMode,
}

#[derive(Clone, Default, Debug)]
pub struct TrackInfo {
    pub language: String,
    pub forced: bool,
    pub is_default: bool,
}

fn entry(code: &str) -> Option<&'static [&'static str]> {
    LANGUAGES
        .iter()
        .map(|(_, codes)| *codes)
        .find(|codes| codes.contains(&code))
}

pub fn canonical(code: &str) -> String {
    let primary = code
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if primary == "und" {
        return String::new();
    }
    entry(&primary).map_or(primary, |codes| codes[0].to_owned())
}

pub fn language_name(code: &str) -> String {
    let code = canonical(code);
    match LANGUAGES.iter().find(|(_, codes)| codes[0] == code) {
        Some((name, _)) => (*name).to_owned(),
        None if code.is_empty() => "Any".into(),
        None => code.to_uppercase(),
    }
}

pub fn language_codes(code: &str) -> String {
    let code = canonical(code);
    entry(&code).map_or(code, |codes| codes.join(","))
}

fn iso1(code: &str) -> String {
    let code = canonical(code);
    entry(&code)
        .and_then(|codes| codes.iter().find(|c| c.len() == 2))
        .map_or(code, |c| (*c).to_owned())
}

fn same(a: &str, b: &str) -> bool {
    let a = canonical(a);
    !a.is_empty() && a == canonical(b)
}

fn best(tracks: &[TrackInfo], f: impl Fn(&TrackInfo) -> bool) -> Option<usize> {
    tracks
        .iter()
        .position(|t| f(t) && t.is_default)
        .or_else(|| tracks.iter().position(f))
}

pub fn pick_audio(tracks: &[TrackInfo], p: &LanguagePrefs) -> Option<usize> {
    if tracks.is_empty() {
        return None;
    }
    best(tracks, |t| same(&t.language, &p.audio))
        .or_else(|| tracks.iter().position(|t| t.is_default))
        .or(Some(0))
}

pub fn pick_subtitle(tracks: &[TrackInfo], audio: &str, p: &LanguagePrefs) -> Option<usize> {
    let native = if p.subtitles.is_empty() {
        &p.audio
    } else {
        &p.subtitles
    };
    let full = match p.mode {
        SubtitleMode::Off => false,
        SubtitleMode::Always => true,
        SubtitleMode::ForeignAudio => {
            !canonical(audio).is_empty() && !canonical(native).is_empty() && !same(audio, native)
        }
    };
    full.then(|| {
        best(tracks, |t| {
            !t.forced && (p.subtitles.is_empty() || same(&t.language, &p.subtitles))
        })
    })
    .flatten()
    .or_else(|| {
        best(tracks, |t| {
            t.forced && (canonical(&t.language).is_empty() || same(&t.language, audio))
        })
    })
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Debug)]
#[serde(default)]
pub struct LanguageRule {
    pub libraries: Vec<String>,
    pub audio: Vec<String>,
    pub subtitles: Vec<String>,
    pub subtitle_mode: SubtitleMode,
}

impl LanguageRule {
    pub fn matches(&self, library: &str) -> bool {
        self.libraries.iter().any(|l| l == library)
    }

    pub fn resolve(&self, audio: &[TrackInfo], subtitles: &[TrackInfo]) -> LanguagePrefs {
        let first_present = |order: &[String], tracks: &[TrackInfo]| {
            order
                .iter()
                .find(|l| tracks.iter().any(|t| same(&t.language, l)))
                .or(order.first())
                .cloned()
                .unwrap_or_default()
        };
        LanguagePrefs {
            audio: first_present(&self.audio, audio),
            subtitles: first_present(&self.subtitles, subtitles),
            mode: self.subtitle_mode,
        }
    }
}

pub fn rule_for<'a>(rules: &'a [LanguageRule], library: &str) -> Option<&'a LanguageRule> {
    rules.iter().find(|r| r.matches(library))
}

pub fn language_rule(server: &str, it: &Item, rules: &[LanguageRule]) -> Option<LanguageRule> {
    rule_for(rules, &library_key(server, &it.library_section_id)).cloned()
}

pub fn pick_with(
    audio: &[TrackInfo],
    subtitles: &[TrackInfo],
    p: &LanguagePrefs,
    rule: Option<&LanguageRule>,
) -> (Option<usize>, Option<usize>) {
    match rule {
        Some(r) => pick(audio, subtitles, &r.resolve(audio, subtitles)),
        None => pick(audio, subtitles, p),
    }
}

pub fn pick_source(media: &Media, p: &LanguagePrefs, rule: Option<&LanguageRule>) -> Picks {
    let info = |s: &Stream| TrackInfo {
        language: s.language_code.clone(),
        forced: s.forced,
        is_default: s.selected,
    };
    let audio: Vec<TrackInfo> = media.audio_streams().map(info).collect();
    let subtitles: Vec<TrackInfo> = media.subtitle_streams().map(info).collect();
    let (audio, subtitle) = pick_with(&audio, &subtitles, p, rule);
    Picks { audio, subtitle }
}

pub fn pick(
    audio: &[TrackInfo],
    subtitles: &[TrackInfo],
    p: &LanguagePrefs,
) -> (Option<usize>, Option<usize>) {
    let a = pick_audio(audio, p);
    let language = a.map_or("", |i| audio[i].language.as_str());
    (a, pick_subtitle(subtitles, language, p))
}

fn profile_language(v: &Value) -> String {
    let first = match v {
        Value::String(s) => s.split(',').next(),
        Value::Array(a) => a.first().and_then(Value::as_str),
        _ => None,
    };
    first.map(canonical).unwrap_or_default()
}

fn from_profile(json: &Value) -> LanguagePrefs {
    let p = json
        .get("profile")
        .filter(|p| p.is_object())
        .unwrap_or(json);
    let auto = &p["autoSelectSubtitle"];
    let mode = auto
        .as_i64()
        .or_else(|| auto.as_str()?.trim().parse().ok())
        .or_else(|| auto.as_bool().map(i64::from));
    LanguagePrefs {
        audio: profile_language(&p["defaultAudioLanguage"]),
        subtitles: profile_language(&p["defaultSubtitleLanguage"]),
        mode: match mode {
            Some(0) => SubtitleMode::Off,
            Some(2) => SubtitleMode::Always,
            _ => SubtitleMode::ForeignAudio,
        },
    }
}

fn profile_query(p: &LanguagePrefs) -> String {
    let mode = match p.mode {
        SubtitleMode::Off => 0,
        SubtitleMode::ForeignAudio => 1,
        SubtitleMode::Always => 2,
    };
    format!(
        "defaultAudioLanguage={}&defaultSubtitleLanguage={}&autoSelectSubtitle={mode}",
        encode(&iso1(&p.audio)),
        encode(&iso1(&p.subtitles))
    )
}

impl From<jellyfin::LanguagePrefs> for LanguagePrefs {
    fn from(j: jellyfin::LanguagePrefs) -> Self {
        let unset = LanguagePrefs {
            mode: SubtitleMode::Off,
            ..Default::default()
        };
        jellyfin_or(&j, &unset)
    }
}

fn jellyfin_or(j: &jellyfin::LanguagePrefs, unset: &LanguagePrefs) -> LanguagePrefs {
    let or = |code: &str, fallback: &str| match canonical(code) {
        c if c.is_empty() => fallback.to_owned(),
        c => c,
    };
    LanguagePrefs {
        audio: or(&j.audio, &unset.audio),
        subtitles: or(&j.subtitles, &unset.subtitles),
        mode: match j.subtitle_mode {
            jellyfin::SubtitleMode::Always => SubtitleMode::Always,
            jellyfin::SubtitleMode::Smart => SubtitleMode::ForeignAudio,
            jellyfin::SubtitleMode::Default => unset.mode,
            jellyfin::SubtitleMode::OnlyForced | jellyfin::SubtitleMode::None => SubtitleMode::Off,
        },
    }
}

impl From<&LanguagePrefs> for jellyfin::LanguagePrefs {
    fn from(p: &LanguagePrefs) -> Self {
        let subtitle_mode = match p.mode {
            SubtitleMode::Off => jellyfin::SubtitleMode::OnlyForced,
            SubtitleMode::ForeignAudio => jellyfin::SubtitleMode::Smart,
            SubtitleMode::Always => jellyfin::SubtitleMode::Always,
        };
        jellyfin::LanguagePrefs {
            audio: p.audio.clone(),
            subtitles: p.subtitles.clone(),
            subtitle_mode,
        }
    }
}

fn profile_json(c: &Client, token: &str) -> R<Value> {
    c.headers(c.agent.get(PROFILE), Some(token))
        .call()
        .map_err(err)?
        .body_mut()
        .read_json()
        .map_err(err)
}

pub fn plex_account(c: &Client, token: &str) -> R<LanguagePrefs> {
    Ok(from_profile(&profile_json(c, token)?))
}

fn set_plex_account(c: &Client, token: &str, p: &LanguagePrefs) -> R<()> {
    c.headers(
        c.agent.put(format!("{PROFILE}?{}", profile_query(p))),
        Some(token),
    )
    .send_empty()
    .map(drain)
    .map_err(err)?;
    Ok(())
}

enum Cached {
    Prefs(LanguagePrefs),
    Jellyfin(jellyfin::LanguagePrefs),
    RetryAt(Instant),
}

static CACHE: Mutex<BTreeMap<String, Cached>> = Mutex::new(BTreeMap::new());
static WRITES: Mutex<u64> = Mutex::new(0);
static GENERATION: AtomicU64 = AtomicU64::new(0);

fn remember(key: &str, p: &LanguagePrefs) {
    CACHE
        .lock()
        .unwrap()
        .insert(key.to_owned(), Cached::Prefs(p.clone()));
}

pub fn forget() {
    CACHE.lock().unwrap().clear();
}

pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed) + 1
}

pub fn for_server(
    c: &Client,
    srv: &Server,
    plex_token: Option<&str>,
    local: &LanguagePrefs,
) -> LanguagePrefs {
    let key = if srv.is_jellyfin() {
        srv.id.as_str()
    } else {
        PLEX_KEY
    };
    match CACHE.lock().unwrap().get(key) {
        Some(Cached::Prefs(p)) => return p.clone(),
        Some(Cached::Jellyfin(j)) => return jellyfin_or(j, local),
        Some(Cached::RetryAt(until)) if Instant::now() < *until => return local.clone(),
        _ => {}
    }
    let fetched = match (srv.is_jellyfin(), plex_token) {
        (true, _) => jellyfin::language_prefs(srv, c).map(Cached::Jellyfin),
        (false, Some(token)) => plex_account(c, token).map(Cached::Prefs),
        (false, None) => return local.clone(),
    };
    let cached = fetched.unwrap_or_else(|e| {
        log::warn!("language preferences from {}: {e}", srv.name);
        Cached::RetryAt(Instant::now() + RETRY_AFTER)
    });
    let p = match &cached {
        Cached::Prefs(p) => p.clone(),
        Cached::Jellyfin(j) => jellyfin_or(j, local),
        Cached::RetryAt(_) => local.clone(),
    };
    CACHE.lock().unwrap().insert(key.to_owned(), cached);
    p
}

pub fn account(
    c: &Client,
    plex_token: Option<&str>,
    jellyfin: &[Server],
) -> R<Option<LanguagePrefs>> {
    let _writes = WRITES.lock().unwrap();
    Ok(Some(match (plex_token, jellyfin.first()) {
        (Some(token), _) => plex_account(c, token)?,
        (None, Some(srv)) => jellyfin::language_prefs(srv, c)?.into(),
        (None, None) => return Ok(None),
    }))
}

pub fn share_with_new_server(
    c: &Client,
    plex_token: Option<&str>,
    jellyfin: &[Server],
    added: &Server,
) -> R<()> {
    let generation = next_generation();
    let others: Vec<Server> = jellyfin
        .iter()
        .filter(|s| s.id != added.id)
        .cloned()
        .collect();
    match account(c, plex_token, &others)? {
        Some(p) => save(c, None, std::slice::from_ref(added), &p, generation),
        None => Ok(()),
    }
}

pub fn save(
    c: &Client,
    plex_token: Option<&str>,
    jellyfin: &[Server],
    p: &LanguagePrefs,
    generation: u64,
) -> R<()> {
    let mut written = WRITES.lock().unwrap();
    if generation < *written {
        return Ok(());
    }
    *written = generation;
    let mut failed = Vec::new();
    if let Some(token) = plex_token {
        match set_plex_account(c, token, p) {
            Ok(()) => remember(PLEX_KEY, p),
            Err(e) => failed.push(format!("Plex: {e}")),
        }
    }
    for srv in jellyfin {
        match jellyfin::set_language_prefs(srv, c, &p.into()) {
            Ok(()) => remember(&srv.id, p),
            Err(e) => failed.push(format!("{}: {e}", srv.name)),
        }
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_jellyfin_prefs_inherit_the_app_ones() {
        let app = LanguagePrefs {
            audio: "eng".into(),
            subtitles: "eng".into(),
            mode: SubtitleMode::Always,
        };
        let unset = jellyfin::LanguagePrefs::default();
        assert_eq!(jellyfin_or(&unset, &app), app);
        let own = jellyfin::LanguagePrefs {
            audio: "ita".into(),
            subtitles: String::new(),
            subtitle_mode: jellyfin::SubtitleMode::OnlyForced,
        };
        assert_eq!(
            jellyfin_or(&own, &app),
            LanguagePrefs {
                audio: canonical("ita"),
                subtitles: "eng".into(),
                mode: SubtitleMode::Off,
            }
        );
        assert_eq!(LanguagePrefs::from(unset).mode, SubtitleMode::Off);
    }

    #[test]
    fn picks_source_tracks_from_prefs() {
        let stream = |stream_type, lang: &str, selected, forced| Stream {
            stream_type,
            language_code: lang.into(),
            selected,
            forced,
            ..Default::default()
        };
        let media = Media {
            part: vec![crate::plex::Part {
                stream: vec![
                    stream(2, "ita", true, false),
                    stream(2, "eng", false, false),
                    stream(3, "ita", false, true),
                    stream(3, "eng", false, false),
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        let english = prefs("en", "en", SubtitleMode::Always);
        let picks = |audio, subtitle| Picks { audio, subtitle };
        assert_eq!(pick_source(&media, &english, None), picks(Some(1), Some(1)));
        let off = prefs("", "", SubtitleMode::Off);
        assert_eq!(pick_source(&media, &off, None), picks(Some(0), Some(0)));
        assert_eq!(
            pick_source(&Media::default(), &english, None),
            Picks::default()
        );
    }

    #[test]
    fn language_rules_match_and_resolve() {
        let rule = LanguageRule {
            libraries: vec!["s/2".into(), "s/3".into()],
            audio: vec!["jpn".into(), "eng".into()],
            subtitles: vec!["eng".into()],
            subtitle_mode: SubtitleMode::Always,
        };
        let rules = [
            LanguageRule {
                libraries: vec!["s/9".into()],
                ..Default::default()
            },
            rule.clone(),
        ];
        assert_eq!(rule_for(&rules, "s/3"), Some(&rule));
        assert_eq!(rule_for(&rules, "s/1"), None);
        let ep = Item {
            kind: "episode".into(),
            library_section_id: "2".into(),
            ..Default::default()
        };
        assert_eq!(language_rule("s", &ep, &rules), Some(rule.clone()));
        let t = |l: &str| TrackInfo {
            language: l.into(),
            ..Default::default()
        };
        assert_eq!(
            rule.resolve(&[t("en"), t("ja")], &[t("en")]),
            LanguagePrefs {
                audio: "jpn".into(),
                subtitles: "eng".into(),
                mode: SubtitleMode::Always
            }
        );
        assert_eq!(
            rule.resolve(&[t("en")], &[]).audio,
            "eng",
            "falls back to the next language the file has"
        );
        assert_eq!(
            pick_with(
                &[t("en"), t("ja")],
                &[t("fr"), t("en")],
                &LanguagePrefs::default(),
                Some(&rule)
            ),
            (Some(1), Some(1))
        );
    }

    #[test]
    fn languages_map_to_two_letter_codes() {
        assert_eq!(
            (
                two_letter("eng"),
                two_letter("FRA"),
                two_letter("ja"),
                two_letter("xx")
            ),
            (Some("en"), Some("fr"), Some("ja"), None)
        );
        assert_eq!(
            (language_label("ger"), language_label("")),
            (Some("German"), None)
        );
    }

    fn t(language: &str, forced: bool, is_default: bool) -> TrackInfo {
        TrackInfo {
            language: language.into(),
            forced,
            is_default,
        }
    }

    fn prefs(audio: &str, subtitles: &str, mode: SubtitleMode) -> LanguagePrefs {
        LanguagePrefs {
            audio: audio.into(),
            subtitles: subtitles.into(),
            mode,
        }
    }

    #[test]
    fn codes_normalise_across_iso_forms() {
        assert_eq!(
            (
                canonical("en"),
                canonical("ENG"),
                canonical("en-US"),
                canonical("deu"),
                canonical("pt_BR")
            ),
            (
                "eng".into(),
                "eng".into(),
                "eng".into(),
                "ger".into(),
                "por".into()
            )
        );
        assert_eq!(
            (canonical("und"), canonical(""), canonical("xyz")),
            (String::new(), String::new(), "xyz".into())
        );
        assert_eq!(
            (language_name("de"), language_name(""), language_name("xyz")),
            ("German".into(), "Any".into(), "XYZ".into())
        );
        assert_eq!(
            (language_codes("fr"), language_codes("")),
            ("fre,fra,fr".into(), String::new())
        );
        assert_eq!((iso1("ger"), iso1("xyz")), ("de".into(), "xyz".into()));
        assert!(LANGUAGES.iter().all(|(_, codes)| codes[0].len() == 3));
    }

    #[test]
    fn audio_prefers_language_then_default_then_first() {
        let tracks = [
            t("eng", false, true),
            t("jpn", false, false),
            t("jpn", false, true),
        ];
        assert_eq!(
            pick_audio(&tracks, &prefs("ja", "", SubtitleMode::Off)),
            Some(2)
        );
        assert_eq!(
            pick_audio(&tracks, &prefs("fre", "", SubtitleMode::Off)),
            Some(0)
        );
        assert_eq!(
            pick_audio(
                &[t("", false, false), t("", false, false)],
                &prefs("", "", SubtitleMode::Off)
            ),
            Some(0)
        );
        assert_eq!(pick_audio(&[], &prefs("eng", "", SubtitleMode::Off)), None);
    }

    #[test]
    fn subtitle_modes() {
        let audio = [t("jpn", false, true), t("eng", false, false)];
        let subs = [
            t("eng", true, false),
            t("eng", false, false),
            t("eng", false, true),
            t("jpn", true, false),
        ];
        let foreign = prefs("eng", "eng", SubtitleMode::ForeignAudio);
        assert_eq!(pick(&audio, &subs, &foreign), (Some(1), Some(0)));
        assert_eq!(pick(&audio[..1], &subs, &foreign), (Some(0), Some(2)));
        assert_eq!(
            pick(
                &audio,
                &subs,
                &prefs("jpn", "eng", SubtitleMode::ForeignAudio)
            ),
            (Some(0), Some(2))
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("jpn", "", SubtitleMode::ForeignAudio)),
            (Some(0), Some(3))
        );
        assert_eq!(
            pick(
                &audio[..1],
                &subs,
                &prefs("eng", "", SubtitleMode::ForeignAudio)
            ),
            (Some(0), Some(2))
        );
        assert_eq!(
            pick(
                &audio,
                &subs,
                &prefs("eng", "jpn", SubtitleMode::ForeignAudio)
            ),
            (Some(1), Some(0))
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("jpn", "eng", SubtitleMode::Always)),
            (Some(0), Some(2))
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("jpn", "eng", SubtitleMode::Off)),
            (Some(0), Some(3))
        );
        assert_eq!(
            pick(&audio, &subs[..3], &prefs("jpn", "eng", SubtitleMode::Off)),
            (Some(0), None)
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("", "eng", SubtitleMode::ForeignAudio)),
            (Some(0), Some(2))
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("", "", SubtitleMode::ForeignAudio)),
            (Some(0), Some(3))
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("jpn", "", SubtitleMode::Always)),
            (Some(0), Some(2))
        );
        assert_eq!(
            pick(&audio, &subs, &prefs("jpn", "fre", SubtitleMode::Always)),
            (Some(0), Some(3))
        );
        assert_eq!(
            pick(&[t("", false, false)], &subs, &foreign),
            (Some(0), None)
        );
        assert_eq!(
            pick(&[t("", false, false)], &[t("", true, false)], &foreign),
            (Some(0), Some(0))
        );
        assert_eq!(
            pick(&audio, &[], &prefs("jpn", "eng", SubtitleMode::Always)),
            (Some(0), None)
        );
    }

    #[test]
    fn plex_profile_round_trip() {
        let json: Value = serde_json::from_str(r#"{"profile":{"defaultAudioLanguage":"ja","defaultSubtitleLanguage":"en,sv","autoSelectSubtitle":2}}"#).unwrap();
        assert_eq!(
            from_profile(&json),
            prefs("jpn", "eng", SubtitleMode::Always)
        );
        let bare: Value = serde_json::from_str(r#"{"defaultAudioLanguage":["de"],"defaultSubtitleLanguage":null,"autoSelectSubtitle":"0"}"#).unwrap();
        assert_eq!(from_profile(&bare), prefs("ger", "", SubtitleMode::Off));
        assert_eq!(from_profile(&Value::Null), LanguagePrefs::default());
        assert_eq!(
            profile_query(&prefs("jpn", "", SubtitleMode::ForeignAudio)),
            "defaultAudioLanguage=ja&defaultSubtitleLanguage=&autoSelectSubtitle=1"
        );
    }

    #[test]
    fn jellyfin_modes_map_both_ways() {
        let j = jellyfin::LanguagePrefs {
            audio: "jpn".into(),
            subtitles: "eng".into(),
            subtitle_mode: jellyfin::SubtitleMode::Smart,
        };
        let p = LanguagePrefs::from(j);
        assert_eq!(p, prefs("jpn", "eng", SubtitleMode::ForeignAudio));
        for (mode, back) in [
            (SubtitleMode::Off, jellyfin::SubtitleMode::OnlyForced),
            (SubtitleMode::Always, jellyfin::SubtitleMode::Always),
        ] {
            assert_eq!(
                jellyfin::LanguagePrefs::from(&prefs("", "", mode)).subtitle_mode,
                back
            );
        }
        let none = jellyfin::LanguagePrefs {
            subtitle_mode: jellyfin::SubtitleMode::None,
            ..Default::default()
        };
        assert_eq!(LanguagePrefs::from(none).mode, SubtitleMode::Off);
    }

    #[test]
    fn fallbacks_and_cache() {
        let c = Client::new("c".into());
        let local = prefs("eng", "", SubtitleMode::Always);
        let plex = Server {
            name: "p".into(),
            id: "p".into(),
            ..Default::default()
        };
        assert_eq!(for_server(&c, &plex, None, &local), local);
        let down = Server {
            name: "j".into(),
            id: "j-down".into(),
            uri: "http://127.0.0.1:1".into(),
            backend: crate::plex::Backend::Jellyfin,
            ..Default::default()
        };
        assert_eq!(for_server(&c, &down, None, &local), local);
        assert!(
            matches!(CACHE.lock().unwrap().get("j-down"), Some(Cached::RetryAt(until)) if *until > Instant::now())
        );
        let other = prefs("jpn", "", SubtitleMode::Off);
        assert_eq!(for_server(&c, &down, None, &other), other);
        CACHE.lock().unwrap().insert(
            "j-down".into(),
            Cached::Jellyfin(jellyfin::LanguagePrefs::default()),
        );
        assert_eq!(for_server(&c, &down, None, &local), local);
        assert_eq!(for_server(&c, &down, None, &other), other);
        forget();
        assert!(CACHE.lock().unwrap().get("j-down").is_none());
    }

    #[test]
    fn stale_writes_are_skipped() {
        let c = Client::new("c".into());
        let (older, newer) = (next_generation(), next_generation());
        save(&c, None, &[], &LanguagePrefs::default(), newer).unwrap();
        save(&c, None, &[], &LanguagePrefs::default(), older).unwrap();
        assert!(*WRITES.lock().unwrap() >= newer);
    }

    #[test]
    #[ignore]
    fn live_plex_account_prefs() {
        let account: crate::Account =
            crate::store::read_json(&crate::store::config_dir().join("config.json"))
                .expect("config.json");
        let c = Client::new(account.client_id);
        let token = account.token.expect("signed in");
        let json = profile_json(&c, &token).unwrap();
        let p = json.get("profile").unwrap_or(&json);
        for field in [
            "defaultAudioLanguage",
            "defaultSubtitleLanguage",
            "autoSelectSubtitle",
        ] {
            println!("{field} = {}", p[field]);
        }
        println!("parsed: {:?}", from_profile(&json));
    }
}
