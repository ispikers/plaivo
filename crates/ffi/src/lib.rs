use plaivo_core::downloads::{Downloads, State};
use plaivo_core::menu::{self, Action};
use plaivo_core::plex::{
    self, Client, Ctx, DISCOVER_ID, Hub, Item, Picks, PlexUser, QUALITIES, SORTS, Server, Source,
    fmt_bitrate, resolve_playable,
};
use plaivo_core::prefs::{self, LANGUAGES, LanguagePrefs, LanguageRule, SubtitleMode, TrackInfo};
use plaivo_core::settings::{
    AUTO_SKIP_NOTICE, BLUR_PARAM, CardPrefs, EpisodePoster, GridSpacing, HomeClock, PlaybackRule,
    SUBTITLE_COLORS, Scale, Settings, SkipMode, Theme, ViewMode, library_key, rank_in,
};
use plaivo_core::trackers::{self, Media, PlayState, Poll, Service, Status, Trackers};
use plaivo_core::{Account, detail, first, jellyfin, join, store};
use qrcodegen::{QrCode, QrCodeEcc};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

uniffi::setup_scaffolding!();

mod watch;

#[derive(Debug, uniffi::Error)]
#[uniffi(flat_error)]
pub enum PlaivoError {
    Failed(String),
}

impl std::fmt::Display for PlaivoError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let PlaivoError::Failed(msg) = self;
        f.write_str(msg)
    }
}

impl From<String> for PlaivoError {
    fn from(msg: String) -> Self {
        PlaivoError::Failed(msg)
    }
}

impl From<&str> for PlaivoError {
    fn from(msg: &str) -> Self {
        PlaivoError::Failed(msg.to_owned())
    }
}

type R<T> = Result<T, PlaivoError>;

type ItemKey = (String, String);

#[uniffi::remote(Enum)]
pub enum Scale {
    Fit,
    Fill,
    Stretch,
}

#[uniffi::remote(Record)]
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
    pub watch_name: String,
    pub watch_relay: String,
    pub watch_rooms: Vec<String>,
    pub check_updates: bool,
    pub home_clock: HomeClock,
}

#[uniffi::remote(Enum)]
pub enum SubtitleMode {
    Off,
    ForeignAudio,
    Always,
}

#[uniffi::remote(Record)]
pub struct LanguagePrefs {
    pub audio: String,
    pub subtitles: String,
    pub mode: SubtitleMode,
}

#[uniffi::remote(Record)]
pub struct TrackInfo {
    pub language: String,
    pub forced: bool,
    pub is_default: bool,
}

#[derive(uniffi::Record)]
pub struct TrackPick {
    pub audio: Option<u32>,
    pub text: Option<u32>,
}

#[derive(uniffi::Record)]
pub struct Language {
    pub code: String,
    pub name: String,
}

#[uniffi::remote(Enum)]
pub enum SkipMode {
    Off,
    Button,
    Auto,
}

#[uniffi::remote(Record)]
pub struct PlaybackRule {
    pub libraries: Vec<String>,
    pub shader: Option<String>,
    pub quality: Option<u32>,
}

#[uniffi::remote(Record)]
pub struct LanguageRule {
    pub libraries: Vec<String>,
    pub audio: Vec<String>,
    pub subtitles: Vec<String>,
    pub subtitle_mode: SubtitleMode,
}

#[uniffi::remote(Enum)]
pub enum Action {
    FromStart,
    MarkWatched,
    MarkUnwatched,
    ClearProgress,
    Trailer,
    Rate,
    Versions,
    FileInfo,
    Download,
    AddToWatchlist,
    RemoveFromWatchlist,
    Details,
}

#[derive(uniffi::Record)]
pub struct InfoLine {
    pub label: String,
    pub value: String,
}

#[derive(uniffi::Enum)]
pub enum RatingSource {
    Imdb,
    Critics,
    Audience,
    Tmdb,
    Stars,
}

impl From<plex::RatingSource> for RatingSource {
    fn from(s: plex::RatingSource) -> Self {
        match s {
            plex::RatingSource::Imdb => Self::Imdb,
            plex::RatingSource::Critics => Self::Critics,
            plex::RatingSource::Audience => Self::Audience,
            plex::RatingSource::Tmdb => Self::Tmdb,
            plex::RatingSource::Stars => Self::Stars,
        }
    }
}

#[derive(uniffi::Record)]
pub struct Rating {
    pub source: RatingSource,
    pub value: String,
}

#[derive(uniffi::Record)]
pub struct CastMember {
    pub id: String,
    pub name: String,
    pub role: String,
    pub image: Option<String>,
}

#[derive(uniffi::Record)]
pub struct Season {
    pub key: String,
    pub title: String,
    pub image: Option<String>,
}

#[derive(uniffi::Record)]
pub struct Detail {
    pub server: String,
    pub key: String,
    pub kind: String,
    pub title: String,
    pub eyebrow: String,
    pub logo: Option<String>,
    pub backdrop: Option<String>,
    pub chips: Vec<String>,
    pub ratings: Vec<Rating>,
    pub genres: Vec<String>,
    pub summary: String,
    pub play_key: String,
    pub play_label: String,
    pub progress: f32,
    pub media_line: String,
    pub info: Vec<InfoLine>,
    pub cast: Vec<CastMember>,
    pub extras: Vec<Card>,
    pub seasons: Vec<Season>,
    pub season: Option<String>,
    pub watched: bool,
    pub trailer_key: Option<String>,
    pub photo: Option<String>,
    pub rows: Vec<Row>,
}

#[derive(uniffi::Record)]
pub struct ItemMenu {
    pub actions: Vec<Action>,
    pub trailer_key: Option<String>,
    pub stars: u8,
    pub versions: Vec<String>,
    pub file_info: Vec<InfoLine>,
}

#[uniffi::export]
pub fn action_label(action: Action) -> String {
    action.label().into()
}

#[uniffi::remote(Enum)]
pub enum Theme {
    System,
    Light,
    Dark,
    Oled,
}

#[uniffi::remote(Enum)]
pub enum ViewMode {
    Grid,
    List,
}

#[uniffi::remote(Enum)]
pub enum GridSpacing {
    Tight,
    Normal,
    Spacious,
}

#[uniffi::remote(Enum)]
pub enum HomeClock {
    Never,
    Fullscreen,
    Always,
}

#[uniffi::remote(Enum)]
pub enum EpisodePoster {
    Series,
    Season,
    Thumbnail,
}

#[uniffi::remote(Record)]
pub struct PlexUser {
    pub name: String,
    pub thumb: Option<String>,
}

#[derive(uniffi::Record)]
pub struct SignIn {
    pub id: u64,
    pub code: String,
    pub url: String,
    pub qr_size: u32,
    pub qr: Vec<bool>,
    pub expires_in_s: u64,
}

#[uniffi::remote(Enum)]
pub enum Service {
    Trakt,
    Mal,
}

#[uniffi::remote(Record)]
pub struct Status {
    pub configured: bool,
    pub signed_in: bool,
    pub scrobble: bool,
}

#[derive(uniffi::Record)]
pub struct TrackerSignIn {
    pub url: String,
    pub user_code: Option<String>,
    pub qr_size: u32,
    pub qr: Vec<bool>,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
    pub session: Arc<TrackerSession>,
}

#[derive(uniffi::Object)]
pub struct TrackerSession {
    trackers: Trackers,
    sign_in: Mutex<trackers::SignIn>,
}

#[uniffi::export]
impl TrackerSession {
    pub fn poll(&self) -> TrackerPoll {
        let mut s = self.sign_in.lock().unwrap();
        match self.trackers.poll_sign_in(&mut s) {
            Poll::Pending => TrackerPoll::Pending {
                interval_secs: s.interval_secs,
            },
            Poll::Done => TrackerPoll::Done,
            Poll::Failed(reason) => TrackerPoll::Failed { reason },
        }
    }
}

#[derive(uniffi::Enum)]
pub enum TrackerPoll {
    Pending { interval_secs: u64 },
    Done,
    Failed { reason: String },
}

#[derive(uniffi::Record)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub owned: bool,
    pub jellyfin: bool,
}

#[derive(uniffi::Record)]
pub struct JellyfinProbe {
    pub uri: String,
    pub name: String,
    pub id: String,
    pub version: String,
}

#[derive(uniffi::Record)]
pub struct JellyfinServer {
    pub id: String,
    pub name: String,
    pub user: String,
    pub address: String,
}

#[derive(uniffi::Record)]
pub struct QuickConnectCode {
    pub code: String,
    pub secret: String,
}

impl From<JellyfinProbe> for jellyfin::ServerInfo {
    fn from(p: JellyfinProbe) -> Self {
        Self {
            uri: p.uri,
            name: p.name,
            id: p.id,
            version: p.version,
        }
    }
}

