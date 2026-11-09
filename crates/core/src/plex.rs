use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use crate::{first, jellyfin, join, store};

pub type R<T> = Result<T, String>;

pub(crate) const PRODUCT: &str = "Plaivo";
const PLEX_TV: &str = "https://plex.tv/api/v2";
pub const DISCOVER_ID: &str = "discover";
const DISCOVER_URI: &str = "https://discover.provider.plex.tv";

#[derive(Clone)]
pub struct Client {
    pub(crate) agent: ureq::Agent,
    pub client_id: Arc<str>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Plex,
    Jellyfin,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub name: String,
    pub id: String,
    pub uri: String,
    pub token: String,
    #[serde(default)]
    pub owned: bool,
    #[serde(default)]
    pub backend: Backend,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub user: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Source {
    Cache,
    Network,
    Live,
}

pub struct Ctx {
    pub client: Client,
    source: Source,
    changed: AtomicBool,
}

impl Ctx {
    pub fn new(client: Client, source: Source) -> Self {
        Self { client, source, changed: AtomicBool::new(false) }
    }

    pub fn changed(&self) -> bool {
        self.changed.load(Ordering::Relaxed)
    }
}

#[derive(Deserialize)]
pub struct Pin {
    pub id: u64,
    pub code: String,
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
    #[serde(rename = "expiresIn")]
    pub expires_in: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Resource {
    pub name: String,
    pub client_identifier: String,
    pub provides: String,
    pub access_token: Option<String>,
    pub owned: bool,
    pub connections: Vec<Connection>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Connection {
    pub uri: String,
    pub local: bool,
    pub relay: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct Envelope<T: Default> {
    media_container: T,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Container {
    #[serde(rename = "Metadata")]
    pub metadata: Vec<Item>,
    #[serde(rename = "Directory")]
    pub directory: Vec<Directory>,
    #[serde(rename = "Hub")]
    pub hub: Vec<Hub>,
    #[serde(rename = "SearchResult")]
    pub search_result: Vec<SearchHit>,
    #[serde(rename = "SearchResults")]
    pub search_results: Vec<SearchGroup>,
    #[serde(rename = "totalSize")]
    pub total_size: Option<usize>,
    pub size: usize,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SearchHit {
    pub score: Option<f64>,
    #[serde(rename = "Metadata")]
    pub metadata: Option<Item>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SearchGroup {
    #[serde(rename = "SearchResult")]
    pub search_result: Vec<SearchHit>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Directory {
    pub key: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Hub {
    pub title: String,
    pub key: String,
    #[serde(rename = "hubIdentifier")]
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "Metadata")]
    pub metadata: Vec<Item>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Item {
    pub rating_key: String,
    pub guid: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub title: String,
    pub parent_title: String,
    pub grandparent_title: String,
    pub parent_rating_key: String,
    pub grandparent_rating_key: String,
    pub parent_index: Option<i64>,
    pub index: Option<i64>,
    pub year: Option<i64>,
    pub summary: String,
    pub tagline: String,
    pub content_rating: String,
    pub audience_rating: Option<f64>,
    pub thumb: String,
    pub art: String,
    pub parent_thumb: String,
    pub grandparent_thumb: String,
    pub grandparent_art: String,
    pub duration: Option<u64>,
    pub view_offset: Option<u64>,
    pub view_count: Option<u64>,
    pub leaf_count: Option<u64>,
    pub viewed_leaf_count: Option<u64>,
    pub child_count: Option<u64>,
    pub studio: String,
    pub originally_available_at: String,
    pub added_at: Option<i64>,
    pub user_rating: Option<f64>,
    #[serde(rename = "librarySectionID", deserialize_with = "flex_string")]
    pub library_section_id: String,
    pub primary_extra_key: String,
    pub subtype: String,
    #[serde(rename = "Role")]
    pub role: Vec<Person>,
    #[serde(rename = "Director")]
    pub director: Vec<Tag>,
    #[serde(rename = "Writer")]
    pub writer: Vec<Tag>,
    #[serde(rename = "Rating")]
    pub ratings: Vec<Rating>,
    #[serde(rename = "Image")]
    pub images: Vec<Image>,
    #[serde(rename = "Extras")]
    pub extras: Option<Extras>,
    #[serde(rename = "OnDeck")]
    pub on_deck: Option<Box<OnDeck>>,
    #[serde(rename = "Genre")]
    pub genre: Vec<Tag>,
    #[serde(rename = "Guid")]
    pub guids: Vec<Guid>,
    #[serde(rename = "Media")]
    pub media: Vec<Media>,
    #[serde(rename = "Marker")]
    pub marker: Vec<Marker>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Marker {
    #[serde(rename = "type")]
    pub kind: String,
    pub start_time_offset: u64,
    pub end_time_offset: u64,
}

impl Marker {
    pub fn skip_label(&self) -> Option<&'static str> {
        match self.kind.as_str() {
            "intro" => Some("Skip Intro"),
            "credits" => Some("Skip Credits"),
            "commercial" => Some("Skip Ad"),
            _ => None,
        }
    }
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct OnDeck {
    #[serde(rename = "Metadata")]
    pub metadata: Option<Item>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Tag {
    pub tag: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Person {
    pub tag: String,
    pub role: String,
    pub thumb: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Rating {
    pub image: String,
    pub value: f64,
}

#[derive(Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SubtitleResult {
    pub key: String,
    pub codec: String,
    pub language_code: String,
    pub language: String,
    pub provider_title: String,
    pub display_title: String,
    pub title: String,
    #[serde(deserialize_with = "flex_bool")]
    pub hearing_impaired: bool,
    #[serde(deserialize_with = "flex_bool")]
    pub perfect_match: bool,
    #[serde(deserialize_with = "flex_bool")]
    pub forced: bool,
    #[serde(deserialize_with = "flex_f64")]
    pub score: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SubtitleResults {
    #[serde(rename = "Stream")]
    stream: Vec<SubtitleResult>,
}

fn flex_bool<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.),
        serde_json::Value::String(s) => matches!(s.as_str(), "1" | "true"),
        _ => false,
    })
}

fn flex_string<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::String(s) => s,
        serde_json::Value::Number(n) => n.to_string(),
        _ => String::new(),
    })
}

fn flex_f64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RatingSource {
    Imdb,
    Critics,
    Audience,
    Tmdb,
    Stars,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Image {
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Extras {
    #[serde(rename = "Metadata")]
    pub metadata: Vec<Item>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Guid {
    pub id: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Media {
    pub bitrate: Option<u64>,
    pub height: Option<u64>,
    pub container: String,
    pub video_codec: String,
    pub video_resolution: String,
    pub video_dynamic_range: String,
    pub audio_codec: String,
    pub audio_channels: Option<u32>,
    #[serde(skip)]
    pub source_id: String,
    #[serde(rename = "Part")]
    pub part: Vec<Part>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
pub struct Part {
    pub key: String,
    pub file: String,
    #[serde(rename = "Stream")]
    pub stream: Vec<Stream>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Stream {
    pub stream_type: u8,
    pub key: String,
    pub codec: String,
    pub language_code: String,
    pub display_title: String,
    pub forced: bool,
    pub selected: bool,
}

impl Hub {
    pub fn is_continue(&self) -> bool {
        let id = self.id.to_ascii_lowercase();
        id.contains("continue") || id.contains("ondeck") || id.contains("inprogress")
    }

    pub fn wide(&self) -> bool {
        matches!(self.kind.as_str(), "episode" | "clip" | "mixed") || self.is_continue()
    }
}

impl Item {
    pub fn as_show(self) -> Item {
        let (key, title, thumb, art) = match self.kind.as_str() {
            "episode" => (self.grandparent_rating_key, self.grandparent_title, self.grandparent_thumb, self.grandparent_art),
            "season" => (self.parent_rating_key, self.parent_title, self.parent_thumb, self.art),
            _ => return self,
        };
        Item { rating_key: key, kind: "show".into(), title, thumb, art, ..Default::default() }
    }

    pub fn watched(&self) -> bool {
        match self.leaf_count {
            Some(n) => n > 0 && self.viewed_leaf_count == Some(n),
            None => self.view_count.unwrap_or(0) > 0,
        }
    }

    pub fn progress(&self) -> f32 {
        match (self.view_offset, self.duration) {
            (Some(o), Some(d)) if d > 0 => (o as f32 / d as f32).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    pub fn unwatched(&self) -> u64 {
        self.leaf_count.unwrap_or(0).saturating_sub(self.viewed_leaf_count.unwrap_or(0))
    }

    pub fn card_text(&self) -> (String, String) {
        match self.kind.as_str() {
            "episode" => (self.grandparent_title.clone(), join(&[self.episode_label(), self.title.clone()])),
            "season" => (self.title.clone(), self.parent_title.clone()),
            _ => (self.title.clone(), self.year.map(|y| y.to_string()).unwrap_or_default()),
        }
    }

    pub fn card_art(&self, wide: bool) -> &str {
        match (wide, self.kind.as_str()) {
            (true, "episode") => first(&[&self.thumb, &self.art]),
            (true, _) => first(&[&self.art, &self.grandparent_art, &self.thumb]),
            (false, "episode") => first(&[&self.grandparent_thumb, &self.parent_thumb, &self.thumb]),
            (false, _) => first(&[&self.thumb, &self.parent_thumb]),
        }
    }

    pub fn logo(&self) -> &str {
        self.images.iter().find(|i| i.kind == "clearLogo").map_or("", |i| i.url.as_str())
    }

    pub fn extras(&self) -> &[Item] {
        self.extras.as_ref().map_or(&[], |e| &e.metadata)
    }

    pub fn trailer(&self) -> Option<&Item> {
        let primary = self.primary_extra_key.rsplit('/').next().filter(|k| !k.is_empty());
        self.extras().iter().find(|x| Some(x.rating_key.as_str()) == primary).or_else(|| self.extras().iter().find(|x| x.subtype == "trailer"))
    }

    pub fn rating_labels(&self) -> Vec<(RatingSource, String)> {
        let labels: Vec<(RatingSource, String)> = self
            .ratings
            .iter()
            .filter_map(|r| {
                let (source, kind) = r.image.split_once("://")?;
                Some(match (source, kind) {
                    ("imdb", _) => (RatingSource::Imdb, format!("{:.1}", r.value)),
                    ("rottentomatoes", "image.rating.ripe" | "image.rating.rotten") => (RatingSource::Critics, format!("{:.0}%", r.value * 10.)),
                    ("rottentomatoes", _) => (RatingSource::Audience, format!("{:.0}%", r.value * 10.)),
                    ("themoviedb", _) => (RatingSource::Tmdb, format!("{:.0}%", r.value * 10.)),
                    _ => return None,
                })
            })
            .collect();
        if labels.is_empty() { self.audience_rating.map(|r| (RatingSource::Stars, format!("{r:.1}"))).into_iter().collect() } else { labels }
    }

    pub fn audio_label(&self) -> String {
        let Some(m) = self.media.first() else { return String::new() };
        let channels = match m.audio_channels {
            Some(1) => "1.0",
            Some(2) => "2.0",
            Some(6) => "5.1",
            Some(8) => "7.1",
            _ => "",
        };
        [m.audio_codec.to_ascii_uppercase().as_str(), channels].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
    }

    pub fn media_line(&self, version: usize) -> String {
        let Some(m) = self.media.get(version).or(self.media.first()) else { return String::new() };
        let streams = m.part.first().map_or(&[][..], |p| &p.stream);
        let pick = |t: u8| streams.iter().filter(|s| s.stream_type == t).min_by_key(|s| !s.selected).map(|s| s.display_title.clone()).unwrap_or_default();
        join(&[fmt_resolution(&m.video_resolution), m.video_codec.to_ascii_uppercase(), m.video_dynamic_range.clone(), pick(2), pick(3)])
    }

    pub fn plex_guid(&self) -> Option<&str> {
        self.guid.strip_prefix("plex://").and_then(|g| g.rsplit('/').next()).filter(|id| !id.is_empty())
    }

    pub fn episode_label(&self) -> String {
        match (self.parent_index, self.index) {
            (Some(s), Some(e)) => format!("S{s} · E{e}"),
            (None, Some(e)) => format!("E{e}"),
            _ => String::new(),
        }
    }
}

pub fn resolve_playable(srv: &Server, cx: &Ctx, key: &str) -> R<Item> {
    let mut key = key.to_owned();
    for _ in 0..4 {
        let it = srv.metadata(cx, &key)?;
        match it.kind.as_str() {
            "show" if it.on_deck.as_ref().and_then(|d| d.metadata.as_ref()).is_some() => {
                key = it.on_deck.unwrap().metadata.unwrap().rating_key;
            }
            "show" | "season" => {
                let children = srv.children(cx, &key)?;
                let next = children.iter().find(|i| !i.watched()).or(children.first()).ok_or("Nothing to play")?;
                key = next.rating_key.clone();
            }
            _ => return Ok(it),
        }
    }
    Err("Nothing to play".into())
}

static DOWN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub const HUB_MAX: usize = 1000;

pub const SORTS: &[(&str, &str)] = &[
    ("titleSort", "Title"),
    ("addedAt:desc", "Recently added"),
    ("originallyAvailableAt:desc", "Release date"),
    ("audienceRating:desc", "Rating"),
];

pub fn initial_sort(key: &str) -> String {
    let param = key.split(['?', '&']).find_map(|p| p.strip_prefix("sort="));
    if let Some((id, _)) = SORTS.iter().find(|(id, _)| Some(*id) == param) {
        return (*id).to_owned();
    }
    if key.starts_with("latest:") || key.to_ascii_lowercase().contains("recentlyadded") { "addedAt:desc".to_owned() } else { String::new() }
}

pub fn sort_items(items: &mut [&Item], sort: &str) {
    use std::cmp::Reverse;
    match sort {
        "titleSort" => items.sort_by_cached_key(|it| first(&[&it.grandparent_title, &it.title]).to_lowercase()),
        "addedAt:desc" => items.sort_by_key(|it| Reverse(it.added_at)),
        "originallyAvailableAt:desc" => items.sort_by_cached_key(|it| Reverse(if it.originally_available_at.is_empty() { it.year.map(|y| y.to_string()).unwrap_or_default() } else { it.originally_available_at.clone() })),
        "audienceRating:desc" => items.sort_by(|a, b| b.audience_rating.partial_cmp(&a.audience_rating).unwrap_or(std::cmp::Ordering::Equal)),
        _ => {}
    }
}

pub fn hub_sorts(items: &[Item], key: &str) -> Vec<(&'static str, &'static str)> {
    let added = items.iter().any(|it| it.added_at.is_some()) || initial_sort(key) == "addedAt:desc";
    std::iter::once(("", "Default")).chain(SORTS.iter().copied().filter(|(id, _)| *id != "addedAt:desc" || added)).collect()
}

pub fn is_down(server_id: &str) -> bool {
    DOWN.lock().unwrap_or_else(|e| e.into_inner()).contains(server_id)
}

pub fn set_down(server_id: &str, down: bool) {
    let mut set = DOWN.lock().unwrap_or_else(|e| e.into_inner());
    let changed = if down { set.insert(server_id.to_owned()) } else { set.remove(server_id) };
    if changed {
        log::info!("server {server_id} is {}", if down { "offline" } else { "back online" });
    }
}

fn unreachable(e: &ureq::Error) -> bool {
    matches!(e, ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::HostNotFound | ureq::Error::Io(_))
}

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn escape_uri(path: &str) -> String {
    path.bytes().map(|b| if b.is_ascii_graphic() && !b"<>\"{}|\\^`".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn watchlisted_in(v: &serde_json::Value) -> bool {
    let state = &v["MediaContainer"]["UserState"];
    let state = state.as_array().and_then(|a| a.first()).unwrap_or(state);
    !state["watchlistedAt"].is_null()
}

pub fn fmt_resolution(r: &str) -> String {
    match r.to_ascii_lowercase().as_str() {
        "" => String::new(),
        "4k" => "4K".into(),
        r if r.chars().all(|c| c.is_ascii_digit()) => format!("{r}p"),
        r if r.ends_with('p') => r.into(),
        r => r.to_uppercase(),
    }
}

pub fn fmt_duration(ms: u64) -> String {
    let m = ms / 60_000;
    match m / 60 {
        0 => format!("{m}m"),
        h => format!("{h}h {}m", m % 60),
    }
}

impl Client {
    pub fn new(client_id: String) -> Self {
        let agent = crate::http().timeout_connect(Some(Duration::from_secs(5))).timeout_global(Some(Duration::from_secs(30))).http_status_as_error(true).build().into();
        Self { agent, client_id: client_id.into() }
    }

    fn get(&self, url: &str, token: Option<&str>) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        self.headers(self.agent.get(url), token)
    }

    pub(crate) fn headers<B>(&self, r: ureq::RequestBuilder<B>, token: Option<&str>) -> ureq::RequestBuilder<B> {
        let mut r = r
            .header("Accept", "application/json")
            .header("X-Plex-Product", PRODUCT)
            .header("X-Plex-Version", env!("CARGO_PKG_VERSION"))
            .header("X-Plex-Client-Identifier", &*self.client_id)
            .header("X-Plex-Platform", std::env::consts::OS)
            .header("X-Plex-Device-Name", PRODUCT);
        if let Some(t) = token {
            r = r.header("X-Plex-Token", t);
        }
        r
    }

    pub fn create_pin(&self, strong: bool) -> R<Pin> {
        self.agent
            .post(format!("{PLEX_TV}/pins?strong={strong}"))
            .header("Accept", "application/json")
            .header("X-Plex-Product", PRODUCT)
            .header("X-Plex-Client-Identifier", &*self.client_id)
            .send_empty()
            .map_err(err)?
            .body_mut()
            .read_json()
            .map_err(err)
    }

    pub fn fetch(&self, url: &str) -> R<Vec<u8>> {
        self.agent.get(url).call().map_err(err)?.body_mut().with_config().limit(32 << 20).read_to_vec().map_err(err)
    }

    pub fn discover(&self, account_token: &str) -> Server {
        Server { name: "Plex".into(), id: DISCOVER_ID.into(), uri: DISCOVER_URI.into(), token: account_token.into(), ..Default::default() }
    }

    pub fn discover_search(&self, q: &str, account_token: &str) -> R<Vec<Item>> {
        let url = format!("{DISCOVER_URI}/library/search?query={}&limit=30&searchTypes=movies,tv&searchProviders=discover&includeMetadata=1", encode(q));
        let env: Envelope<Container> = self.get(&url, Some(account_token)).call().map_err(err)?.body_mut().with_config().limit(16 << 20).read_json().map_err(err)?;
        Ok(env.media_container.search_results.into_iter().flat_map(|g| g.search_result).filter_map(|h| h.metadata).collect())
    }

    pub fn watchlisted(&self, account_token: &str, guid: &str) -> R<bool> {
        let url = format!("{DISCOVER_URI}/library/metadata/{}/userState", encode(guid));
        let v: serde_json::Value = self.get(&url, Some(account_token)).call().map_err(err)?.body_mut().read_json().map_err(err)?;
        log::debug!("watchlist userState {guid}: {}", v["MediaContainer"]["UserState"]);
        Ok(watchlisted_in(&v))
    }

    pub fn set_watchlisted(&self, account_token: &str, guid: &str, on: bool) -> R<()> {
        let url = format!("{DISCOVER_URI}/actions/{}?ratingKey={}", if on { "addToWatchlist" } else { "removeFromWatchlist" }, encode(guid));
        self.headers(self.agent.put(&url), Some(account_token)).send_empty().map_err(err)?;
        Ok(())
    }

    pub fn auth_url(&self, code: &str) -> String {
        format!(
            "https://app.plex.tv/auth#?clientID={}&code={code}&context%5Bdevice%5D%5Bproduct%5D={PRODUCT}",
            self.client_id
        )
    }

    pub fn link_url(&self, code: &str) -> String {
        format!("https://www.plex.tv/link/?pin={code}")
    }

    pub fn check_pin(&self, id: u64) -> R<Option<String>> {
        let pin: Pin = self.get(&format!("{PLEX_TV}/pins/{id}"), None).call().map_err(err)?.body_mut().read_json().map_err(err)?;
        Ok(pin.auth_token.filter(|t| !t.is_empty()))
    }

    pub fn account_name(&self, token: &str) -> R<String> {
        #[derive(Deserialize, Default)]
        #[serde(default, rename_all = "camelCase")]
        struct User {
            friendly_name: String,
            title: String,
            username: String,
        }
        let u: User = self.get(&format!("{PLEX_TV}/user"), Some(token)).call().map_err(err)?.body_mut().read_json().map_err(err)?;
        Ok(first(&[&u.friendly_name, &u.title, &u.username]).to_owned())
    }

    pub fn servers(&self, token: &str) -> R<Vec<Resource>> {
        let all: Vec<Resource> = self
            .get(&format!("{PLEX_TV}/resources?includeHttps=1&includeRelay=1"), Some(token))
            .call()
            .map_err(err)?
            .body_mut()
            .read_json()
            .map_err(err)?;
        Ok(all.into_iter().filter(|r| r.provides.split(',').any(|p| p == "server")).collect())
    }

    pub fn connect(&self, res: &Resource, account_token: &str) -> R<Server> {
        let token = res.access_token.clone().unwrap_or_else(|| account_token.to_owned());
        let mut conns = res.connections.clone();
        conns.sort_by_key(|c| (c.relay, !c.local));
        let (tx, rx) = mpsc::channel();
        for (rank, c) in conns.iter().enumerate() {
            let (tx, client, uri, token) = (tx.clone(), self.clone(), c.uri.clone(), token.clone());
            std::thread::spawn(move || {
                let ok = client
                    .agent
                    .get(format!("{uri}/identity"))
                    .config()
                    .timeout_global(Some(Duration::from_secs(4)))
                    .build()
                    .header("X-Plex-Token", &token)
                    .call()
                    .is_ok();
                log::debug!("probe {uri}: {}", if ok { "ok" } else { "unreachable" });
                let _ = tx.send((rank, ok.then_some(uri)));
            });
        }
        drop(tx);
        let mut results: Vec<Option<String>> = vec![None; conns.len()];
        let mut done = vec![false; conns.len()];
        for (rank, uri) in rx {
            results[rank] = uri;
            done[rank] = true;
            if let Some(best) = (0..conns.len()).take_while(|&i| done[i]).find(|&i| results[i].is_some()) {
                set_down(&res.client_identifier, false);
                return Ok(Server {
                    name: res.name.clone(),
                    id: res.client_identifier.clone(),
                    uri: results[best].take().unwrap(),
                    token,
                    owned: res.owned,
                    ..Default::default()
                });
            }
        }
        set_down(&res.client_identifier, true);
        Err(format!("Could not reach {}", res.name))
    }
}

impl Server {
    pub fn is_jellyfin(&self) -> bool {
        self.backend == Backend::Jellyfin
    }

    fn get<T: DeserializeOwned + Default>(&self, cx: &Ctx, path: &str) -> R<T> {
        Ok(serde_json::from_slice::<Envelope<T>>(&self.fetch_cached(cx, path)?).map_err(err)?.media_container)
    }

    pub(crate) fn fetch_cached(&self, cx: &Ctx, path: &str) -> R<Vec<u8>> {
        self.try_fetch_cached(cx, path).inspect_err(|e| {
            if cx.source != Source::Cache {
                log::warn!("{} GET {path} failed: {e}", self.name)
            }
        })
    }

    pub(crate) fn try_fetch_cached(&self, cx: &Ctx, path: &str) -> R<Vec<u8>> {
        let file = store::cache_path("meta", &format!("{}{path}", self.id));
        if cx.source == Source::Cache {
            if is_down(&self.id) {
                return Err(format!("{} is offline", self.name));
            }
            std::fs::read(&file).map_err(err)
        } else {
            let started = std::time::Instant::now();
            let url = format!("{}{path}", self.uri);
            let request = match self.backend {
                Backend::Plex => cx.client.get(&url, Some(&self.token)),
                Backend::Jellyfin => jellyfin::authed(cx.client.agent.get(&url), &cx.client, Some(&self.token)),
            };
            let mut response = request.call().map_err(|e| {
                set_down(&self.id, unreachable(&e));
                err(e)
            })?;
            set_down(&self.id, false);
            let fresh = response.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(err)?;
            let changed = cx.source == Source::Live || std::fs::read(&file).ok().as_deref() != Some(&fresh[..]);
            log::debug!("{} GET {path} {}KB in {}ms{}", self.name, fresh.len() / 1024, started.elapsed().as_millis(), if changed { "" } else { " (unchanged)" });
            if changed {
                cx.changed.store(true, Ordering::Relaxed);
                if cx.source == Source::Network {
                    let _ = store::write_atomic(&file, &fresh);
                }
            }
            Ok(fresh)
        }
    }

    pub fn hubs(&self, c: &Ctx) -> R<Vec<Hub>> {
        if self.is_jellyfin() {
            return jellyfin::hubs(self, c);
        }
        let mut hubs = self.get::<Container>(c, "/hubs?count=20&includeLibraryPlaylists=0")?.hub;
        hubs.retain(|h| !h.metadata.is_empty() && !h.is_continue() && matches!(h.kind.as_str(), "movie" | "show" | "season" | "episode" | "mixed" | "clip"));
        if let Ok(cw) = self.get::<Container>(c, "/hubs/continueWatching?count=20")
            && let Some(h) = cw.hub.into_iter().find(|h| !h.metadata.is_empty())
        {
            hubs.insert(0, h);
        }
        Ok(hubs)
    }

    pub fn sections(&self, c: &Ctx) -> R<Vec<Directory>> {
        if self.is_jellyfin() {
            return jellyfin::sections(self, c);
        }
        let mut d = self.get::<Container>(c, "/library/sections")?.directory;
        d.retain(|d| matches!(d.kind.as_str(), "movie" | "show"));
        Ok(d)
    }

    pub fn section_items(&self, c: &Ctx, key: &str, start: usize, size: usize, sort: &str, summaries: bool) -> R<(Vec<Item>, usize)> {
        if self.is_jellyfin() {
            return jellyfin::section_items(self, c, key, start, size, sort, summaries);
        }
        let exclude = if summaries { "" } else { "&excludeFields=summary" };
        self.paged(c, &format!("/library/sections/{key}/all?sort={sort}{exclude}"), start, size)
    }

    pub fn section_hubs(&self, c: &Ctx, key: &str, kind: &str) -> R<Vec<Hub>> {
        if self.is_jellyfin() {
            return jellyfin::section_hubs(self, c, key, kind);
        }
        let mut hubs = self.get::<Container>(c, &format!("/hubs/sections/{key}?count=20&includeLibraryPlaylists=0"))?.hub;
        hubs.retain(|h| !h.metadata.is_empty() && matches!(h.kind.as_str(), "movie" | "show" | "season" | "episode" | "mixed" | "clip"));
        if let Some(i) = hubs.iter().position(Hub::is_continue) {
            let h = hubs.remove(i);
            hubs.insert(0, h);
        }
        Ok(hubs)
    }

    pub fn collections(&self, c: &Ctx, section: &str, start: usize, size: usize) -> R<(Vec<Item>, usize)> {
        if self.is_jellyfin() {
            return jellyfin::collections(self, c, start, size);
        }
        self.paged(c, &format!("/library/sections/{section}/collections"), start, size)
    }

    pub fn collection_items(&self, c: &Ctx, key: &str, start: usize, size: usize, summaries: bool) -> R<(Vec<Item>, usize)> {
        if self.is_jellyfin() {
            return jellyfin::collection_items(self, c, key, start, size, summaries);
        }
        self.paged(c, &format!("/library/collections/{key}/children"), start, size)
    }

    pub fn hub_items(&self, c: &Ctx, key: &str) -> R<Vec<Item>> {
        if self.is_jellyfin() {
            return jellyfin::hub_items(self, c, key);
        }
        let key = escape_uri(key);
        let mut out = Vec::new();
        loop {
            let (items, total) = self.paged(c, &key, out.len(), 100)?;
            let done = items.is_empty() || out.len() + items.len() >= total.min(HUB_MAX);
            out.extend(items);
            if done {
                return Ok(out);
            }
        }
    }

    pub fn watchlist(&self, c: &Ctx, kind: &str, start: usize, size: usize, sort: &str) -> R<(Vec<Item>, usize)> {
        let sort = if sort == "addedAt:desc" { "watchlistedAt:desc" } else { sort };
        let path = format!("/library/sections/watchlist/all?type={kind}&sort={sort}&includeGuids=1");
        let (mut items, mut total) = (Vec::new(), 0);
        while items.len() < size {
            // Discover rejects large container sizes, so page it in small chunks like plezy does.
            let (page, t) = self.paged(c, &path, start + items.len(), (size - items.len()).min(25))?;
            total = t;
            let done = page.is_empty() || start + items.len() + page.len() >= t;
            items.extend(page);
            if done {
                break;
            }
        }
        Ok((items, total))
    }

    fn paged(&self, c: &Ctx, path: &str, start: usize, size: usize) -> R<(Vec<Item>, usize)> {
        let sep = if path.contains('?') { '&' } else { '?' };
        let ct: Container = self.get(c, &format!("{path}{sep}X-Plex-Container-Start={start}&X-Plex-Container-Size={size}"))?;
        let total = ct.total_size.unwrap_or(start + ct.size);
        Ok((ct.metadata, total))
    }

    pub fn metadata(&self, c: &Ctx, key: &str) -> R<Item> {
        if self.is_jellyfin() {
            return jellyfin::metadata(self, c, key);
        }
        let mut it = self.get::<Container>(c, &format!("/library/metadata/{key}?includeOnDeck=1&includeMarkers=1&includeGuids=1&includeExtras=1"))?.metadata.into_iter().next().ok_or("Item not found")?;
        for (i, m) in it.media.iter_mut().enumerate() {
            m.source_id = i.to_string();
        }
        Ok(it)
    }

    pub fn related(&self, c: &Ctx, key: &str) -> R<Vec<Hub>> {
        if self.is_jellyfin() {
            return jellyfin::related(self, c, key);
        }
        let mut hubs = self.get::<Container>(c, &format!("/library/metadata/{key}/related?count=20"))?.hub;
        hubs.retain(|h| !h.metadata.is_empty());
        Ok(hubs)
    }

    fn library_action(&self, c: &Client, path: &str, put: bool) -> R<()> {
        let url = format!("{}{path}&identifier=com.plexapp.plugins.library", self.uri);
        if put { c.headers(c.agent.put(&url), Some(&self.token)).send_empty() } else { c.get(&url, Some(&self.token)).call() }.map_err(err)?;
        Ok(())
    }

    pub fn set_watched(&self, c: &Client, key: &str, watched: bool) -> R<()> {
        if self.is_jellyfin() {
            return jellyfin::set_watched(self, c, key, watched);
        }
        self.library_action(c, &format!("/:/{}?key={}", if watched { "scrobble" } else { "unscrobble" }, encode(key)), false)
    }

    pub fn remove_from_continue_watching(&self, c: &Client, key: &str) -> R<()> {
        if self.is_jellyfin() {
            return jellyfin::clear_progress(self, c, key);
        }
        let url = format!("{}/actions/removeFromContinueWatching?ratingKey={}", self.uri, encode(key));
        c.headers(c.agent.put(&url), Some(&self.token)).send_empty().map_err(err)?;
        Ok(())
    }

    pub fn can_search_subtitles(&self) -> bool {
        !self.is_jellyfin() && self.id != DISCOVER_ID
    }

    pub fn search_subtitles(&self, c: &Client, key: &str, language: &str) -> R<Vec<SubtitleResult>> {
        let url = format!("{}/library/metadata/{}/subtitles?language={}&hearingImpaired=0&forced=0", self.uri, encode(key), encode(language));
        let env: Envelope<SubtitleResults> = c.get(&url, Some(&self.token)).call().map_err(err)?.body_mut().read_json().map_err(err)?;
        Ok(env.media_container.stream)
    }

    pub fn download_subtitle(&self, c: &Client, key: &str, s: &SubtitleResult, language: &str) -> R<()> {
        let codec = first(&[&s.codec, "srt"]).to_owned();
        let language = first(&[&s.language_code, language]).to_owned();
        let params = [("key", s.key.clone()), ("codec", codec), ("language", language), ("hearingImpaired", u8::from(s.hearing_impaired).to_string()), ("forced", u8::from(s.forced).to_string()), ("providerTitle", s.provider_title.clone())];
        let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
        let url = format!("{}/library/metadata/{}/subtitles?{}", self.uri, encode(key), query.join("&"));
        c.headers(c.agent.put(&url), Some(&self.token)).send_empty().map_err(err)?;
        Ok(())
    }

    pub fn can_rate(&self) -> bool {
        !self.is_jellyfin() && self.id != DISCOVER_ID
    }

    pub fn rate(&self, c: &Client, key: &str, stars: Option<u8>) -> R<()> {
        let rating = stars.map_or(-1, |s| i32::from(s.min(5)) * 2);
        self.library_action(c, &format!("/:/rate?key={}&rating={rating}", encode(key)), true)
    }

    pub fn children(&self, c: &Ctx, key: &str) -> R<Vec<Item>> {
        if self.is_jellyfin() {
            return jellyfin::children(self, c, key);
        }
        Ok(self.get::<Container>(c, &format!("/library/metadata/{key}/children?excludeAllLeaves=1"))?.metadata)
    }

    pub fn all_episodes(&self, c: &Ctx, show: &str) -> R<Vec<Item>> {
        if self.is_jellyfin() {
            return jellyfin::all_episodes(self, c, show);
        }
        Ok(self.get::<Container>(c, &format!("/library/metadata/{show}/allLeaves"))?.metadata)
    }

    pub fn search(&self, c: &Ctx, q: &str) -> R<Vec<(Item, f64)>> {
        if self.is_jellyfin() {
            return jellyfin::search(self, c, q);
        }
        let q = encode(q);
        let scored: Vec<(Item, f64)> = self
            .get::<Container>(c, &format!("/library/search?query={q}&limit=40&searchTypes=movies,tv&includeCollections=0"))
            .map(|ct| ct.search_result.into_iter().filter_map(|h| Some((h.metadata?, h.score.unwrap_or(0.0)))).collect())
            .unwrap_or_default();
        if !scored.is_empty() {
            return Ok(scored);
        }
        let hubs = self.get::<Container>(c, &format!("/hubs/search?query={q}&limit=30"))?.hub;
        let items = hubs.into_iter().filter(|h| matches!(h.kind.as_str(), "movie" | "show" | "season" | "episode")).flat_map(|h| h.metadata);
        Ok(items.enumerate().map(|(i, it)| (it, 1.0 - i as f64 * 0.01)).collect())
    }

    pub fn timeline(&self, c: &Client, key: &str, state: &str, time_ms: u64, duration_ms: u64, transcode_session: Option<&str>) -> bool {
        if self.is_jellyfin() {
            return jellyfin::timeline(self, c, key, state, time_ms, transcode_session);
        }
        let url = format!(
            "{}/:/timeline?ratingKey={key}&key=%2Flibrary%2Fmetadata%2F{key}&state={state}&time={time_ms}&duration={duration_ms}",
            self.uri
        );
        match c.get(&url, Some(&self.token)).call() {
            Ok(_) => {
                log::debug!("{} timeline {state} {key} at {}s", self.name, time_ms / 1000);
                true
            }
            Err(e) => {
                log::warn!("{} timeline {state} for {key} failed: {e}", self.name);
                false
            }
        }
    }

    pub fn image_url(&self, path: &str, w: u32, h: u32) -> String {
        if self.is_jellyfin() {
            return jellyfin::image_url(self, path, w, h);
        }
        let (fill, format) = if path.contains("clearLogo") { (0, "png") } else { (1, "jpeg") };
        format!("{}/photo/:/transcode?width={w}&height={h}&minSize={fill}&upscale={fill}&format={format}&url={}&X-Plex-Token={}", self.uri, encode(path), self.token)
    }

    pub fn media_url(&self, key: &str) -> String {
        if self.is_jellyfin() {
            return jellyfin::media_url(self, key);
        }
        format!("{}{key}{}X-Plex-Token={}", self.uri, if key.contains('?') { '&' } else { '?' }, self.token)
    }

    pub fn stream_url(&self, c: &Client, key: &str, media: &Media, kbps: u32, force_transcode: bool) -> (String, Option<(&'static Quality, String)>) {
        let transcode = if force_transcode { Some(Quality::find(kbps).unwrap_or(&QUALITIES[0])) } else { Quality::find(kbps).filter(|q| !q.covers(media)) };
        let Some(q) = transcode else { return (media.part.first().map(|p| self.media_url(&p.key)).unwrap_or_default(), None) };
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let session = format!("plaivo-{nanos:x}");
        log::info!("transcoding {key} at {} kbps", q.kbps);
        (self.transcode_url(c, key, media, q, &session), Some((q, session)))
    }

    pub fn transcode_url(&self, c: &Client, key: &str, media: &Media, q: &Quality, session: &str) -> String {
        if self.is_jellyfin() {
            return jellyfin::transcode_url(self, c, key, &media.source_id, q, session);
        }
        let extra = format!(
            "add-limitation(scope=videoCodec&scopeName=*&type=upperBound&name=video.bitrate&value={}&replace=true)+add-transcode-target(type=videoProfile&context=streaming&protocol=hls&container=mpegts&videoCodec=h264&audioCodec=aac%2Cac3%2Ceac3%2Cmp3)",
            q.kbps
        );
        let (kbps, quality, path) = (q.kbps.to_string(), q.quality.to_string(), format!("/library/metadata/{key}"));
        let params = [
            ("path", path.as_str()),
            ("mediaIndex", first(&[&media.source_id, "0"])),
            ("partIndex", "0"),
            ("protocol", "hls"),
            ("fastSeek", "1"),
            ("directPlay", "0"),
            ("directStream", "0"),
            ("directStreamAudio", "1"),
            ("videoResolution", q.resolution),
            ("videoQuality", &quality),
            ("maxVideoBitrate", &kbps),
            ("subtitles", "none"),
            ("location", "lan"),
            ("autoAdjustQuality", "0"),
            ("mediaBufferSize", "102400"),
            ("session", session),
            ("X-Plex-Session-Identifier", session),
            ("X-Plex-Client-Profile-Extra", &extra),
            ("X-Plex-Client-Profile-Name", "Generic"),
            ("X-Plex-Platform", "Generic"),
            ("X-Plex-Product", PRODUCT),
            ("X-Plex-Client-Identifier", &c.client_id),
            ("X-Plex-Token", &self.token),
        ];
        let query: Vec<String> = params.iter().map(|(k, v)| format!("{k}={}", encode(v))).collect();
        format!("{}/video/:/transcode/universal/start.m3u8?{}", self.uri, query.join("&"))
    }

    pub fn stop_transcode(&self, c: &Client, session: &str) {
        if self.is_jellyfin() {
            return jellyfin::stop_transcode(self, c, session);
        }
        let url = format!("{}/video/:/transcode/universal/stop?session={}", self.uri, encode(session));
        if let Err(e) = c.get(&url, Some(&self.token)).call() {
            log::debug!("{} stop transcode {session}: {e}", self.name);
        }
    }
}

pub struct Quality {
    pub kbps: u32,
    pub height: u32,
    pub resolution: &'static str,
    pub quality: u8,
    pub label: &'static str,
}

pub const QUALITIES: &[Quality] = &[
    Quality { kbps: 20000, height: 1080, resolution: "1920x1080", quality: 100, label: "1080p (High)" },
    Quality { kbps: 12000, height: 1080, resolution: "1920x1080", quality: 90, label: "1080p (Medium)" },
    Quality { kbps: 10000, height: 1080, resolution: "1920x1080", quality: 75, label: "1080p" },
    Quality { kbps: 8000, height: 1080, resolution: "1920x1080", quality: 60, label: "1080p (Low)" },
    Quality { kbps: 4000, height: 720, resolution: "1280x720", quality: 100, label: "720p (High)" },
    Quality { kbps: 3000, height: 720, resolution: "1280x720", quality: 75, label: "720p (Medium)" },
    Quality { kbps: 2000, height: 720, resolution: "1280x720", quality: 60, label: "720p" },
    Quality { kbps: 1500, height: 480, resolution: "720x480", quality: 60, label: "480p" },
    Quality { kbps: 720, height: 320, resolution: "576x320", quality: 40, label: "320p" },
];

impl Quality {
    pub fn find(kbps: u32) -> Option<&'static Quality> {
        QUALITIES.iter().find(|q| q.kbps == kbps)
    }

    pub fn covers(&self, m: &Media) -> bool {
        matches!((m.bitrate, m.height), (Some(b), Some(h)) if b <= self.kbps as u64 && h <= self.height as u64)
    }
}

pub fn fmt_bitrate(kbps: u64) -> String {
    if kbps >= 1000 { format!("{:.1} Mbps", kbps as f64 / 1000.0).replace(".0 ", " ") } else { format!("{kbps} kbps") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_hub_lists_open_on_their_date_sort() {
        assert_eq!(initial_sort("/library/sections/1/all?sort=originallyAvailableAt:desc&originallyAvailableAt>=-1y"), "originallyAvailableAt:desc");
        assert_eq!(initial_sort("/hubs/home/recentlyAdded?type=1&sectionID=1"), "addedAt:desc");
        assert_eq!(initial_sort("latest:abc"), "addedAt:desc");
        assert_eq!(initial_sort("/library/sections/1/all?sort=lastViewedAt:desc"), "");
        assert_eq!(initial_sort("/hubs/continueWatching/items"), "");
    }

    #[test]
    fn hub_lists_sort_with_missing_values_last() {
        let a = Item { added_at: Some(5), audience_rating: Some(7.), year: Some(1999), ..Item { rating_key: "a".into(), kind: "movie".into(), title: "beta".into(), ..Default::default() } };
        let b = Item { audience_rating: Some(9.), originally_available_at: "2005-01-02".into(), ..Item { rating_key: "b".into(), kind: "movie".into(), title: "Alpha".into(), ..Default::default() } };
        let c = Item { added_at: Some(9), grandparent_title: "Gamma".into(), ..Item { rating_key: "c".into(), kind: "episode".into(), title: "Aardvark".into(), ..Default::default() } };
        let keys = |sort: &str| {
            let mut v = vec![&a, &b, &c];
            sort_items(&mut v, sort);
            v.iter().map(|it| it.rating_key.as_str()).collect::<String>()
        };
        assert_eq!(keys(""), "abc");
        assert_eq!(keys("titleSort"), "bac");
        assert_eq!(keys("addedAt:desc"), "cab");
        assert_eq!(keys("originallyAvailableAt:desc"), "bac");
        assert_eq!(keys("audienceRating:desc"), "bac");
    }

    #[test]
    fn parses_metadata_container() {
        let json = r#"{"MediaContainer":{"size":1,"totalSize":40,"Metadata":[{"ratingKey":"7","type":"episode","title":"Pilot","parentIndex":1,"index":2,"duration":1000,"viewOffset":250,"Media":[{"Part":[{"key":"/library/parts/1/file.mkv","Stream":[{"streamType":3,"key":"/library/streams/9","codec":"srt","languageCode":"eng","displayTitle":"English","forced":true}]}]}]}]}}"#;
        let ct: Envelope<Container> = serde_json::from_str(json).unwrap();
        let it = &ct.media_container.metadata[0];
        assert_eq!(ct.media_container.total_size, Some(40));
        assert_eq!(it.episode_label(), "S1 · E2");
        assert_eq!(it.progress(), 0.25);
        assert_eq!(it.media[0].part[0].stream[0].key, "/library/streams/9");
        assert!(it.media[0].part[0].stream[0].forced);
    }

    #[test]
    fn hub_keys_escape_only_characters_invalid_in_uris() {
        assert_eq!(escape_uri("/library/sections/1/all?sort=a:desc&originallyAvailableAt>=-1y&t=a b%20"), "/library/sections/1/all?sort=a:desc&originallyAvailableAt%3E=-1y&t=a%20b%20");
    }

    #[test]
    fn continue_hubs_detected_by_any_id() {
        for id in ["home.continue", "continueWatching", "home.ondeck", "home.onDeck", "movie.inprogress.1", "tv.ondeck.2"] {
            assert!(Hub { id: id.into(), ..Default::default() }.is_continue(), "{id}");
        }
        assert!(!Hub { id: "home.movies.recent".into(), ..Default::default() }.is_continue());
    }

    #[test]
    fn episodes_collapse_into_their_show() {
        let ep = Item { kind: "episode".into(), rating_key: "9".into(), grandparent_rating_key: "1".into(), grandparent_title: "Dexter".into(), ..Default::default() };
        let show = ep.as_show();
        assert_eq!((show.kind.as_str(), show.rating_key.as_str(), show.title.as_str()), ("show", "1", "Dexter"));
        assert_eq!(encode("a b/c:d"), "a%20b%2Fc%3Ad");
    }

    #[test]
    fn parses_search_results() {
        let json = r#"{"MediaContainer":{"SearchResult":[{"score":0.9,"Metadata":{"ratingKey":"5","type":"movie","title":"X"}}],"SearchResults":[{"SearchResult":[{"Metadata":{"ratingKey":"abc","guid":"plex://show/1","type":"show","title":"Y"}}]}]}}"#;
        let ct: Envelope<Container> = serde_json::from_str(json).unwrap();
        assert_eq!(ct.media_container.search_result[0].score, Some(0.9));
        assert_eq!(ct.media_container.search_results[0].search_result[0].metadata.as_ref().unwrap().guid, "plex://show/1");
    }

    #[test]
    fn markers_and_quality() {
        let json = r#"{"MediaContainer":{"Metadata":[{"ratingKey":"7","Marker":[{"type":"intro","startTimeOffset":1000,"endTimeOffset":60000}],"Media":[{"bitrate":3100,"height":1080}]}]}}"#;
        let ct: Envelope<Container> = serde_json::from_str(json).unwrap();
        let it = &ct.media_container.metadata[0];
        assert_eq!((it.marker[0].kind.as_str(), it.marker[0].end_time_offset), ("intro", 60000));
        assert!(Quality::find(4000).unwrap().covers(&Media { bitrate: Some(3100), height: Some(720), ..Default::default() }));
        assert!(!Quality::find(4000).unwrap().covers(&it.media[0]));
        assert_eq!((fmt_bitrate(3100), fmt_bitrate(20000), fmt_bitrate(720)), ("3.1 Mbps".into(), "20 Mbps".into(), "720 kbps".into()));
        let url = Server { name: "s".into(), id: "i".into(), uri: "http://h".into(), token: "t".into(), owned: true, ..Default::default() }.transcode_url(&Client::new("c".into()), "7", &Media::default(), &QUALITIES[0], "s1");
        assert!(url.starts_with("http://h/video/:/transcode/universal/start.m3u8?path=%2Flibrary%2Fmetadata%2F7&"));
        assert!(url.contains("scopeName%3D%2A") && url.contains("aac%252Cac3"));
    }

    #[test]
    fn detail_metadata() {
        let json = r#"{"MediaContainer":{"Metadata":[{"ratingKey":"7","guid":"plex://movie/5d77","type":"movie","primaryExtraKey":"/library/metadata/91","userRating":8.0,
            "Role":[{"tag":"Jacob Elordi","role":"Hig","thumb":"https://img/x.jpg"}],"Director":[{"tag":"Ridley Scott"}],
            "Rating":[{"image":"imdb://image.rating","value":6.3,"type":"audience"},{"image":"rottentomatoes://image.rating.rotten","value":4.1,"type":"critic"},{"image":"rottentomatoes://image.rating.upright","value":6.9,"type":"audience"},{"image":"themoviedb://image.rating","value":7.2,"type":"audience"}],
            "Image":[{"type":"coverPoster","url":"/p"},{"type":"clearLogo","url":"/library/metadata/7/clearLogo/1"}],
            "Extras":{"Metadata":[{"ratingKey":"90","type":"clip","subtype":"trailer"},{"ratingKey":"91","type":"clip","subtype":"trailer"}]},
            "Media":[{"videoResolution":"4k","videoCodec":"hevc","videoDynamicRange":"DV P8","Part":[{"Stream":[{"streamType":2,"displayTitle":"Commentary"},{"streamType":2,"displayTitle":"English (EAC3 5.1)","selected":true},{"streamType":3,"displayTitle":"English (SRT)"}]}]}]}]}}"#;
        let it = serde_json::from_str::<Envelope<Container>>(json).unwrap().media_container.metadata.remove(0);
        let labels: Vec<String> = it.rating_labels().into_iter().map(|(s, v)| format!("{s:?} {v}")).collect();
        assert_eq!(labels, ["Imdb 6.3", "Critics 41%", "Audience 69%", "Tmdb 72%"]);
        let audio = |codec: &str, ch| Item { media: vec![Media { audio_codec: codec.into(), audio_channels: ch, ..Default::default() }], ..Default::default() }.audio_label();
        assert_eq!((audio("eac3", Some(6)), audio("aac", Some(2)), audio("truehd", None)), ("EAC3 5.1".into(), "AAC 2.0".into(), "TRUEHD".into()));
        assert_eq!((it.logo(), it.plex_guid(), it.user_rating), ("/library/metadata/7/clearLogo/1", Some("5d77"), Some(8.0)));
        assert_eq!(it.trailer().map(|t| t.rating_key.as_str()), Some("91"), "primary extra wins over the first trailer");
        assert_eq!(it.media_line(0), "4K · HEVC · DV P8 · English (EAC3 5.1) · English (SRT)");
        assert_eq!((it.role[0].role.as_str(), it.director[0].tag.as_str()), ("Hig", "Ridley Scott"));
        assert_eq!(Item { audience_rating: Some(7.25), ..Default::default() }.rating_labels(), [(RatingSource::Stars, "7.2".to_owned())]);
        assert_eq!(Item { guid: "com.plexapp.agents.imdb://tt1".into(), ..Default::default() }.plex_guid(), None);
        let state = |s: &str| watchlisted_in(&serde_json::from_str(s).unwrap());
        assert!(state(r#"{"MediaContainer":{"UserState":{"watchlistedAt":{"value":1731000000}}}}"#));
        assert!(state(r#"{"MediaContainer":{"UserState":[{"watchlistedAt":1731000000}]}}"#));
        assert!(!state(r#"{"MediaContainer":{"UserState":{"viewCount":0}}}"#));
    }

    #[test]
    fn parses_subtitle_search_results() {
        let json = r#"{"MediaContainer":{"size":2,"Stream":[{"id":1,"key":"/sync/subtitles/abc","codec":"srt","languageCode":"en","language":"English","providerTitle":"OpenSubtitles","displayTitle":"English (SRT)","hearingImpaired":"1","perfectMatch":true,"forced":0,"score":"8.5"},{"key":"/sync/subtitles/def","score":3}]}}"#;
        let r = serde_json::from_str::<Envelope<SubtitleResults>>(json).unwrap().media_container.stream;
        assert_eq!((r[0].hearing_impaired, r[0].perfect_match, r[0].forced, r[0].score), (true, true, false, Some(8.5)));
        assert_eq!((r[0].provider_title.as_str(), r[1].score, r[1].codec.as_str()), ("OpenSubtitles", Some(3.0), ""));
    }

    #[test]
    fn media_urls_keep_existing_queries() {
        let srv = Server { uri: "http://h".into(), token: "t".into(), ..Default::default() };
        assert_eq!(srv.media_url("/library/parts/1/file.mkv"), "http://h/library/parts/1/file.mkv?X-Plex-Token=t");
        assert_eq!(srv.media_url("/services/iva/assets/9/video.mp4?fmt=4&bitrate=5000"), "http://h/services/iva/assets/9/video.mp4?fmt=4&bitrate=5000&X-Plex-Token=t");
    }

    #[test]
    fn offline_servers_skip_the_cache() {
        let srv = Server { name: "nas".into(), id: "offline-test".into(), ..Default::default() };
        let ctx = Ctx::new(Client::new("c".into()), Source::Cache);
        set_down(&srv.id, true);
        assert_eq!(srv.try_fetch_cached(&ctx, "/library/sections").unwrap_err(), "nas is offline");
        set_down(&srv.id, false);
        assert!(!is_down(&srv.id));
    }

    #[test]
    fn watched_rules() {
        let show = Item { leaf_count: Some(3), viewed_leaf_count: Some(3), ..Default::default() };
        let movie = Item { view_count: Some(1), ..Default::default() };
        assert!(show.watched() && movie.watched() && !Item::default().watched());
        assert_eq!(fmt_duration(95 * 60_000), "1h 35m");
    }
}
