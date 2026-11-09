mod anime;
mod http;
mod lists;
mod trakt;

use crate::downloads::Downloads;
use crate::plex::{Client, Ctx, Item, Server, Source};
use crate::store;
use http::{Request, Response, send};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

pub use crate::watch_together::ids_from_guids;

pub type Ids = BTreeMap<String, String>;

const ACCOUNTS_FILE: &str = "trackers.json";
const QUEUE_FILE: &str = "tracker-queue.json";
const QUEUE_MAX: usize = 50;

fn env(v: Option<&'static str>) -> Option<&'static str> {
    v.filter(|s| !s.trim().is_empty())
}

fn trakt_id() -> Option<&'static str> {
    env(option_env!("PLAIVO_TRAKT_ID"))
}

fn relay() -> &'static str {
    env(option_env!("PLAIVO_OAUTH_RELAY")).unwrap_or("https://relay.plaivo.app")
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

const ID_KINDS: [&str; 5] = ["imdb", "tmdb", "tvdb", "anidb", "mal"];

fn item_ids(it: &Item) -> Ids {
    let mut ids = ids_from_guids(it.guids.iter().map(|g| g.id.as_str()));
    ids.retain(|k, _| ID_KINDS.contains(&k.as_str()));
    ids
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Trakt,
    Mal,
}

impl Service {
    pub const ALL: [Service; 2] = [Service::Trakt, Service::Mal];