#[derive(uniffi::Record)]
pub struct Card {
    pub server: String,
    pub key: String,
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub image: Option<String>,
    pub backdrop: Option<String>,
    pub meta: String,
    pub summary: String,
    pub progress: f32,
    pub watched: bool,
    pub unwatched: u64,
    #[uniffi(default)]
    pub left: Option<String>,
}

#[derive(uniffi::Record)]
pub struct Section {
    pub id: String,
    pub server: String,
    pub server_name: String,
    pub key: String,
    pub title: String,
    pub movies: bool,
}

#[derive(uniffi::Record)]
pub struct LibraryPage {
    pub cards: Vec<Card>,
    pub total: u64,
}

#[derive(uniffi::Record)]
pub struct Row {
    pub key: String,
    pub title: String,
    pub resume: bool,
    pub wide: bool,
    pub cards: Vec<Card>,
}

#[derive(uniffi::Record)]
pub struct Home {
    pub rows: Vec<Row>,
    pub hero: Vec<Card>,
}

#[derive(uniffi::Enum)]
pub enum Listing {
    Items { sort: String },
    Collections,
    CollectionItems,
    Watchlist { sort: String, shows: bool },
}

#[derive(uniffi::Record)]
pub struct SortOption {
    pub id: String,
    pub label: String,
}

#[derive(uniffi::Record)]
pub struct HubSort {
    pub id: String,
    pub label: String,
    pub order: Vec<u32>,
}

#[derive(uniffi::Record)]
pub struct HubList {
    pub cards: Vec<Card>,
    pub sorts: Vec<HubSort>,
    pub initial: String,
}

#[derive(uniffi::Record)]
pub struct Subtitle {
    pub url: String,
    pub codec: String,
    pub title: String,
    pub language: String,
    pub forced: bool,
    pub stream: u32,
}

#[derive(uniffi::Record)]
pub struct SourceSubtitle {
    pub label: String,
    pub embedded: bool,
    pub burns: bool,
}

#[derive(uniffi::Record)]
pub struct TrackRequest {
    pub audio: Option<u32>,
    pub subtitle: Option<u32>,
}

#[derive(uniffi::Record)]
pub struct SubtitleColor {
    pub hex: String,
    pub name: String,
}

#[derive(uniffi::Record)]
pub struct QualityOption {
    pub kbps: u32,
    pub label: String,
    pub bitrate: String,
}

#[derive(uniffi::Record)]
pub struct UpNext {
    pub left_s: f64,
    pub anchor_ms: u64,
}

#[derive(uniffi::Record)]
pub struct SkipMarker {
    pub kind: String,
    pub mode: SkipMode,
    pub auto_at_ms: u64,
    pub notice_at_ms: u64,
    pub label: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(uniffi::Record)]
pub struct Playback {
    pub server: String,
    pub key: String,
    pub url: String,
    pub title: String,
    pub subtitle: String,
    pub start_ms: u64,
    pub duration_ms: u64,
    pub subtitles: Vec<Subtitle>,
    pub markers: Vec<SkipMarker>,
    pub show_key: Option<String>,
    pub file: String,
    pub container: String,
    pub bitrate_kbps: u64,
    pub source: String,
    pub transcode_session: Option<String>,
    pub transcode_label: String,
    pub audio_streams: Vec<String>,
    pub transcode_audio: Option<u32>,
    pub subtitle_streams: Vec<SourceSubtitle>,
    pub burned_subtitle: Option<u32>,
    pub subtitle_pick: Option<u32>,
    pub languages: LanguagePrefs,
    pub language_rule: Option<LanguageRule>,
    pub library: String,
    pub quality_kbps: u32,
}

#[uniffi::export(callback_interface)]
pub trait DownloadListener: Send + Sync {
    fn changed(&self);
}

#[derive(uniffi::Enum)]
pub enum DownloadState {
    Queued,
    Downloading,
    Paused,
    Failed,
    Done,
}

#[derive(uniffi::Enum)]
pub enum DownloadScope {
    Item,
    Season,
    Show,
}

#[derive(uniffi::Record)]
pub struct Download {
    pub server: String,
    pub key: String,
    pub kind: String,
    pub title: String,
    pub subtitle: String,
    pub state: DownloadState,
    pub bytes: u64,
    pub total: u64,
    pub error: String,
    pub watched: bool,
}

#[derive(uniffi::Object)]
pub struct Plaivo {
    client: Client,
    account: Mutex<Account>,
    plex_user: Mutex<Option<(String, PlexUser)>>,
    settings: Mutex<Settings>,
    downloads: OnceLock<Downloads>,
    trackers: Trackers,
    tracked: Mutex<HashMap<ItemKey, (Media, Option<String>)>>,
    watch: Mutex<Option<watch::Session>>,
    watch_listener: OnceLock<Arc<dyn watch::WatchListener>>,
    continuing: Mutex<HashMap<String, Hub>>,
}

impl Plaivo {
    fn live_servers(&self) -> Vec<Server> {
        let home = self.settings.lock().unwrap().home_server.clone();
        let mut servers = self.account.lock().unwrap().all_servers();
        servers.retain(|s| !plex::is_down(&s.id));
        servers.sort_by_key(|s| s.id != home);
        servers
    }

    fn card_prefs(&self) -> CardPrefs {
        self.settings.lock().unwrap().card_prefs()
    }
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn card(srv: &Server, it: &Item, wide: bool, prefs: &CardPrefs) -> Card {
    let blur = srv.id != DISCOVER_ID && prefs.blur(it, prefs.art(it, wide));
    card_with(&srv.id, it, wide, prefs, |path, w, h| {
        Some(srv.image_url(path, w, h) + if blur && w < 1920 { BLUR_PARAM } else { "" })
    })
}

fn offline_card(d: &Downloads, server: &str, it: &Item, wide: bool, prefs: &CardPrefs) -> Card {
    card_with(server, it, wide, prefs, |path, _, _| {
        d.image(server, path).map(|p| file_url(&p))
    })
}

fn card_with(
    server: &str,
    it: &Item,
    wide: bool,
    prefs: &CardPrefs,
    image: impl Fn(&str, u32, u32) -> Option<String>,
) -> Card {
    let (title, subtitle) = prefs.text(it);
    let art = prefs.art(it, wide);
    let (w, h) = if wide { (480, 270) } else { (300, 450) };
    let backdrop = first(&[&it.art, &it.grandparent_art]);
    let kind = match it.kind.as_str() {
        "movie" => "Movie",
        "show" => "TV Show",
        _ => "",
    };
    Card {
        server: server.to_owned(),
        key: it.rating_key.clone(),
        kind: it.kind.clone(),
        title,
        subtitle,
        image: (!art.is_empty()).then(|| image(art, w, h)).flatten(),
        backdrop: (!backdrop.is_empty())
            .then(|| image(backdrop, 1920, 1080))
            .flatten(),
        meta: join(&[
            kind.into(),
            it.audience_rating
                .map(|r| format!("★ {r:.1}"))
                .unwrap_or_default(),
            it.content_rating.clone(),
            it.year.map(|y| y.to_string()).unwrap_or_default(),
        ]),
        summary: if prefs.spoiler(it) {
            String::new()
        } else {
            it.summary.clone()
        },
        progress: it.progress(),
        watched: it.watched(),
        unwatched: it.unwatched(),
        left: it.time_left(),
    }
}

#[uniffi::export]
impl Plaivo {
    #[uniffi::constructor]
    pub fn new(config_dir: String, cache_dir: String) -> Arc<Self> {
        #[cfg(target_os = "android")]
        logcat::init();
        store::set_dirs(config_dir.into(), cache_dir.into());
        let account = Account::load();
        let settings = Settings::load();
        apply_log_level(&settings);
        std::thread::spawn(|| store::prune_cache(plaivo_core::DISK_CACHE_BYTES));
        let trackers = Trackers::start();
        trackers.set_watched_threshold(90.0);
        Arc::new(Self {
            client: Client::new(account.client_id.clone()),
            account: Mutex::new(account),
            plex_user: Mutex::new(None),
            settings: Mutex::new(settings),
            downloads: OnceLock::new(),
            trackers,
            tracked: Mutex::new(HashMap::new()),
            watch: Mutex::new(None),
            watch_listener: OnceLock::new(),
            continuing: Mutex::new(HashMap::new()),
        })
    }

    pub fn signed_in(&self) -> bool {
        self.account.lock().unwrap().token.is_some()
    }

