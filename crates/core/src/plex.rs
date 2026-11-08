use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
#[cfg(not(target_os = "android"))]
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

use crate::{first, join, store};

pub type R<T> = Result<T, String>;

const PRODUCT: &str = "Plaivo";
const PLEX_TV: &str = "https://plex.tv/api/v2";
pub const DISCOVER_ID: &str = "discover";
const DISCOVER_URI: &str = "https://discover.provider.plex.tv";

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    pub client_id: Arc<str>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub name: String,
    pub id: String,
    pub uri: String,
    pub token: String,
    #[serde(default)]
    pub owned: bool,
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
    #[serde(rename = "hubIdentifier")]
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "Metadata")]
    pub metadata: Vec<Item>,
}

#[derive(Deserialize, Default, Clone)]
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
    #[serde(rename = "OnDeck")]
    pub on_deck: Option<Box<OnDeck>>,
    #[serde(rename = "Genre")]
    pub genre: Vec<Tag>,
    #[serde(rename = "Media")]
    pub media: Vec<Media>,
    #[serde(rename = "Marker")]
    pub marker: Vec<Marker>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Marker {
    #[serde(rename = "type")]
    pub kind: String,
    pub start_time_offset: u64,
    pub end_time_offset: u64,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct OnDeck {
    #[serde(rename = "Metadata")]
    pub metadata: Option<Item>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Tag {
    pub tag: String,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Media {
    pub bitrate: Option<u64>,
    pub height: Option<u64>,
    pub container: String,
    pub video_resolution: String,
    pub video_dynamic_range: String,
    pub audio_codec: String,
    pub audio_channels: Option<u32>,
    #[serde(rename = "Part")]
    pub part: Vec<Part>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Part {
    pub key: String,
    pub file: String,
    #[serde(rename = "Stream")]
    pub stream: Vec<Stream>,
}

#[derive(Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Stream {
    pub stream_type: u8,
    pub key: String,
    pub codec: String,
    pub language_code: String,
    pub display_title: String,
}

impl Hub {
    pub fn is_continue(&self) -> bool {
        let id = self.id.to_ascii_lowercase();
        id.contains("continue") || id.contains("ondeck")
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

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
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
        let config = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(5))).timeout_global(Some(Duration::from_secs(30))).http_status_as_error(true);
        #[cfg(not(target_os = "android"))]
        let config = config.tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build());
        let agent = config.build().into();
        Self { agent, client_id: client_id.into() }
    }

    fn get(&self, url: &str, token: Option<&str>) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let mut r = self
            .agent
            .get(url)
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
        Server { name: "Plex".into(), id: DISCOVER_ID.into(), uri: DISCOVER_URI.into(), token: account_token.into(), owned: false }
    }

    pub fn discover_search(&self, q: &str, account_token: &str) -> R<Vec<Item>> {
        let url = format!("{DISCOVER_URI}/library/search?query={}&limit=30&searchTypes=movies,tv&searchProviders=discover&includeMetadata=1", encode(q));
        let env: Envelope<Container> = self.get(&url, Some(account_token)).call().map_err(err)?.body_mut().with_config().limit(16 << 20).read_json().map_err(err)?;
        Ok(env.media_container.search_results.into_iter().flat_map(|g| g.search_result).filter_map(|h| h.metadata).collect())
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
                return Ok(Server {
                    name: res.name.clone(),
                    id: res.client_identifier.clone(),
                    uri: results[best].take().unwrap(),
                    token,
                    owned: res.owned,
                });
            }
        }
        Err(format!("Could not reach {}", res.name))
    }
}

impl Server {
    fn get<T: DeserializeOwned + Default>(&self, cx: &Ctx, path: &str) -> R<T> {
        let file = store::cache_path("meta", &format!("{}{path}", self.id));
        let bytes = if cx.source == Source::Cache {
            std::fs::read(&file).map_err(err)?
        } else {
            let started = std::time::Instant::now();
            let fresh = cx
                .client
                .get(&format!("{}{path}", self.uri), Some(&self.token))
                .call()
                .map_err(err)
                .and_then(|mut r| r.body_mut().with_config().limit(64 << 20).read_to_vec().map_err(err))
                .inspect_err(|e| log::warn!("{} GET {path} failed: {e}", self.name))?;
            let changed = cx.source == Source::Live || std::fs::read(&file).ok().as_deref() != Some(&fresh[..]);
            log::debug!("{} GET {path} {}KB in {}ms{}", self.name, fresh.len() / 1024, started.elapsed().as_millis(), if changed { "" } else { " (unchanged)" });
            if changed {
                cx.changed.store(true, Ordering::Relaxed);
                if cx.source == Source::Network {
                    let _ = store::write_atomic(&file, &fresh);
                }
            }
            fresh
        };
        Ok(serde_json::from_slice::<Envelope<T>>(&bytes).map_err(err)?.media_container)
    }

    pub fn hubs(&self, c: &Ctx) -> R<Vec<Hub>> {
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
        let mut d = self.get::<Container>(c, "/library/sections")?.directory;
        d.retain(|d| matches!(d.kind.as_str(), "movie" | "show"));
        Ok(d)
    }