    fn key(self) -> &'static str {
        match self {
            Service::Trakt => "trakt",
            Service::Mal => "mal",
        }
    }

    pub fn configured(self) -> bool {
        match self {
            Service::Trakt => trakt_id().is_some(),
            Service::Mal => true,
        }
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Token {
    access: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<String>,
    #[serde(default)]
    expires_at: u64,
}

impl Token {
    fn parse(v: &Value, now: u64, previous_refresh: Option<String>) -> Option<Token> {
        let access = v["access_token"].as_str().filter(|s| !s.is_empty())?.to_owned();
        let refresh = v["refresh_token"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).or(previous_refresh);
        Some(Token { access, refresh, expires_at: v["expires_in"].as_u64().map_or(0, |s| now + s) })
    }

    fn stale(&self, now: u64) -> bool {
        self.expires_at != 0 && now + 300 >= self.expires_at
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Account {
    token: Option<Token>,
    scrobble: bool,
}

impl Default for Account {
    fn default() -> Self {
        Self { token: None, scrobble: true }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Accounts {
    trakt: Account,
    mal: Account,
    trakt_progress: bool,
}

impl Default for Accounts {
    fn default() -> Self {
        Self { trakt: Account::default(), mal: Account::default(), trakt_progress: true }
    }
}

impl Accounts {
    fn get(&self, s: Service) -> &Account {
        match s {
            Service::Trakt => &self.trakt,
            Service::Mal => &self.mal,
        }
    }

    fn get_mut(&mut self, s: Service) -> &mut Account {
        match s {
            Service::Trakt => &mut self.trakt,
            Service::Mal => &mut self.mal,
        }
    }

    fn active(&self, s: Service, configured: [bool; 2]) -> bool {
        configured[s as usize] && self.get(s).token.is_some() && self.get(s).scrobble
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Media {
    pub movie: bool,
    pub ids: Ids,
    pub show_ids: Ids,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub order: Option<EpisodeOrder>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EpisodeOrder {
    Tvdb,
    Tmdb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Stopped,
}

impl PlayState {
    pub fn parse(state: &str) -> Option<Self> {
        match state {
            "playing" => Some(Self::Playing),
            "paused" => Some(Self::Paused),
            "stopped" => Some(Self::Stopped),
            _ => None,
        }
    }
}

impl Media {
    pub fn for_item(it: &Item) -> Option<Self> {
        let number = |n: Option<i64>| n.and_then(|n| u32::try_from(n).ok());
        match it.kind.as_str() {
            "movie" => Some(Media { movie: true, ids: item_ids(it), ..Media::default() }),
            "episode" => Some(Media { ids: item_ids(it), season: number(it.parent_index), episode: number(it.index), ..Media::default() }),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub configured: bool,
    pub signed_in: bool,
    pub scrobble: bool,
}

pub struct SignIn {
    pub service: Service,
    pub url: String,
    pub user_code: Option<String>,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
    secret: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    Pending,
    Done,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scrobble {
    Idle,
    Started,
    Paused,
    Stopped,
}

#[derive(Debug, PartialEq)]
enum Action {
    Trakt(&'static str, f64),
    Watched(f64),
}

struct Session {
    media: Media,
    scrobble: Scrobble,
    last: Option<f64>,
    watched: bool,
}

impl Session {
    fn new(media: Media) -> Self {
        Self { media, scrobble: Scrobble::Idle, last: None, watched: false }
    }

    fn step(&mut self, state: PlayState, pct: f64, threshold: f64, realtime: bool) -> Vec<Action> {
        let mut out = Vec::new();
        if realtime {
            let (next, action) = match (state, self.scrobble) {
                (PlayState::Playing, s) if s != Scrobble::Started => (Scrobble::Started, Some("start")),
                (PlayState::Paused, Scrobble::Started) => (Scrobble::Paused, Some("pause")),
                (PlayState::Stopped, Scrobble::Started | Scrobble::Paused) => (Scrobble::Stopped, Some("stop")),
                (_, s) => (s, None),
            };
            self.scrobble = next;
            out.extend(action.map(|a| Action::Trakt(a, pct)));
        }
        if !self.watched && self.last.is_some_and(|l| l < threshold) && pct >= threshold {
            self.watched = true;
            out.push(Action::Watched(pct));
        }
        self.last = Some(pct);
        out
    }

    fn end(&self) -> Option<Action> {
        matches!(self.scrobble, Scrobble::Started | Scrobble::Paused).then(|| Action::Trakt("stop", self.last.unwrap_or(0.0)))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Write {
    service: Service,
    media: Media,
    pct: f64,
}

#[derive(Debug, PartialEq)]
enum Job {
    Scrobble(&'static str, Media, f64),
    Write(Write),
    Retry,
}

fn jobs(actions: Vec<Action>, media: &Media, active: impl Fn(Service) -> bool, realtime: bool) -> Vec<Job> {
    let mut out = Vec::new();
    for action in actions {
        match action {
            Action::Trakt("stop", pct) if pct >= trakt::WATCHED_PCT => out.push(Job::Write(Write { service: Service::Trakt, media: media.clone(), pct })),
            Action::Trakt(a, pct) => out.push(Job::Scrobble(a, media.clone(), pct)),
            Action::Watched(pct) => out.extend(
                Service::ALL
                    .into_iter()
                    .filter(|&s| active(s) && !(s == Service::Trakt && realtime))
                    .map(|service| Job::Write(Write { service, media: media.clone(), pct })),
            ),
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Done,
    Retry,
    Drop,
}

fn classify(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Done,
        408 | 429 | 500..=599 => Outcome::Retry,
        _ => Outcome::Drop,
    }
}

struct Queue {
    path: PathBuf,
    items: Vec<Write>,
}

impl Queue {
    fn load(path: PathBuf) -> Self {
        let items = store::read_json(&path).unwrap_or_default();
        Self { path, items }
    }

    fn push(&mut self, w: Write) {
        self.items.push(w);
        let over = self.items.len().saturating_sub(QUEUE_MAX);
        self.items.drain(..over);
        store::write_json(&self.path, &self.items);
    }

    fn remove(&mut self, w: &Write) {
        if let Some(i) = self.items.iter().rposition(|x| x == w) {
            self.items.remove(i);
            store::write_json(&self.path, &self.items);
        }
    }

}

struct Shared {
    accounts: Accounts,
    session: Option<Session>,
    threshold: f64,
    shows: HashMap<(String, String), Ids>,
}

struct Inner {
    dir: PathBuf,
    configured: [bool; 2],
    shared: Mutex<Shared>,
    queue: Mutex<Queue>,
    tx: Sender<Job>,
}

#[derive(Clone)]
pub struct Trackers(Arc<Inner>);

impl Trackers {
    pub fn start() -> Self {
        let config = store::config_dir();
        let (trackers, rx) = Self::detached(config.clone(), Service::ALL.map(Service::configured));
        let (weak, cache) = (Arc::downgrade(&trackers.0), store::cache_dir());
        let spawned = std::thread::Builder::new().name("trackers".into()).spawn(move || worker(weak, rx, anime::Mappings::new(cache)));
        if let Err(e) = spawned {
            log::error!("could not start tracker worker: {e}");
        }
        let _ = trackers.0.tx.send(Job::Retry);
        trackers
    }

    fn detached(config: PathBuf, configured: [bool; 2]) -> (Self, Receiver<Job>) {
        let accounts = store::read_json(&config.join(ACCOUNTS_FILE)).unwrap_or_default();
        let (tx, rx) = channel();
        let shared = Mutex::new(Shared { accounts, session: None, threshold: 90.0, shows: HashMap::new() });
        let queue = Mutex::new(Queue::load(config.join(QUEUE_FILE)));
        (Self(Arc::new(Inner { dir: config, configured, shared, queue, tx })), rx)
    }

    pub fn status(&self, service: Service) -> Status {
        let s = self.0.lock();
        let a = s.accounts.get(service);
        let configured = self.0.configured[service as usize];
        Status { configured, signed_in: configured && a.token.is_some(), scrobble: a.scrobble }
    }

    pub fn set_scrobble(&self, service: Service, on: bool) {
        self.0.update(|a| a.get_mut(service).scrobble = on);
    }

    pub fn trakt_progress(&self) -> bool {
        self.0.lock().accounts.trakt_progress
    }

    pub fn set_trakt_progress(&self, on: bool) {
        self.0.update(|a| a.trakt_progress = on);
    }

    pub fn set_watched_threshold(&self, pct: f64) {
        self.0.lock().threshold = pct.clamp(1.0, 100.0);
    }

    pub fn sign_out(&self, service: Service) {
        self.0.update(|a| a.get_mut(service).token = None);
    }

    pub fn begin_sign_in(&self, service: Service) -> Result<SignIn, String> {
        if !self.0.configured[service as usize] {
            return Err("not configured".into());
        }
        let req = match service {
            Service::Trakt => trakt::device_code_request(trakt_id().unwrap_or_default()),
            s => lists::start_request(relay(), s),
        };
        let resp = send(&req)?;
        let v = resp.json();
        if resp.status != 200 {
            return Err(v["error"].as_str().map_or_else(|| format!("HTTP {}", resp.status), str::to_owned));
        }
        let text = |k: &str| v[k].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        let (secret, url) = match service {
            Service::Trakt => (text("device_code"), text("verification_url")),
            _ => (text("session"), text("authorize_url")),
        };
        let (Some(secret), Some(url)) = (secret, url) else { return Err("invalid response".into()) };
        let user_code = text("user_code");
        Ok(SignIn {
            service,
            url: match &user_code {
                Some(code) => format!("{}/{code}", url.trim_end_matches('/')),
                None => url,
            },
            user_code,
            interval_secs: v["interval"].as_u64().unwrap_or(3).clamp(1, 60),
            expires_in_secs: v["expires_in"].as_u64().unwrap_or(600),
            secret,
        })
    }

    pub fn poll_sign_in(&self, sign_in: &mut SignIn) -> Poll {
        let s = sign_in.service;
        let req = match s {
            Service::Trakt => trakt::device_token_request(trakt_id().unwrap_or_default(), &sign_in.secret),
            _ => lists::result_request(relay(), &sign_in.secret),
        };
        let Ok(resp) = send(&req) else { return Poll::Pending };
        match (s, resp.status) {
            (_, 200) => match Token::parse(&resp.json(), now(), None) {
                Some(t) => {
                    self.0.update(|a| a.get_mut(s).token = Some(t));
                    let _ = self.0.tx.send(Job::Retry);
                    Poll::Done
                }
                None => Poll::Failed("invalid response".into()),
            },
            (Service::Trakt, 429) => {
                sign_in.interval_secs += 5;
                Poll::Pending
            }
            (Service::Trakt, 404) | (_, 410) => Poll::Failed("expired".into()),
            (Service::Trakt, 409 | 418) => Poll::Failed("denied".into()),
            (Service::Mal, 403) => Poll::Failed(resp.json()["error"].as_str().unwrap_or("denied").to_owned()),
            _ => Poll::Pending,
        }
    }

    pub fn show_ids(&self, client: &Client, srv: &Server, downloads: Option<&Downloads>, show: &str) -> Option<Ids> {
        let key = (srv.id.clone(), show.to_owned());
        if let Some(ids) = self.0.lock().shows.get(&key) {
            return Some(ids.clone());
        }
        let it = downloads.and_then(|d| d.item(&srv.id, show)).filter(|s| !s.guids.is_empty()).or_else(|| srv.metadata(&Ctx::new(client.clone(), Source::Network), show).ok())?;
        let ids = item_ids(&it);
        self.0.lock().shows.insert(key, ids.clone());
        Some(ids)
    }

    pub fn on_progress(&self, media: &Media, state: PlayState, position_ms: u64, duration_ms: u64) {
        if duration_ms == 0 {
            return;
        }
        let pct = (position_ms as f64 * 100.0 / duration_ms as f64).min(100.0);
        let mut out = Vec::new();
        {
            let mut guard = self.0.lock();
            let s = &mut *guard;
            let configured = self.0.configured;
            let active = |svc| s.accounts.active(svc, configured);
            let realtime = active(Service::Trakt) && s.accounts.trakt_progress;
            if s.session.as_ref().is_none_or(|x| x.media != *media) {
                if let Some(old) = s.session.take() {
                    out.extend(jobs(old.end().into_iter().collect(), &old.media, active, realtime));
                }
                s.session = Some(Session::new(media.clone()));
            }
            let actions = s.session.as_mut().map(|x| x.step(state, pct, s.threshold, realtime)).unwrap_or_default();
            out.extend(jobs(actions, media, |svc| s.accounts.active(svc, configured), realtime));
        }
        for job in out {
            if let Job::Write(w) = &job {
                self.0.queue().push(w.clone());
            }
            let _ = self.0.tx.send(job);
        }
    }
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn queue(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn drain(&self, mappings: &mut anime::Mappings) {
        while let Some(w) = self.queue().items.first().cloned() {
            if self.write(&w, mappings) == Outcome::Retry {
                break;
            }
            self.queue().remove(&w);
        }
    }

    fn update(&self, f: impl FnOnce(&mut Accounts)) {
        let mut s = self.lock();
        f(&mut s.accounts);
        store::write_json(&self.dir.join(ACCOUNTS_FILE), &s.accounts);
    }

    fn token(&self, s: Service) -> Option<Token> {
        self.configured[s as usize].then(|| self.lock().accounts.get(s).token.clone()).flatten()
    }

    fn replace_token(&self, s: Service, old: &Token, new: Option<Token>) {
        self.update(|a| {
            let slot = &mut a.get_mut(s).token;
            if slot.as_ref().is_some_and(|t| t.access == old.access) {
                *slot = new;
            }
        });
    }

    fn refresh(&self, s: Service, token: &Token) -> Result<Token, Outcome> {
        let req = match (s, &token.refresh) {
            (Service::Trakt, Some(r)) => trakt::refresh_request(trakt_id().unwrap_or_default(), r),
            (Service::Mal, Some(r)) => lists::refresh_request(relay(), s, r),
            _ => {
                log::warn!("{} sign-in expired", s.key());
                self.replace_token(s, token, None);
                return Err(Outcome::Drop);
            }
        };
        let resp = send(&req).map_err(|_| Outcome::Retry)?;
        match resp.status {
            200 => {
                let fresh = Token::parse(&resp.json(), now(), token.refresh.clone()).ok_or(Outcome::Retry)?;
                self.replace_token(s, token, Some(fresh.clone()));
                Ok(fresh)
            }
            400 | 401 => {
                log::warn!("{} sign-in was revoked", s.key());
                self.replace_token(s, token, None);
                Err(Outcome::Drop)
            }
            _ => Err(Outcome::Retry),
        }
    }

    fn call(&self, s: Service, build: impl Fn(&str) -> Request) -> Result<Response, Outcome> {
        let mut token = self.token(s).ok_or(Outcome::Drop)?;
        if token.stale(now()) {
            token = self.refresh(s, &token)?;
        }
        let failed = |e: String| {
            log::warn!("{} request failed: {e}", s.key());
            Outcome::Retry
        };
        let resp = send(&build(&token.access)).map_err(failed)?;
        if resp.status != 401 {
            return Ok(resp);
        }
        token = self.refresh(s, &token)?;
        send(&build(&token.access)).map_err(failed)
    }

    fn scrobble(&self, action: &str, media: &Media, pct: f64) -> Outcome {
        let (Some(id), Some(body)) = (trakt_id(), trakt::scrobble_body(media, pct)) else { return Outcome::Drop };
        let status = match self.call(Service::Trakt, |t| trakt::scrobble_request(id, t, action, body.clone())) {
            Ok(r) => r.status,
            Err(o) => return o,
        };
        log::info!("trakt scrobble {action} at {pct:.0}%: HTTP {status}");
        if status == 409 { Outcome::Done } else { classify(status) }
    }

    fn write(&self, w: &Write, mappings: &mut anime::Mappings) -> Outcome {
        let s = w.service;
        if s == Service::Trakt {
            return self.scrobble("stop", &w.media, w.pct.max(trakt::WATCHED_PCT));
        }
        if self.token(s).is_none() {
            return Outcome::Drop;
        }
        let m = match mappings.resolve(&w.media, now()) {
            Ok(Some(m)) => m,
            Ok(None) => return Outcome::Drop,
            Err(()) => return Outcome::Retry,
        };
        let Some(id) = m.mal else { return Outcome::Drop };
        let snapshot = match self.call(s, |t| lists::snapshot_request(id, t)) {
            Ok(r) if r.status == 200 => r.json(),
            Ok(r) => return classify(r.status),
            Err(o) => return o,
        };
        let Some(snap) = lists::parse_snapshot(&snapshot) else { return Outcome::Drop };
        let Some((progress, complete)) = lists::decide(&snap, m.progress, w.media.movie) else {
            log::info!("{} {id}: list already at {} episodes, nothing to write", s.key(), snap.progress);
            return Outcome::Done;
        };
        match self.call(s, |t| lists::update_request(id, t, progress, complete)) {
            Ok(r) => {
                log::info!("{} {id}: progress {progress}{}: HTTP {}", s.key(), if complete { " (completed)" } else { "" }, r.status);
                if r.json().get("errors").is_some() { Outcome::Drop } else { classify(r.status) }
            }
            Err(o) => o,
        }
    }
}

fn worker(inner: Weak<Inner>, rx: Receiver<Job>, mut mappings: anime::Mappings) {
    while let Ok(job) = rx.recv() {
        let Some(inner) = inner.upgrade() else { break };
        match job {
            Job::Scrobble(action, media, pct) => {
                inner.scrobble(action, &media, pct);
            }
            Job::Write(w) if inner.queue().items.contains(&w) => {
                let outcome = inner.write(&w, &mut mappings);
                if outcome != Outcome::Retry {
                    inner.queue().remove(&w);
                }
                if outcome == Outcome::Done {
                    inner.drain(&mut mappings);
                }
            }
            Job::Retry => inner.drain(&mut mappings),
            Job::Write(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Body;
    use serde_json::json;

    fn ids(pairs: &[(&str, &str)]) -> Ids {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn episode() -> Media {
        Media { movie: false, ids: ids(&[("tvdb", "999")]), show_ids: ids(&[("imdb", "tt0213338"), ("tvdb", "76885"), ("tmdb", "30991")]), season: Some(1), episode: Some(3), order: None }
    }

    #[test]
    fn ids_from_both_forms() {
        let plex = ids_from_guids(["imdb://tt0213338", "tmdb://30991", "tvdb://76885", "plex://show/5d9c", "local://1"]);
        assert_eq!(plex.get("tvdb").map(String::as_str), Some("76885"));
        assert_eq!(plex.get("imdb").map(String::as_str), Some("tt0213338"));
        let jelly = ids_from_guids(["Imdb://tt0213338", "tmdb://30991", "tvdb://76885", "tvrage://1", "AniList://1", "MyAnimeList://7", "tvdb://"]);
        assert_eq!(jelly, ids(&[("imdb", "tt0213338"), ("tmdb", "30991"), ("tvdb", "76885"), ("anilist", "1"), ("mal", "7")]));
    }

    #[test]
    fn media_for_movies_and_episodes() {
        use crate::plex::Guid;
        let guids = |ids: &[&str]| ids.iter().map(|id| Guid { id: (*id).into() }).collect();
        let movie = Item { kind: "movie".into(), guids: guids(&["imdb://tt1", "tmdb://2", "local://x", "myanimelist://7", "plex://movie/1", "kitsu://3"]), ..Default::default() };
        let m = Media::for_item(&movie).unwrap();
        assert!(m.movie && m.season.is_none() && m.order.is_none());
        assert_eq!(m.ids, ids(&[("imdb", "tt1"), ("tmdb", "2"), ("mal", "7")]));
        let episode = Item { kind: "episode".into(), parent_index: Some(2), index: Some(5), guids: guids(&["tvdb://9"]), ..Default::default() };
        let e = Media::for_item(&episode).unwrap();
        assert_eq!((e.movie, e.season, e.episode, e.ids.get("tvdb").map(String::as_str)), (false, Some(2), Some(5), Some("9")));
        assert_eq!(Media::for_item(&Item { kind: "episode".into(), index: Some(-1), ..Default::default() }).unwrap().episode, None);
        assert!(Media::for_item(&Item { kind: "clip".into(), ..Default::default() }).is_none());
        assert_eq!(["playing", "paused", "stopped", "buffering"].map(PlayState::parse), [Some(PlayState::Playing), Some(PlayState::Paused), Some(PlayState::Stopped), None]);
    }

    #[test]
    fn show_ids_are_cached_and_a_failed_fetch_is_not() {
        let dir = std::env::temp_dir().join(format!("plaivo-trackers-shows-{}", std::process::id()));
        let (t, _rx) = Trackers::detached(dir.clone(), [false; 2]);
        let srv = Server { id: "s".into(), uri: "http://127.0.0.1:9".into(), ..Default::default() };
        let client = Client::new("test".into());
        assert_eq!(t.show_ids(&client, &srv, None, "1"), None);
        assert!(t.0.lock().shows.is_empty());
        t.0.lock().shows.insert(("s".into(), "1".into()), ids(&[("tvdb", "9")]));
        assert_eq!(t.show_ids(&client, &srv, None, "1"), Some(ids(&[("tvdb", "9")])));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn watched_threshold_fires_once() {
        let mut s = Session::new(episode());
        assert!(s.step(PlayState::Playing, 95.0, 90.0, false).is_empty());
        let mut s = Session::new(episode());
        assert!(s.step(PlayState::Playing, 10.0, 90.0, false).is_empty());
        assert!(s.step(PlayState::Playing, 89.9, 90.0, false).is_empty());
        assert_eq!(s.step(PlayState::Playing, 90.0, 90.0, false), vec![Action::Watched(90.0)]);
        assert!(s.step(PlayState::Playing, 95.0, 90.0, false).is_empty());
        assert!(s.step(PlayState::Playing, 20.0, 90.0, false).is_empty());
        assert!(s.step(PlayState::Playing, 92.0, 90.0, false).is_empty());
        assert!(s.step(PlayState::Stopped, 99.0, 90.0, false).is_empty());
    }

    #[test]
    fn scrobble_transitions() {
        let mut s = Session::new(episode());
        let step = |s: &mut Session, st, pct| s.step(st, pct, 90.0, true);
        assert!(step(&mut s, PlayState::Paused, 1.0).is_empty());
        assert_eq!(step(&mut s, PlayState::Playing, 1.0), vec![Action::Trakt("start", 1.0)]);
        assert!(step(&mut s, PlayState::Playing, 2.0).is_empty());
        assert_eq!(step(&mut s, PlayState::Paused, 3.0), vec![Action::Trakt("pause", 3.0)]);
        assert!(step(&mut s, PlayState::Paused, 3.0).is_empty());
        assert_eq!(step(&mut s, PlayState::Playing, 3.0), vec![Action::Trakt("start", 3.0)]);
        assert_eq!(s.end(), Some(Action::Trakt("stop", 3.0)));
        assert_eq!(step(&mut s, PlayState::Playing, 91.0), vec![Action::Watched(91.0)]);
        assert_eq!(step(&mut s, PlayState::Stopped, 93.0), vec![Action::Trakt("stop", 93.0)]);
        assert!(step(&mut s, PlayState::Stopped, 93.0).is_empty());
        assert_eq!(s.end(), None);
    }

    #[test]
    fn actions_become_jobs() {
        let m = episode();
        let all = |_| true;
        let w = |service| Job::Write(Write { service, media: m.clone(), pct: 91.0 });
        assert_eq!(jobs(vec![Action::Watched(91.0)], &m, all, true), vec![w(Service::Mal)]);
        assert_eq!(jobs(vec![Action::Watched(91.0)], &m, all, false), vec![w(Service::Trakt), w(Service::Mal)]);
        assert_eq!(jobs(vec![Action::Watched(91.0)], &m, |s| s == Service::Mal, false), vec![w(Service::Mal)]);
        assert_eq!(jobs(vec![Action::Trakt("stop", 50.0)], &m, all, true), vec![Job::Scrobble("stop", m.clone(), 50.0)]);
        assert_eq!(jobs(vec![Action::Trakt("stop", 91.0)], &m, all, true), vec![w(Service::Trakt)]);
    }

    #[test]
    fn retry_queue_persists_and_is_bounded() {
        let dir = std::env::temp_dir().join(format!("plaivo-trackers-queue-{}", std::process::id()));
        let path = dir.join(QUEUE_FILE);
        let mut q = Queue::load(path.clone());
        for i in 0..QUEUE_MAX + 5 {
            q.push(Write { service: Service::Mal, media: episode(), pct: i as f64 });
        }
        let q2 = Queue::load(path.clone());
        assert_eq!(q2.items.len(), QUEUE_MAX);
        assert_eq!(q2.items[0].pct, 5.0);
        let mut q = q2;
        let first = q.items[0].clone();
        q.remove(&first);
        assert_eq!(Queue::load(path.clone()).items.len(), QUEUE_MAX - 1);
        assert_eq!(Queue::load(path).items[0].pct, 6.0);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn accounts_defaults_and_tokens() {
        let a: Accounts = serde_json::from_str("{}").unwrap();
        assert!(a.trakt_progress && a.mal.scrobble && a.mal.token.is_none());
        let t = Token::parse(&json!({ "access_token": "a", "expires_in": 1000 }), 100, Some("r".into())).unwrap();
        assert_eq!((t.access.as_str(), t.refresh.as_deref(), t.expires_at), ("a", Some("r"), 1100));
        assert!(!t.stale(700) && t.stale(800));
        assert!(Token::parse(&json!({ "error": "x" }), 0, None).is_none());
        assert!(!Token::parse(&json!({ "access_token": "a" }), 0, None).unwrap().stale(u64::MAX - 1000));
    }

    #[test]
    fn trakt_request_shapes() {
        let body = trakt::scrobble_body(&episode(), 12.345).unwrap();
        assert_eq!(body, json!({ "show": { "ids": { "imdb": "tt0213338", "tvdb": 76885, "tmdb": 30991 } }, "episode": { "season": 1, "number": 3 }, "progress": 12.35 }));
        let bare = Media { show_ids: Ids::new(), ..episode() };
        assert_eq!(trakt::scrobble_body(&bare, 50.0).unwrap(), json!({ "episode": { "ids": { "tvdb": 999 } }, "progress": 50.0 }));
        let movie = Media { movie: true, ids: ids(&[("tmdb", "329865"), ("imdb", "tt2543164")]), ..Media::default() };
        assert_eq!(trakt::scrobble_body(&movie, 100.0).unwrap(), json!({ "movie": { "ids": { "imdb": "tt2543164", "tmdb": 329865 } }, "progress": 100.0 }));
        assert!(trakt::scrobble_body(&Media { movie: true, ..Media::default() }, 1.0).is_none());

        let r = trakt::scrobble_request("cid", "tok", "pause", body.clone());
        assert_eq!((r.method, r.url.as_str()), ("POST", "https://api.trakt.tv/scrobble/pause"));
        assert_eq!(r.header_value("trakt-api-version"), Some("2"));
        assert_eq!(r.header_value("trakt-api-key"), Some("cid"));
        assert_eq!(r.header_value("authorization"), Some("Bearer tok"));
        assert_eq!(r.body, Body::Json(body));

        let r = trakt::device_code_request("cid");
        assert_eq!((r.url.as_str(), r.body), ("https://api.trakt.tv/oauth/device/code", Body::Json(json!({ "client_id": "cid" }))));
        let r = trakt::device_token_request("cid", "dev");
        assert_eq!((r.url.as_str(), r.body), ("https://api.trakt.tv/oauth/device/token", Body::Json(json!({ "code": "dev", "client_id": "cid" }))));
        let r = trakt::refresh_request("cid", "ref");
        assert_eq!((r.url.as_str(), r.body), ("https://api.trakt.tv/oauth/token", Body::Json(json!({ "refresh_token": "ref", "client_id": "cid", "redirect_uri": "urn:ietf:wg:oauth:2.0:oob", "grant_type": "refresh_token" }))));
    }

    #[test]
    fn relay_request_shapes() {
        let r = lists::start_request("https://relay.example/", Service::Mal);
        assert_eq!((r.method, r.url.as_str(), r.body), ("POST", "https://relay.example/auth/start", Body::Json(json!({ "service": "mal" }))));
        let r = lists::result_request("https://relay.example", "ab12");
        assert_eq!((r.method, r.url.as_str(), r.body), ("GET", "https://relay.example/auth/result?session=ab12", Body::Empty));
        let r = lists::refresh_request("https://relay.example", Service::Mal, "rt");
        assert_eq!((r.url.as_str(), r.body), ("https://relay.example/auth/refresh", Body::Json(json!({ "service": "mal", "refresh_token": "rt" }))));
    }

    #[test]
    fn list_request_shapes_and_decisions() {
        let r = lists::update_request(1, "tok", 5, false);
        assert_eq!((r.method, r.url.as_str()), ("PATCH", "https://api.myanimelist.net/v2/anime/1/my_list_status"));
        assert_eq!(r.body, Body::Form("status=watching&num_watched_episodes=5".into()));
        assert_eq!(r.header_value("authorization"), Some("Bearer tok"));
        assert_eq!(lists::update_request(1, "tok", 26, true).body, Body::Form("status=completed&num_watched_episodes=26".into()));
        assert_eq!(lists::snapshot_request(1, "t").url, "https://api.myanimelist.net/v2/anime/1?fields=num_episodes,my_list_status");

        let mal = lists::parse_snapshot(&json!({ "num_episodes": 26, "my_list_status": { "status": "watching", "num_episodes_watched": 4, "is_rewatching": false } })).unwrap();
        assert_eq!(mal, lists::Snapshot { total: 26, progress: 4, locked: false });
        let fresh = lists::parse_snapshot(&json!({ "id": 1, "num_episodes": 0 })).unwrap();
        assert_eq!(fresh, lists::Snapshot { total: 0, progress: 0, locked: false });
        let done = lists::parse_snapshot(&json!({ "num_episodes": 12, "my_list_status": { "status": "completed", "num_episodes_watched": 12 } })).unwrap();
        assert!(done.locked);
        assert!(lists::parse_snapshot(&json!({ "error": "not_found" })).is_none());

        assert_eq!(lists::decide(&mal, 5, false), Some((5, false)));
        assert_eq!(lists::decide(&mal, 4, false), None);
        assert_eq!(lists::decide(&mal, 2, false), None);
        assert_eq!(lists::decide(&mal, 26, false), Some((26, true)));
        assert_eq!(lists::decide(&mal, 30, false), None);
        assert_eq!(lists::decide(&fresh, 3, false), Some((3, false)));
        assert_eq!(lists::decide(&fresh, 1, true), Some((1, true)));
        assert_eq!(lists::decide(&done, 13, false), None);
        let at_end = lists::Snapshot { total: 12, progress: 12, locked: false };
        assert_eq!(lists::decide(&at_end, 12, false), Some((12, true)));
    }

    fn signed_in(t: &Trackers, services: &[Service]) {
        t.0.update(|a| {
            for &s in services {
                a.get_mut(s).token = Some(Token { access: "x".into(), refresh: None, expires_at: 0 });
            }
        });
    }

    #[test]
    fn on_progress_dispatch() {
        let dir = std::env::temp_dir().join(format!("plaivo-trackers-dispatch-{}", std::process::id()));
        let (t, rx) = Trackers::detached(dir.clone(), [true; 2]);
        let drain = || rx.try_iter().collect::<Vec<_>>();
        let a = episode();
        let b = Media { episode: Some(4), ..episode() };
        let w = |service, media: &Media, pct| Job::Write(Write { service, media: media.clone(), pct });
        let persisted = || Queue::load(dir.join(QUEUE_FILE)).items.into_iter().map(|w| (w.service, w.pct)).collect::<Vec<_>>();

        t.on_progress(&a, PlayState::Playing, 10, 100);
        assert!(drain().is_empty());

        signed_in(&t, &[Service::Trakt, Service::Mal]);
        t.on_progress(&a, PlayState::Playing, 20, 100);
        assert_eq!(drain(), vec![Job::Scrobble("start", a.clone(), 20.0)]);
        t.on_progress(&a, PlayState::Playing, 50, 100);
        assert!(drain().is_empty());
        t.on_progress(&b, PlayState::Playing, 1, 100);
        assert_eq!(drain(), vec![Job::Scrobble("stop", a.clone(), 50.0), Job::Scrobble("start", b.clone(), 1.0)]);
        t.on_progress(&b, PlayState::Playing, 95, 100);
        assert_eq!(drain(), vec![w(Service::Mal, &b, 95.0)]);
        t.on_progress(&b, PlayState::Stopped, 96, 100);
        assert_eq!(drain(), vec![w(Service::Trakt, &b, 96.0)]);
        assert_eq!(persisted(), [(Service::Mal, 95.0), (Service::Trakt, 96.0)]);

        t.set_trakt_progress(false);
        t.set_scrobble(Service::Mal, false);
        t.on_progress(&a, PlayState::Playing, 50, 100);
        t.on_progress(&a, PlayState::Paused, 60, 100);
        assert!(drain().is_empty());
        t.on_progress(&a, PlayState::Playing, 92, 100);
        assert_eq!(drain(), vec![w(Service::Trakt, &a, 92.0)]);

        let (u, rx) = Trackers::detached(dir.clone(), [false; 2]);
        assert!(!u.status(Service::Trakt).configured && !u.status(Service::Trakt).signed_in);
        u.on_progress(&b, PlayState::Playing, 1, 100);
        u.on_progress(&b, PlayState::Playing, 95, 100);
        assert!(rx.try_iter().next().is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