    pub fn start_sign_in(&self) -> R<SignIn> {
        let pin = self.client.create_pin(false)?;
        let url = self.client.link_url(&pin.code);
        let (qr_size, qr) = qr(&url)?;
        Ok(SignIn {
            id: pin.id,
            code: pin.code,
            url,
            qr_size,
            qr,
            expires_in_s: pin.expires_in.unwrap_or(900),
        })
    }

    pub fn finish_sign_in(&self, id: u64) -> R<bool> {
        let Some(token) = self.client.check_pin(id)? else {
            return Ok(false);
        };
        let mut account = self.account.lock().unwrap();
        account.token = Some(token);
        account.save();
        Ok(true)
    }

    pub fn plex_user(&self) -> Option<PlexUser> {
        let token = self.account.lock().unwrap().token.clone()?;
        if let Some((cached, user)) = &*self.plex_user.lock().unwrap()
            && *cached == token
        {
            return Some(user.clone());
        }
        let user = self
            .client
            .user(&token)
            .inspect_err(|e| log::warn!("plex user: {e}"))
            .ok()?;
        *self.plex_user.lock().unwrap() = Some((token, user.clone()));
        Some(user)
    }

    pub fn sign_out(&self) {
        plaivo_core::sign_out(
            &mut self.account.lock().unwrap(),
            &mut self.settings.lock().unwrap(),
        );
        self.tracked.lock().unwrap().clear();
        self.watch_leave();
        self.sync_download_servers();
    }

    pub fn jellyfin_servers(&self) -> Vec<JellyfinServer> {
        let account = self.account.lock().unwrap();
        account
            .jellyfin
            .iter()
            .map(|s| JellyfinServer {
                id: s.id.clone(),
                name: s.name.clone(),
                user: s.user.clone(),
                address: s.uri.clone(),
            })
            .collect()
    }

    pub fn remove_jellyfin(&self, id: String) {
        let mut account = self.account.lock().unwrap();
        account.jellyfin.retain(|s| s.id != id);
        account.save();
        drop(account);
        self.sync_download_servers();
    }

    pub fn jellyfin_probe(&self, address: String) -> R<JellyfinProbe> {
        let i = jellyfin::probe(&self.client, &address)
            .map_err(|e| format!("Could not connect: {e}"))?;
        Ok(JellyfinProbe {
            uri: i.uri,
            name: i.name,
            id: i.id,
            version: i.version,
        })
    }

    pub fn jellyfin_sign_in(
        &self,
        server: JellyfinProbe,
        username: String,
        password: String,
    ) -> R<()> {
        let srv = jellyfin::sign_in(&self.client, &server.into(), username.trim(), &password)?;
        self.add_jellyfin(srv);
        Ok(())
    }

    pub fn jellyfin_quick_connect(&self, server: JellyfinProbe) -> R<QuickConnectCode> {
        let qc = jellyfin::quick_connect_start(&self.client, &server.into())?;
        Ok(QuickConnectCode {
            code: qc.code,
            secret: qc.secret,
        })
    }

    pub fn jellyfin_quick_connect_poll(&self, server: JellyfinProbe, secret: String) -> R<bool> {
        let info = server.into();
        if !jellyfin::quick_connect_ready(&self.client, &info, &secret)? {
            return Ok(false);
        }
        self.add_jellyfin(jellyfin::quick_connect_finish(
            &self.client,
            &info,
            &secret,
        )?);
        Ok(true)
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        settings.save();
        apply_log_level(&settings);
        let auto_remove = settings.auto_remove_watched;
        let changed = std::mem::replace(&mut *self.settings.lock().unwrap(), settings)
            .auto_remove_watched
            != auto_remove;
        if changed {
            self.with_downloads(|d| d.set_auto_remove_watched(auto_remove));
        }
    }

    pub fn arrange_libraries(
        &self,
        mut sections: Vec<Section>,
        order: Vec<String>,
    ) -> Vec<Section> {
        sections.sort_by_cached_key(|s| rank_in(&order, &s.id));
        sections
    }

    pub fn up_next_in(
        &self,
        time_ms: u64,
        duration_ms: u64,
        credits_ms: Option<u64>,
        anchor_ms: Option<u64>,
    ) -> Option<UpNext> {
        let s = |ms: u64| ms as f64 / 1000.0;
        let mut anchor = anchor_ms.map(s);
        let left_s = self.settings.lock().unwrap().up_next_in(
            s(time_ms),
            s(duration_ms),
            credits_ms.map(s),
            &mut anchor,
        )?;
        Some(UpNext {
            left_s,
            anchor_ms: anchor.map_or(time_ms, |a| (a * 1000.0) as u64),
        })
    }

    pub fn clear_cache(&self) {
        store::clear_cache();
    }

    pub fn migrate_android_prefs(
        &self,
        sidebar_compact: Option<bool>,
        home_server: Option<String>,
        quality: Option<i32>,
        scale: Option<String>,
    ) -> bool {
        let mut settings = self.settings.lock().unwrap();
        settings.migrate_android(sidebar_compact, home_server, quality, scale.as_deref());
        settings.save()
    }

    pub fn has_servers(&self) -> bool {
        self.account.lock().unwrap().has_servers()
    }

    pub fn servers(&self) -> Vec<ServerInfo> {
        let account = self.account.lock().unwrap();
        account
            .servers
            .iter()
            .chain(&account.jellyfin)
            .map(|s| ServerInfo {
                id: s.id.clone(),
                name: s.name.clone(),
                owned: s.owned,
                jellyfin: s.is_jellyfin(),
            })
            .collect()
    }

    pub fn refresh_servers(&self) -> R<Vec<ServerInfo>> {
        let Some(token) = self.account.lock().unwrap().token.clone() else {
            return Ok(self.servers());
        };
        let resources = self.client.servers(&token)?;
        let previous = self.account.lock().unwrap().servers.clone();
        let connected: Vec<Server> = std::thread::scope(|s| {
            let probes: Vec<_> = resources
                .iter()
                .map(|r| {
                    let last = previous
                        .iter()
                        .find(|p| p.id == r.client_identifier)
                        .map(|p| p.uri.as_str());
                    let token = token.as_str();
                    (r, s.spawn(move || self.client.connect(r, token, last)))
                })
                .collect();
            probes
                .into_iter()
                .filter_map(|(r, p)| {
                    let probed = p.join().unwrap_or_else(|_| Err("probe panicked".into()));
                    match &probed {
                        Ok(srv) => log::info!("discovery: {} reachable at {}", srv.name, srv.uri),
                        Err(e) => log::warn!("discovery: {e}, keeping the last known address"),
                    }
                    probed.ok().or_else(|| {
                        previous
                            .iter()
                            .find(|s| s.id == r.client_identifier)
                            .cloned()
                    })
                })
                .collect()
        });
        let mut account = self.account.lock().unwrap();
        account.servers = connected;
        account.save();
        drop(account);
        self.sync_download_servers();
        Ok(self.servers())
    }

    pub fn home(&self, server_id: String, cached: bool) -> R<Home> {
        let srv = self.server(&server_id)?;
        let ctx = Ctx::new(
            self.client.clone(),
            if cached {
                Source::Cache
            } else {
                Source::Network
            },
        );
        let mut hubs = srv.hubs(&ctx).inspect_err(|_| {
            if !cached && plex::is_down(&srv.id) {
                self.continuing.lock().unwrap().remove(&srv.id);
            }
        })?;
        let prefs = self.card_prefs();
        let hero = plex::pick_hero(&hubs)
            .into_iter()
            .map(|it| card(&srv, it, false, &prefs))
            .collect();
        let resume = hubs
            .iter()
            .position(Hub::is_continue)
            .map(|i| hubs.remove(i))
            .unwrap_or_default();
        self.continuing
            .lock()
            .unwrap()
            .insert(srv.id.clone(), resume);
        Ok(Home {
            hero,
            rows: rows(&srv, &hubs, &prefs),
        })
    }

    pub fn refresh_continue(&self, server_id: String, cached: bool) {
        let Ok(srv) = self.server(&server_id) else {
            return;
        };
        let source = if cached {
            Source::Cache
        } else {
            Source::Network
        };
        match srv.continue_items(&Ctx::new(self.client.clone(), source)) {
            Ok(metadata) => {
                let hub = Hub {
                    metadata,
                    ..Default::default()
                };
                self.continuing.lock().unwrap().insert(server_id, hub);
            }
            Err(_) if !cached && plex::is_down(&server_id) => {
                self.continuing.lock().unwrap().remove(&server_id);
            }
            Err(_) => {}
        }
    }