    pub fn section_items(&self, c: &Ctx, key: &str, start: usize, size: usize, sort: &str) -> R<(Vec<Item>, usize)> {
        let ct: Container = self.get(
            c,
            &format!("/library/sections/{key}/all?sort={sort}&excludeFields=summary&X-Plex-Container-Start={start}&X-Plex-Container-Size={size}"),
        )?;
        let total = ct.total_size.unwrap_or(start + ct.size);
        Ok((ct.metadata, total))
    }

    pub fn metadata(&self, c: &Ctx, key: &str) -> R<Item> {
        self.get::<Container>(c, &format!("/library/metadata/{key}?includeOnDeck=1&includeMarkers=1"))?.metadata.into_iter().next().ok_or_else(|| "Item not found".into())
    }

    pub fn children(&self, c: &Ctx, key: &str) -> R<Vec<Item>> {
        Ok(self.get::<Container>(c, &format!("/library/metadata/{key}/children?excludeAllLeaves=1"))?.metadata)
    }

    pub fn all_episodes(&self, c: &Ctx, show: &str) -> R<Vec<Item>> {
        Ok(self.get::<Container>(c, &format!("/library/metadata/{show}/allLeaves"))?.metadata)
    }

    pub fn search(&self, c: &Ctx, q: &str) -> R<Vec<(Item, f64)>> {
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

    pub fn timeline(&self, c: &Client, key: &str, state: &str, time_ms: u64, duration_ms: u64) {
        let url = format!(
            "{}/:/timeline?ratingKey={key}&key=%2Flibrary%2Fmetadata%2F{key}&state={state}&time={time_ms}&duration={duration_ms}",
            self.uri
        );
        match c.get(&url, Some(&self.token)).call() {
            Ok(_) => log::debug!("{} timeline {state} {key} at {}s", self.name, time_ms / 1000),
            Err(e) => log::warn!("{} timeline {state} for {key} failed: {e}", self.name),
        }
    }

    pub fn image_url(&self, path: &str, w: u32, h: u32) -> String {
        format!(
            "{}/photo/:/transcode?width={w}&height={h}&minSize=1&upscale=1&format=jpeg&url={}&X-Plex-Token={}",
            self.uri,
            encode(path),
            self.token
        )
    }

    pub fn media_url(&self, key: &str) -> String {
        format!("{}{key}?X-Plex-Token={}", self.uri, self.token)
    }

    pub fn transcode_url(&self, c: &Client, key: &str, q: &Quality, session: &str) -> String {
        let extra = format!(
            "add-limitation(scope=videoCodec&scopeName=*&type=upperBound&name=video.bitrate&value={}&replace=true)+add-transcode-target(type=videoProfile&context=streaming&protocol=hls&container=mpegts&videoCodec=h264&audioCodec=aac%2Cac3%2Ceac3%2Cmp3)",
            q.kbps
        );
        let (kbps, quality, path) = (q.kbps.to_string(), q.quality.to_string(), format!("/library/metadata/{key}"));
        let params = [
            ("path", path.as_str()),
            ("mediaIndex", "0"),
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
    fn parses_metadata_container() {
        let json = r#"{"MediaContainer":{"size":1,"totalSize":40,"Metadata":[{"ratingKey":"7","type":"episode","title":"Pilot","parentIndex":1,"index":2,"duration":1000,"viewOffset":250,"Media":[{"Part":[{"key":"/library/parts/1/file.mkv","Stream":[{"streamType":3,"key":"/library/streams/9","codec":"srt","languageCode":"eng","displayTitle":"English"}]}]}]}]}}"#;
        let ct: Envelope<Container> = serde_json::from_str(json).unwrap();
        let it = &ct.media_container.metadata[0];
        assert_eq!(ct.media_container.total_size, Some(40));
        assert_eq!(it.episode_label(), "S1 · E2");
        assert_eq!(it.progress(), 0.25);
        assert_eq!(it.media[0].part[0].stream[0].key, "/library/streams/9");
    }

    #[test]
    fn continue_hubs_detected_by_any_id() {
        for id in ["home.continue", "continueWatching", "home.ondeck", "home.onDeck"] {
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
        let url = Server { name: "s".into(), id: "i".into(), uri: "http://h".into(), token: "t".into(), owned: true }.transcode_url(&Client::new("c".into()), "7", &QUALITIES[0], "s1");
        assert!(url.starts_with("http://h/video/:/transcode/universal/start.m3u8?path=%2Flibrary%2Fmetadata%2F7&"));
        assert!(url.contains("scopeName%3D%2A") && url.contains("aac%252Cac3"));
    }

    #[test]
    fn watched_rules() {
        let show = Item { leaf_count: Some(3), viewed_leaf_count: Some(3), ..Default::default() };
        let movie = Item { view_count: Some(1), ..Default::default() };
        assert!(show.watched() && movie.watched() && !Item::default().watched());
        assert_eq!(fmt_duration(95 * 60_000), "1h 35m");
    }
}