    pub fn continue_row(&self, home_id: String) -> Option<Row> {
        let servers = self.account.lock().unwrap().all_servers();
        let continuing = self.continuing.lock().unwrap();
        let none = Hub::default();
        let prefs = self.card_prefs();
        let row = plex::continue_row(
            &home_id,
            continuing.get(&home_id).unwrap_or(&none),
            servers
                .iter()
                .filter_map(|s| Some((s.id.as_str(), &continuing.get(&s.id)?.metadata[..]))),
            &prefs,
        )?;
        Some(Row {
            key: row.key.to_owned(),
            title: row.title.to_owned(),
            resume: true,
            wide: row.wide,
            cards: row
                .items
                .iter()
                .filter_map(|&(id, it)| {
                    let srv = servers.iter().find(|s| s.id == id)?;
                    Some(card(srv, it, row.wide, &prefs))
                })
                .collect(),
        })
    }

    pub fn library_hubs(
        &self,
        server_id: String,
        key: String,
        movies: bool,
        cached: bool,
    ) -> R<Vec<Row>> {
        let srv = self.server(&server_id)?;
        let ctx = Ctx::new(
            self.client.clone(),
            if cached {
                Source::Cache
            } else {
                Source::Network
            },
        );
        Ok(rows(
            &srv,
            &srv.section_hubs(&ctx, &key, if movies { "movie" } else { "show" })?,
            &self.card_prefs(),
        ))
    }

    pub fn hub_list(&self, server_id: String, key: String) -> R<HubList> {
        let srv = self.server(&server_id)?;
        let items = srv.hub_items(&Ctx::new(self.client.clone(), Source::Live), &key)?;
        let prefs = self.card_prefs();
        Ok(HubList {
            cards: items
                .iter()
                .map(|it| card(&srv, it, false, &prefs))
                .collect(),
            sorts: hub_sorts(&items, &key),
            initial: plex::initial_sort(&key),
        })
    }

    pub fn libraries(&self, cached: bool) -> Vec<Section> {
        let servers = self.account.lock().unwrap().all_servers();
        let ctx = Ctx::new(
            self.client.clone(),
            if cached {
                Source::Cache
            } else {
                Source::Network
            },
        );
        std::thread::scope(|s| {
            let jobs: Vec<_> = servers
                .iter()
                .map(|srv| (srv, s.spawn(|| srv.sections(&ctx))))
                .collect();
            jobs.into_iter()
                .filter_map(|(srv, job)| {
                    Some((
                        srv,
                        job.join()
                            .ok()?
                            .inspect_err(|e| log::warn!("sections for {}: {e}", srv.name))
                            .ok()?,
                    ))
                })
                .flat_map(|(srv, dirs)| {
                    dirs.into_iter().map(|d| Section {
                        id: library_key(&srv.id, &d.key),
                        server: srv.id.clone(),
                        server_name: srv.name.clone(),
                        movies: d.kind == "movie",
                        key: d.key,
                        title: d.title,
                    })
                })
                .collect()
        })
    }

    pub fn library_page(
        &self,
        server_id: String,
        key: String,
        listing: Listing,
        start: u64,
        size: u64,
    ) -> R<LibraryPage> {
        let srv = self.server(&server_id)?;
        let (prefs, list) = {
            let s = self.settings.lock().unwrap();
            (s.card_prefs(), s.view_mode == ViewMode::List)
        };
        let ctx = Ctx::new(self.client.clone(), Source::Live);
        let (start, size) = (start as usize, size as usize);
        let (items, total) = match listing {
            Listing::Items { sort } => srv.section_items(&ctx, &key, start, size, &sort, list)?,
            Listing::Collections => srv.collections(&ctx, &key, start, size)?,
            Listing::CollectionItems => srv.collection_items(&ctx, &key, start, size, list)?,
            Listing::Watchlist { sort, shows } => {
                srv.watchlist(&ctx, if shows { "2" } else { "1" }, start, size, &sort)?
            }
        };
        Ok(LibraryPage {
            cards: items
                .iter()
                .map(|it| card(&srv, it, false, &prefs))
                .collect(),
            total: total as u64,
        })
    }

    pub fn detail(&self, server_id: String, key: String, cached: bool) -> R<Detail> {
        let srv = self.server(&server_id)?;
        let ctx = Ctx::new(
            self.client.clone(),
            if cached {
                Source::Cache
            } else {
                Source::Network
            },
        );
        let mut it = srv.metadata(&ctx, &key)?;
        let mut wanted = None;
        if it.kind == "season" && !it.parent_rating_key.is_empty() {
            let show = srv.metadata(&ctx, &it.parent_rating_key)?;
            wanted = Some(std::mem::replace(&mut it, show).rating_key);
        }
        let seasons = if it.kind == "show" {
            srv.children(&ctx, &it.rating_key)?
        } else {
            Vec::new()
        };
        let summary = detail::summary(&it, server_id != DISCOVER_ID);
        let prefs = self.card_prefs();
        let image =
            |path: &str, w: u32, h: u32| (!path.is_empty()).then(|| srv.image_url(path, w, h));
        Ok(Detail {
            server: server_id,
            key: it.rating_key.clone(),
            kind: it.kind.clone(),
            title: it.title.clone(),
            eyebrow: summary.eyebrow,
            logo: image(it.logo(), 880, 300),
            backdrop: image(first(&[&it.art, &it.grandparent_art]), 1920, 1080),
            chips: summary.chips,
            ratings: it
                .rating_labels()
                .into_iter()
                .map(|(source, value)| Rating {
                    source: source.into(),
                    value,
                })
                .collect(),
            genres: it.genre.iter().take(4).map(|t| t.tag.clone()).collect(),
            summary: it.summary.clone(),
            play_key: summary.play_key,
            play_label: summary.play_label,
            progress: summary.progress,
            media_line: if it.kind == "show" {
                String::new()
            } else {
                it.media_line(0)
            },
            info: summary
                .info
                .into_iter()
                .map(|(label, value)| InfoLine {
                    label: label.into(),
                    value,
                })
                .collect(),
            cast: it
                .role
                .iter()
                .take(40)
                .map(|p| CastMember {
                    id: p.id.clone(),
                    name: p.tag.clone(),
                    role: p.role.clone(),
                    image: image(&p.thumb, 300, 300),
                })
                .collect(),
            extras: it
                .extras()
                .iter()
                .map(|x| Card {
                    subtitle: x.duration.map(plex::fmt_duration).unwrap_or_default(),
                    ..card(&srv, x, true, &prefs)
                })
                .collect(),
            season: detail::pick_season(&it, &seasons, wanted.as_deref()),
            seasons: seasons
                .iter()
                .map(|x| Season {
                    key: x.rating_key.clone(),
                    title: x.title.clone(),
                    image: image(&x.thumb, 300, 450),
                })
                .collect(),
            watched: it.watched(),
            trailer_key: it.trailer().map(|t| t.rating_key.clone()),
            photo: None,
            rows: Vec::new(),
        })
    }

    pub fn person(&self, server_id: String, id: String, name: String, cached: bool) -> R<Detail> {
        let srv = self.server(&server_id)?;
        let ctx = Ctx::new(
            self.client.clone(),
            if cached {
                Source::Cache
            } else {
                Source::Network
            },
        );
        let discover = self.server(DISCOVER_ID).ok();
        let servers = self.live_servers();
        let matched: &[Server] = if server_id == DISCOVER_ID {
            &servers
        } else {
            &[]
        };
        let (info, items) = plex::person_page(&srv, &ctx, &id, &name, discover.as_ref(), matched)?;
        let prefs = self.card_prefs();
        let mut keys = HashSet::new();
        let library: Vec<Card> = items
            .iter()
            .filter(|(s, it)| keys.insert((s.clone(), it.rating_key.clone())))
            .filter_map(|(s, it)| {
                let owner = servers.iter().chain([&srv]).find(|x| x.id == *s)?;
                Some(card(owner, it, false, &prefs))
            })
            .collect();
        let mut keys = HashSet::new();
        let known_for: Vec<Card> = match &discover {
            Some(d) => info
                .known_for
                .iter()
                .filter(|it| keys.insert(it.rating_key.clone()))
                .map(|it| card(d, it, false, &prefs))
                .collect(),
            None => Vec::new(),
        };
        let backdrop = library
            .iter()
            .chain(&known_for)
            .find_map(|c| c.backdrop.clone());
        let row = |title: &str, cards: Vec<Card>| Row {
            key: String::new(),
            title: title.into(),
            resume: false,
            wide: false,
            cards,
        };
        Ok(Detail {
            server: server_id,
            key: id,
            kind: "person".into(),
            title: name,
            eyebrow: String::new(),
            logo: None,
            backdrop,
            chips: Vec::new(),
            ratings: Vec::new(),
            genres: Vec::new(),
            summary: info.summary,
            play_key: String::new(),
            play_label: String::new(),
            progress: 0.,
            media_line: String::new(),
            info: [
                ("Born", info.born),
                ("Died", info.died),
                ("Birthplace", info.birthplace),
            ]
            .into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(label, value)| InfoLine {
                label: label.into(),
                value,
            })
            .collect(),
            cast: Vec::new(),
            extras: Vec::new(),
            seasons: Vec::new(),
            season: None,
            watched: false,
            trailer_key: None,
            photo: (!info.thumb.is_empty()).then(|| srv.image_url(&info.thumb, 400, 600)),
            rows: [
                row("Media in Library", library),
                row("Known For", known_for),
            ]
            .into_iter()
            .filter(|r| !r.cards.is_empty())
            .collect(),
        })
    }

    pub fn episodes(&self, server_id: String, season_key: String, cached: bool) -> R<Vec<Card>> {
        let srv = self.server(&server_id)?;
        let ctx = Ctx::new(
            self.client.clone(),
            if cached {
                Source::Cache
            } else {
                Source::Network
            },
        );
        let prefs = self.card_prefs();
        Ok(srv
            .children(&ctx, &season_key)?
            .iter()
            .map(|e| Card {
                title: format!("{}. {}", e.index.unwrap_or(0), e.title),
                meta: join(&[
                    e.duration.map(plex::fmt_duration).unwrap_or_default(),
                    e.originally_available_at.clone(),
                ]),
                ..card(&srv, e, true, &prefs)
            })
            .collect())
    }

    pub fn related(&self, server_id: String, key: String) -> R<Vec<Row>> {
        if server_id == DISCOVER_ID {
            return Ok(Vec::new());
        }
        let srv = self.server(&server_id)?;
        let hubs = srv.related(&Ctx::new(self.client.clone(), Source::Network), &key)?;
        Ok(rows(&srv, &hubs[..hubs.len().min(3)], &self.card_prefs()))
    }

    pub fn item_menu(&self, server_id: String, key: String) -> R<ItemMenu> {
        let srv = self.server(&server_id)?;
        let it = srv.metadata(&Ctx::new(self.client.clone(), Source::Network), &key)?;
        let token = self.account.lock().unwrap().token.clone();
        let watchlisted = match (&token, it.plex_guid()) {
            (Some(t), Some(guid)) if matches!(it.kind.as_str(), "movie" | "show") => {
                self.client.watchlisted(t, guid).ok()
            }
            _ => None,
        };
        let ctx = menu::Context {
            local: server_id != DISCOVER_ID,
            details: false,
            can_rate: srv.can_rate(),
            can_download: self.downloads.get().is_some(),
            signed_in: token.is_some(),
            watchlisted,
        };
        Ok(ItemMenu {
            actions: menu::actions(&it, &ctx),
            trailer_key: it.trailer().map(|t| t.rating_key.clone()),
            stars: menu::stars(&it),
            versions: it.media.iter().map(menu::version_label).collect(),
            file_info: menu::file_info(&it)
                .into_iter()
                .map(|(label, value)| InfoLine { label, value })
                .collect(),
        })
    }

    pub fn menu_action(&self, server_id: String, key: String, action: Action) -> R<String> {
        let srv = self.server(&server_id)?;
        Ok(match action {
            Action::MarkWatched | Action::MarkUnwatched => {
                let on = action == Action::MarkWatched;
                srv.set_watched(&self.client, &key, on)?;
                if on {
                    "Marked as watched"
                } else {
                    "Marked as unwatched"
                }
                .into()
            }
            Action::ClearProgress => {
                srv.remove_from_continue_watching(&self.client, &key)?;
                "Removed from Continue Watching".into()
            }
            Action::AddToWatchlist | Action::RemoveFromWatchlist => {
                let on = action == Action::AddToWatchlist;
                let token = self
                    .account
                    .lock()
                    .unwrap()
                    .token
                    .clone()
                    .ok_or("Sign in to Plex to use the watchlist")?;
                let it = srv.metadata(&Ctx::new(self.client.clone(), Source::Network), &key)?;
                self.client.set_watchlisted(
                    &token,
                    it.plex_guid().ok_or("This title has no Plex ID")?,
                    on,
                )?;
                if on {
                    "Added to watchlist"
                } else {
                    "Removed from watchlist"
                }
                .into()
            }
            _ => {
                return Err(PlaivoError::Failed(
                    "This action opens on the device".into(),
                ));
            }
        })
    }

    pub fn rate(&self, server_id: String, key: String, stars: Option<u8>) -> R<()> {
        Ok(self.server(&server_id)?.rate(&self.client, &key, stars)?)
    }

    pub fn server_offline(&self, server_id: String) -> bool {
        plaivo_core::plex::is_down(&server_id)
    }

    pub fn search(&self, query: String) -> Vec<Card> {
        let mut servers = self.account.lock().unwrap().all_servers();
        servers.retain(|s| !plaivo_core::plex::is_down(&s.id));
        let ctx = Ctx::new(self.client.clone(), Source::Live);
        let prefs = self.card_prefs();
        let mut hits: Vec<(f64, Card)> = std::thread::scope(|s| {
            let jobs: Vec<_> = servers
                .iter()
                .map(|srv| (srv, s.spawn(|| srv.search(&ctx, &query))))
                .collect();
            jobs.into_iter()
                .filter_map(|(srv, job)| {
                    Some((
                        srv,
                        job.join()
                            .ok()?
                            .inspect_err(|e| log::warn!("search on {}: {e}", srv.name))
                            .ok()?,
                    ))
                })
                .flat_map(|(srv, found)| {
                    found
                        .into_iter()
                        .map(|(it, score)| (score, card(srv, &it, false, &prefs)))
                        .collect::<Vec<_>>()
                })
                .collect()
        });
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.into_iter().map(|(_, c)| c).collect()
    }

    pub fn qualities(&self) -> Vec<QualityOption> {
        QUALITIES
            .iter()
            .map(|q| QualityOption {
                kbps: q.kbps,
                label: q.label.into(),
                bitrate: fmt_bitrate(q.kbps as u64),
            })
            .collect()
    }

    pub fn playback(
        &self,
        server_id: String,
        key: String,
        force_transcode: bool,
        version: u32,
        audio_codecs: String,
        tracks: Option<TrackRequest>,
    ) -> R<Playback> {
        if !force_transcode
            && version == 0
            && let Some(p) = self.local_playback(&server_id, &key)
        {
            return Ok(p);
        }
        let srv = self.server(&server_id)?;
        let mut it = resolve_playable(&srv, &Ctx::new(self.client.clone(), Source::Live), &key)?;
        if (version as usize) < it.media.len() {
            it.media.swap(0, version as usize);
        }
        let part = it
            .media
            .first()
            .and_then(|m| m.part.first())
            .ok_or("This item has no playable media")?;
        let media = &it.media[0];
        let quality_kbps = self
            .settings
            .lock()
            .unwrap()
            .quality_for(&library_key(&srv.id, &it.library_section_id));
        let local = self.settings.lock().unwrap().languages();
        let token = self.account.lock().unwrap().token.clone();
        let languages = prefs::for_server(&self.client, &srv, token.as_deref(), &local);
        let rules = self.settings.lock().unwrap().language_rules.clone();
        let rule = prefs::language_rule(&srv.id, &it, &rules);
        let picks = match tracks {
            Some(t) => Picks {
                audio: t.audio.map(|i| i as usize),
                subtitle: t.subtitle.map(|i| i as usize),
            },
            None => prefs::pick_source(media, &languages, rule.as_ref()),
        };
        let (url, transcode) = srv.stream_url(
            &self.client,
            &it.rating_key,
            media,
            quality_kbps,
            force_transcode,
            &audio_codecs,
            picks,
        );
        let transcoding = transcode.is_some();
        let subtitles = media
            .subtitle_streams()
            .enumerate()
            .filter_map(|(i, st)| {
                let (key, codec) = st.subtitle_file(transcoding)?;
                Some(Subtitle {
                    url: srv.media_url(key),
                    codec: codec.to_owned(),
                    title: st.display_title.clone(),
                    language: st.language_code.clone(),
                    forced: st.forced,
                    stream: i as u32,
                })
            })
            .collect();
        let subtitle_streams = media
            .subtitle_streams()
            .enumerate()
            .map(|(i, st)| SourceSubtitle {
                label: st.track_label(i),
                embedded: st.key.is_empty(),
                burns: st.subtitle_file(true).is_none(),
            })
            .collect();
        let transcode_label = transcode
            .as_ref()
            .map(|t| {
                format!(
                    "{} · {}",
                    t.quality.label,
                    fmt_bitrate(t.quality.kbps as u64)
                )
            })
            .unwrap_or_default();
        let file = part
            .file
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned();
        let as_u32 = |i: Option<usize>| i.map(|i| i as u32);
        Ok(Playback {
            transcode_audio: as_u32(transcode.as_ref().and_then(|t| t.audio)),
            burned_subtitle: as_u32(transcode.as_ref().and_then(|t| t.burn)),
            subtitle_pick: as_u32(picks.subtitle),
            subtitle_streams,
            ..self.playback_for(
                &srv.id,
                &it,
                url,
                file,
                subtitles,
                transcode.map(|t| t.session),
                transcode_label,
                languages,
                rule,
                quality_kbps,
            )
        })
    }

    pub fn account_languages(&self) -> R<Option<LanguagePrefs>> {
        let account = self.account.lock().unwrap();
        let (token, jellyfin) = (account.token.clone(), account.jellyfin.clone());
        drop(account);
        Ok(prefs::account(&self.client, token.as_deref(), &jellyfin)?)
    }

    pub fn set_account_languages(&self, languages: LanguagePrefs) -> R<()> {
        let account = self.account.lock().unwrap();
        let (token, jellyfin) = (account.token.clone(), account.jellyfin.clone());
        drop(account);
        Ok(prefs::save(
            &self.client,
            token.as_deref(),
            &jellyfin,
            &languages,
            prefs::next_generation(),
        )?)
    }

    pub fn queue(&self, server_id: String, show_key: String) -> R<Vec<Card>> {
        let prefs = self.card_prefs();
        let online = self.server(&server_id).and_then(|srv| {
            Ok(srv
                .all_episodes(&Ctx::new(self.client.clone(), Source::Live), &show_key)?
                .iter()
                .map(|it| card(&srv, it, true, &prefs))
                .collect())
        });
        let Some(d) = self.downloads.get().filter(|_| online.is_err()) else {
            return online;
        };
        let mut episodes: Vec<Item> = d
            .library()
            .into_iter()
            .filter(|(s, it)| *s == server_id && it.grandparent_rating_key == show_key)
            .map(|(_, it)| it)
            .collect();
        episodes.sort_by_key(|it| (it.parent_index, it.index));
        Ok(episodes
            .iter()
            .map(|it| offline_card(d, &server_id, it, true, &prefs))
            .collect())
    }

    pub fn stop_transcode(&self, server_id: String, session: String) {
        if let Ok(srv) = self.server(&server_id) {
            srv.stop_transcode(&self.client, &session);
        }
    }

    pub fn report(
        &self,
        server_id: String,
        key: String,
        state: String,
        time_ms: u64,
        duration_ms: u64,
        transcode_session: Option<String>,
    ) {
        if let Some(state) = PlayState::parse(&state) {
            self.scrobble(&server_id, &key, state, time_ms, duration_ms);
        }
        let synced = self.server(&server_id).is_ok_and(|srv| {
            srv.timeline(
                &self.client,
                &key,
                &state,
                time_ms,
                duration_ms,
                transcode_session.as_deref(),
            )
        });
        if let Some(d) = self.downloads.get()
            && d.local(&server_id, &key).is_some()
        {
            d.record_progress(&server_id, &key, &state, time_ms, duration_ms, synced);
        }
    }

    pub fn tracker_status(&self, service: Service) -> Status {
        self.trackers.status(service)
    }

    pub fn set_tracker_scrobble(&self, service: Service, on: bool) {
        self.trackers.set_scrobble(service, on);
    }

    pub fn trakt_progress(&self) -> bool {
        self.trackers.trakt_progress()
    }

    pub fn set_trakt_progress(&self, on: bool) {
        self.trackers.set_trakt_progress(on);
    }

    pub fn tracker_sign_out(&self, service: Service) {
        self.trackers.sign_out(service);
    }

    pub fn begin_tracker_sign_in(&self, service: Service) -> R<TrackerSignIn> {
        let s = self.trackers.begin_sign_in(service)?;
        let (qr_size, qr) = qr(&s.url)?;
        Ok(TrackerSignIn {
            url: s.url.clone(),
            user_code: s.user_code.clone(),
            qr_size,
            qr,
            interval_secs: s.interval_secs,
            expires_in_secs: s.expires_in_secs,
            session: Arc::new(TrackerSession {
                trackers: self.trackers.clone(),
                sign_in: Mutex::new(s),
            }),
        })
    }

    pub fn start_downloads(&self, root: String, listener: Box<dyn DownloadListener>) {
        if self.downloads.get().is_some() {
            return;
        }
        if self
            .downloads
            .set(Downloads::new(
                root.into(),
                self.client.clone(),
                move || listener.changed(),
            ))
            .is_err()
        {
            return;
        }
        self.sync_download_servers();
        let auto_remove = self.settings.lock().unwrap().auto_remove_watched;
        self.with_downloads(|d| d.set_auto_remove_watched(auto_remove));
    }

    pub fn downloads(&self) -> Vec<Download> {
        let Some(d) = self.downloads.get() else {
            return Vec::new();
        };
        d.snapshot()
            .into_iter()
            .map(|e| Download {
                state: match e.state {
                    State::Queued => DownloadState::Queued,
                    State::Downloading => DownloadState::Downloading,
                    State::Paused => DownloadState::Paused,
                    State::Failed => DownloadState::Failed,
                    State::Done => DownloadState::Done,
                },
                server: e.server,
                key: e.key,
                kind: e.kind,
                title: e.title,
                subtitle: e.subtitle,
                bytes: e.bytes,
                total: e.total,
                error: e.error,
                watched: e.watched,
            })
            .collect()
    }

    pub fn download(&self, server_id: String, key: String, scope: DownloadScope) -> R<()> {
        let d = self.downloads.get().ok_or("Downloads are not ready yet")?;
        let srv = self.server(&server_id)?;
        let cx = Ctx::new(self.client.clone(), Source::Live);
        let it = srv.metadata(&cx, &key)?;
        let it = match download_parent(&scope, &it) {
            None => it,
            Some(k) if k.is_empty() => {
                return Err("This item has no season or show to download".into());
            }
            Some(k) => srv.metadata(&cx, k)?,
        };
        d.enqueue(&srv, &it);
        Ok(())
    }

    pub fn pause_download(&self, server_id: String, key: String) {
        self.with_downloads(|d| d.pause(&server_id, &key));
    }

    pub fn resume_download(&self, server_id: String, key: String) {
        self.with_downloads(|d| d.resume(&server_id, &key));
    }

    pub fn remove_download(&self, server_id: String, key: String) {
        self.with_downloads(|d| d.remove(&server_id, &key));
    }

    pub fn pause_all_downloads(&self) {
        self.with_downloads(Downloads::pause_all);
    }

    pub fn resume_all_downloads(&self) {
        self.with_downloads(Downloads::resume_all);
    }

    pub fn find_owned(&self, server_id: String, key: String) -> R<Option<Card>> {
        if server_id != DISCOVER_ID {
            return Ok(None);
        }
        let discover = self.server(DISCOVER_ID)?;
        let it = discover
            .metadata(&Ctx::new(self.client.clone(), Source::Cache), &key)
            .or_else(|_| {
                discover.metadata(&Ctx::new(self.client.clone(), Source::Network), &key)
            })?;
        let servers = self.live_servers();
        let prefs = self.card_prefs();
        Ok(
            plaivo_core::watch_together::find_owned(&it, &servers, &self.client).and_then(
                |(id, local)| {
                    let srv = servers.iter().find(|s| s.id == id)?;
                    Some(card(srv, &local, false, &prefs))
                },
            ),
        )
    }

    pub fn offline_library(&self) -> Vec<Card> {
        let Some(d) = self.downloads.get() else {
            return Vec::new();
        };
        let mut items = d.library();
        items.sort_by_cached_key(|(_, it)| {
            (it.card_text().0.to_lowercase(), it.parent_index, it.index)
        });
        let prefs = self.card_prefs();
        items
            .iter()
            .map(|(server, it)| offline_card(d, server, it, false, &prefs))
            .collect()
    }
}

#[uniffi::export]
pub fn library_sorts() -> Vec<SortOption> {
    SORTS
        .iter()
        .map(|(id, label)| SortOption {
            id: (*id).into(),
            label: (*label).into(),
        })
        .collect()
}

#[uniffi::export]
pub fn grid_columns(width: f32, card: f32, gap: f32) -> i32 {
    plaivo_core::grid_columns(width, card, gap)
}

#[uniffi::export]
pub fn discover_id() -> String {
    DISCOVER_ID.into()
}

#[uniffi::export]
pub fn set_playback_quality(mut settings: Settings, library: String, kbps: u32) -> Settings {
    settings.set_quality_for(&library, kbps);
    settings
}

#[uniffi::export]
pub fn move_library(
    mut settings: Settings,
    arranged: Vec<String>,
    id: String,
    step: i32,
) -> Settings {
    settings.move_library(&arranged, &id, step as isize);
    settings
}

#[uniffi::export]
pub fn pick_tracks(
    audio: Vec<TrackInfo>,
    text: Vec<TrackInfo>,
    languages: LanguagePrefs,
    rule: Option<LanguageRule>,
) -> TrackPick {
    let (a, t) = prefs::pick_with(&audio, &text, &languages, rule.as_ref());
    TrackPick {
        audio: a.map(|i| i as u32),
        text: t.map(|i| i as u32),
    }
}

#[uniffi::export]
pub fn languages() -> Vec<Language> {
    LANGUAGES
        .iter()
        .map(|(name, codes)| Language {
            code: codes[0].into(),
            name: (*name).into(),
        })
        .collect()
}

#[uniffi::export]
pub fn language_name(code: String) -> String {
    prefs::language_name(&code)
}

#[uniffi::export]
pub fn subtitle_mode_label(mode: SubtitleMode) -> String {
    mode.label().into()
}

#[uniffi::export]
pub fn subtitle_colors() -> Vec<SubtitleColor> {
    SUBTITLE_COLORS
        .iter()
        .map(|(hex, name)| SubtitleColor {
            hex: (*hex).into(),
            name: (*name).into(),
        })
        .collect()
}

#[uniffi::export]
pub fn card_scale(density: u32) -> f32 {
    Settings {
        density,
        ..Default::default()
    }
    .card_scale()
}

fn hub_sorts(items: &[Item], key: &str) -> Vec<HubSort> {
    plex::hub_sorts(items, key)
        .into_iter()
        .map(|(id, label)| {
            let mut sorted: Vec<&Item> = items.iter().collect();
            plex::sort_items(&mut sorted, id);
            let order = sorted
                .iter()
                .map(|it| {
                    items
                        .iter()
                        .position(|x| std::ptr::eq(x, *it))
                        .unwrap_or_default() as u32
                })
                .collect();
            HubSort {
                id: id.into(),
                label: label.into(),
                order,
            }
        })
        .collect()
}

// Kotlin's lazy lists key rows by title and cards by key, and crash on duplicates.
fn rows(srv: &Server, hubs: &[Hub], prefs: &CardPrefs) -> Vec<Row> {
    let mut titles = HashSet::new();
    hubs.iter()
        .filter(|h| titles.insert(h.title.as_str()))
        .map(|h| {
            let wide = prefs.wide_hub(h);
            let mut keys = HashSet::new();
            Row {
                key: h.key.clone(),
                title: h.title.clone(),
                resume: h.is_continue(),
                wide,
                cards: h
                    .metadata
                    .iter()
                    .filter(|it| keys.insert(it.rating_key.as_str()))
                    .map(|it| card(srv, it, wide, prefs))
                    .collect(),
            }
        })
        .collect()
}

fn download_parent<'a>(scope: &DownloadScope, it: &'a Item) -> Option<&'a String> {
    match (scope, it.kind.as_str()) {
        (DownloadScope::Item, _)
        | (DownloadScope::Season, "season")
        | (DownloadScope::Show, "show") => None,
        (DownloadScope::Season, _) | (DownloadScope::Show, "season") => Some(&it.parent_rating_key),
        (DownloadScope::Show, _) => Some(&it.grandparent_rating_key),
    }
}

fn qr(text: &str) -> R<(u32, Vec<bool>)> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|e| format!("QR code: {e:?}"))?;
    let size = qr.size();
    Ok((
        size as u32,
        (0..size)
            .flat_map(|y| (0..size).map(move |x| (x, y)))
            .map(|(x, y)| qr.get_module(x, y))
            .collect(),
    ))
}

fn apply_log_level(settings: &Settings) {
    log::set_max_level(if settings.debug_logging {
        log::LevelFilter::Debug
    } else {
        log::LevelFilter::Info
    });
}

impl Plaivo {
    fn add_jellyfin(&self, srv: Server) {
        log::info!("jellyfin: signed in to {} as {}", srv.name, srv.user);
        let mut account = self.account.lock().unwrap();
        let (c, token, jellyfin, added) = (
            self.client.clone(),
            account.token.clone(),
            account.jellyfin.clone(),
            srv.clone(),
        );
        account.add_jellyfin(srv);
        account.save();
        drop(account);
        self.sync_download_servers();
        std::thread::spawn(move || {
            if let Err(e) = prefs::share_with_new_server(&c, token.as_deref(), &jellyfin, &added) {
                log::warn!("language preferences to {}: {e}", added.name);
            }
        });
    }

    fn scrobble(&self, server: &str, key: &str, state: PlayState, time_ms: u64, duration_ms: u64) {
        let id = (server.to_owned(), key.to_owned());
        let active = Service::ALL
            .into_iter()
            .map(|s| self.trackers.status(s))
            .any(|s| s.signed_in && s.scrobble);
        let media = if active {
            self.tracked_media(&id)
        } else {
            None
        };
        if state == PlayState::Stopped {
            self.tracked.lock().unwrap().remove(&id);
        }
        if let Some(media) = media {
            self.trackers
                .on_progress(&media, state, time_ms, duration_ms);
        }
    }

    fn tracked_media(&self, id: &ItemKey) -> Option<Media> {
        let (mut media, show) = self.tracked.lock().unwrap().get(id)?.clone();
        if let Some(show) = show {
            media.show_ids = self
                .server(&id.0)
                .ok()
                .and_then(|srv| {
                    self.trackers
                        .show_ids(&self.client, &srv, self.downloads.get(), &show)
                })
                .unwrap_or_default();
            if let Some(entry) = self.tracked.lock().unwrap().get_mut(id) {
                *entry = (media.clone(), None);
            }
        }
        Some(media)
    }

    fn sync_download_servers(&self) {
        let servers = self.account.lock().unwrap().all_servers();
        self.with_downloads(|d| d.set_servers(&servers));
    }

    fn with_downloads(&self, f: impl FnOnce(&Downloads)) {
        if let Some(d) = self.downloads.get() {
            f(d);
        }
    }

    fn local_playback(&self, server: &str, key: &str) -> Option<Playback> {
        let d = self.downloads.get()?;
        let local = d.local(server, key)?;
        let it = d.item(server, key)?;
        let subtitles = local
            .subtitles
            .iter()
            .enumerate()
            .map(|(i, (path, st))| Subtitle {
                url: file_url(path),
                codec: st.codec.clone(),
                title: st.display_title.clone(),
                language: st.language_code.clone(),
                forced: st.forced,
                stream: i as u32,
            })
            .collect();
        let languages = self.settings.lock().unwrap().languages();
        let file = local
            .file
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        log::info!("playing {key} from the downloaded file");
        let (rules, quality_kbps) = {
            let s = self.settings.lock().unwrap();
            let library = library_key(server, &it.library_section_id);
            (s.language_rules.clone(), s.quality_for(&library))
        };
        let rule = prefs::language_rule(server, &it, &rules);
        Some(self.playback_for(
            server,
            &it,
            file_url(&local.file),
            file,
            subtitles,
            None,
            String::new(),
            languages,
            rule,
            quality_kbps,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn playback_for(
        &self,
        server: &str,
        it: &Item,
        url: String,
        file: String,
        subtitles: Vec<Subtitle>,
        transcode_session: Option<String>,
        transcode_label: String,
        languages: LanguagePrefs,
        language_rule: Option<LanguageRule>,
        quality_kbps: u32,
    ) -> Playback {
        let (title, subtitle) = it.card_text();
        let start_ms = it.resume_ms().unwrap_or(0);
        let settings = self.settings.lock().unwrap();
        let markers = it
            .marker
            .iter()
            .filter_map(|m| {
                let at = settings.auto_skip_at(&m.kind, m.start_time_offset as f64 / 1000.0);
                Some(SkipMarker {
                    kind: m.kind.clone(),
                    mode: settings.skip_mode(&m.kind),
                    auto_at_ms: (at * 1000.0) as u64,
                    notice_at_ms: ((at - AUTO_SKIP_NOTICE).max(0.0) * 1000.0) as u64,
                    label: m.skip_label()?.into(),
                    start_ms: m.start_time_offset,
                    end_ms: m.end_time_offset,
                })
            })
            .collect();
        drop(settings);
        if let Some(m) = Media::for_item(it) {
            let show = (!m.movie && !it.grandparent_rating_key.is_empty())
                .then(|| it.grandparent_rating_key.clone());
            self.tracked
                .lock()
                .unwrap()
                .insert((server.to_owned(), it.rating_key.clone()), (m, show));
        }
        let media = it.media.first();
        let resolution = match media
            .map(|m| m.video_resolution.to_ascii_lowercase())
            .unwrap_or_default()
            .as_str()
        {
            "" => String::new(),
            "4k" => "4K".into(),
            "sd" => "SD".into(),
            r if r.chars().all(|c| c.is_ascii_digit()) => format!("{r}p"),
            r => r.to_uppercase(),
        };
        Playback {
            url,
            server: server.to_owned(),
            key: it.rating_key.clone(),
            title,
            subtitle,
            start_ms,
            duration_ms: it.duration.unwrap_or(0),
            subtitles,
            markers,
            show_key: (it.kind == "episode" && !it.grandparent_rating_key.is_empty())
                .then(|| it.grandparent_rating_key.clone()),
            file,
            container: media.map(|m| m.container.clone()).unwrap_or_default(),
            bitrate_kbps: media.and_then(|m| m.bitrate).unwrap_or(0),
            source: join(&[
                media
                    .and_then(|m| m.bitrate)
                    .map(fmt_bitrate)
                    .unwrap_or_default(),
                resolution,
            ]),
            transcode_session,
            transcode_label,
            audio_streams: media
                .map(|m| {
                    m.audio_streams()
                        .enumerate()
                        .map(|(i, s)| s.track_label(i))
                        .collect()
                })
                .unwrap_or_default(),
            transcode_audio: None,
            subtitle_streams: Vec::new(),
            burned_subtitle: None,
            subtitle_pick: None,
            languages,
            language_rule,
            library: library_key(server, &it.library_section_id),
            quality_kbps,
        }
    }

    fn server(&self, id: &str) -> R<Server> {
        let account = self.account.lock().unwrap();
        if id == DISCOVER_ID {
            return Ok(self.client.discover(
                account
                    .token
                    .as_deref()
                    .ok_or("Sign in to Plex to use the watchlist")?,
            ));
        }
        account
            .server(id)
            .cloned()
            .ok_or_else(|| "Unknown server".into())
    }
}

#[cfg(target_os = "android")]
mod logcat {
    use log::{Level, LevelFilter, Log, Metadata, Record};
    use std::ffi::{CString, c_char, c_int};

    #[link(name = "log")]
    unsafe extern "C" {
        fn __android_log_write(prio: c_int, tag: *const c_char, text: *const c_char) -> c_int;
    }

    struct Logcat;

    impl Log for Logcat {
        fn enabled(&self, m: &Metadata) -> bool {
            m.level() <= log::max_level()
        }

        fn log(&self, r: &Record) {
            let prio = match r.level() {
                Level::Error => 6,
                Level::Warn => 5,
                Level::Info => 4,
                Level::Debug => 3,
                Level::Trace => 2,
            };
            if let Ok(text) = CString::new(plaivo_core::redact(&r.args().to_string())) {
                unsafe { __android_log_write(prio, c"Plaivo".as_ptr(), text.as_ptr()) };
            }
        }

        fn flush(&self) {}
    }

    pub fn init() {
        if log::set_logger(&Logcat).is_ok() {
            log::set_max_level(LevelFilter::Info);
        }
    }
}

mod licenses {
    include!(concat!(env!("OUT_DIR"), "/licenses.rs"));
}

#[derive(uniffi::Record)]
pub struct RustLicense {
    pub name: String,
    pub version: String,
    pub license: String,
}

#[uniffi::export]
pub fn rust_licenses() -> Vec<RustLicense> {
    licenses::LICENSES
        .iter()
        .map(|l| RustLicense {
            name: l.name.into(),
            version: l.version.into(),
            license: l.license.into(),
        })
        .collect()
}

#[uniffi::export]
pub fn rust_license_text(index: u32) -> String {
    licenses::LICENSES
        .get(index as usize)
        .map(|l| l.parts().collect::<Vec<_>>().join("\n\n"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn rows_drop_duplicate_titles_and_cards() {
        use super::{Hub, Item, Server, Settings, rows};
        let item = |key: &str| Item {
            rating_key: key.into(),
            ..Default::default()
        };
        let hub = |title: &str, keys: &[&str]| Hub {
            title: title.into(),
            metadata: keys.iter().map(|k| item(k)).collect(),
            ..Default::default()
        };
        let hubs = [
            hub("A", &["1", "2", "1"]),
            hub("B", &["1"]),
            hub("A", &["3"]),
        ];
        let out = rows(&Server::default(), &hubs, &Settings::default().card_prefs());
        let shape: Vec<_> = out
            .iter()
            .map(|r| {
                (
                    r.title.as_str(),
                    r.cards.iter().map(|c| c.key.as_str()).collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(shape, [("A", vec!["1", "2"]), ("B", vec!["1"])]);
    }

    #[test]
    fn download_scope_picks_the_season_or_show() {
        use super::{DownloadScope as S, Item, download_parent};
        let episode = Item {
            kind: "episode".into(),
            parent_rating_key: "s".into(),
            grandparent_rating_key: "g".into(),
            ..Default::default()
        };
        let season = Item {
            kind: "season".into(),
            parent_rating_key: "g".into(),
            ..Default::default()
        };
        assert_eq!(download_parent(&S::Item, &episode), None);
        assert_eq!(
            download_parent(&S::Season, &episode).map(String::as_str),
            Some("s")
        );
        assert_eq!(
            download_parent(&S::Show, &episode).map(String::as_str),
            Some("g")
        );
        assert_eq!(download_parent(&S::Season, &season), None);
        assert_eq!(
            download_parent(&S::Show, &season).map(String::as_str),
            Some("g")
        );
    }

    #[test]
    fn show_ids_fall_back_once_then_track_until_stopped_and_sign_out_forgets() {
        use super::{Media, Plaivo, PlayState};
        let dir = std::env::temp_dir().join(format!("plaivo-ffi-shows-{}", std::process::id()));
        let core = Plaivo::new(
            dir.join("config").display().to_string(),
            dir.join("cache").display().to_string(),
        );
        let id = ("nowhere".to_owned(), "1".to_owned());
        let episode = Media {
            season: Some(1),
            episode: Some(2),
            ..Default::default()
        };
        core.tracked
            .lock()
            .unwrap()
            .insert(id.clone(), (episode.clone(), Some("show".into())));
        assert_eq!(core.tracked_media(&id), Some(episode.clone()));
        assert_eq!(core.tracked.lock().unwrap().get(&id).unwrap().1, None);
        core.scrobble("nowhere", "1", PlayState::Stopped, 0, 1000);
        assert!(core.tracked.lock().unwrap().is_empty());
        core.tracked
            .lock()
            .unwrap()
            .insert(id.clone(), (episode, None));
        core.account.lock().unwrap().token = Some("t".into());
        core.settings.lock().unwrap().home_server = "p1".into();
        let sam = super::PlexUser {
            name: "Sam".into(),
            thumb: None,
        };
        *core.plex_user.lock().unwrap() = Some(("t".into(), sam.clone()));
        assert_eq!(core.plex_user(), Some(sam));
        core.sign_out();
        assert!(core.plex_user().is_none());
        assert!(
            !core.signed_in()
                && core.settings().home_server.is_empty()
                && core.tracked.lock().unwrap().is_empty()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn discover_needs_a_plex_sign_in() {
        use super::{DISCOVER_ID, Plaivo};
        let dir = std::env::temp_dir().join(format!("plaivo-ffi-discover-{}", std::process::id()));
        let core = Plaivo::new(
            dir.join("config").display().to_string(),
            dir.join("cache").display().to_string(),
        );
        assert!(core.server(DISCOVER_ID).is_err());
        core.account.lock().unwrap().token = Some("t".into());
        let discover = core.server(DISCOVER_ID).unwrap();
        assert_eq!(
            (discover.id.as_str(), discover.token.as_str()),
            (DISCOVER_ID, "t")
        );
        assert!(core.find_owned("p1".into(), "1".into()).unwrap().is_none());
        assert!(
            core.related(DISCOVER_ID.into(), "1".into())
                .unwrap()
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn hub_sorts_give_card_orders() {
        use super::{Item, hub_sorts};
        let items = [
            Item {
                title: "b".into(),
                ..Default::default()
            },
            Item {
                title: "a".into(),
                added_at: Some(1),
                ..Default::default()
            },
        ];
        let sorts = hub_sorts(&items, "/hubs/x");
        let order = |id: &str| sorts.iter().find(|s| s.id == id).map(|s| s.order.clone());
        assert_eq!(order(""), Some(vec![0, 1]));
        assert_eq!(order("titleSort"), Some(vec![1, 0]));
        assert_eq!(order("addedAt:desc"), Some(vec![1, 0]));
        assert!(
            hub_sorts(&items[..1], "/hubs/x")
                .iter()
                .all(|s| s.id != "addedAt:desc")
        );
    }

    #[test]
    fn license_list_matches_cargo_tree() {
        super::licenses::assert_matches_cargo_tree("plaivo-ffi");
    }
}
