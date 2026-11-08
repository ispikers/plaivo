use plaivo_core::downloads::{Downloads, State};
use plaivo_core::plex::{Client, Ctx, Item, QUALITIES, Quality, Server, Source, fmt_bitrate, resolve_playable};
use plaivo_core::prefs::{self, LANGUAGES, LanguagePrefs, SubtitleMode, TrackInfo};
use plaivo_core::settings::{SUBTITLE_COLORS, Scale, Settings, SkipMode, library_key};
use plaivo_core::trackers::{self, Ids, Media, PlayState, Poll, Service, Status, Trackers, ids_from_guids};
use plaivo_core::{Account, first, jellyfin, join, store};
use qrcodegen::{QrCode, QrCodeEcc};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

uniffi::setup_scaffolding!();

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

#[derive(uniffi::Record)]
pub struct SignIn {
    pub id: u64,
    pub code: String,
    pub qr_size: u32,
    pub qr: Vec<bool>,
    pub expires_in_s: u64,
}

#[uniffi::remote(Enum)]
pub enum Service {
    Trakt,
    Mal,
    Anilist,
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
        Self { uri: p.uri, name: p.name, id: p.id, version: p.version }
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
    pub title: String,
    pub resume: bool,
    pub wide: bool,
    pub cards: Vec<Card>,
}

#[derive(uniffi::Record)]
pub struct Subtitle {
    pub url: String,
    pub codec: String,
    pub title: String,
    pub language: String,
    pub forced: bool,
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
    pub languages: LanguagePrefs,
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
    settings: Mutex<Settings>,
    downloads: OnceLock<Downloads>,
    trackers: Trackers,
    tracker_sign_in: Mutex<Option<trackers::SignIn>>,
    tracked: Mutex<HashMap<ItemKey, (Media, Option<String>)>>,
    show_ids: Mutex<HashMap<ItemKey, Ids>>,
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn card(srv: &Server, it: &Item, wide: bool) -> Card {
    card_with(&srv.id, it, wide, |path, w, h| Some(srv.image_url(path, w, h)))
}

fn offline_card(d: &Downloads, server: &str, it: &Item, wide: bool) -> Card {
    card_with(server, it, wide, |path, _, _| d.image(server, path).map(|p| file_url(&p)))
}

fn card_with(server: &str, it: &Item, wide: bool, image: impl Fn(&str, u32, u32) -> Option<String>) -> Card {
    let (title, subtitle) = it.card_text();
    let art = it.card_art(wide);
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
        backdrop: (!backdrop.is_empty()).then(|| image(backdrop, 1920, 1080)).flatten(),
        meta: join(&[
            kind.into(),
            it.audience_rating.map(|r| format!("★ {r:.1}")).unwrap_or_default(),
            it.content_rating.clone(),
            it.year.map(|y| y.to_string()).unwrap_or_default(),
        ]),
        summary: it.summary.clone(),
        progress: it.progress(),
        watched: it.watched(),
        unwatched: it.unwatched(),
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
        let trackers = Trackers::start();
        trackers.set_watched_threshold(90.0);
        Arc::new(Self {
            client: Client::new(account.client_id.clone()),
            account: Mutex::new(account),
            settings: Mutex::new(settings),
            downloads: OnceLock::new(),
            trackers,
            tracker_sign_in: Mutex::new(None),
            tracked: Mutex::new(HashMap::new()),
            show_ids: Mutex::new(HashMap::new()),
        })
    }

    pub fn signed_in(&self) -> bool {
        self.account.lock().unwrap().token.is_some()
    }

    pub fn start_sign_in(&self) -> R<SignIn> {
        let pin = self.client.create_pin(false)?;
        let (qr_size, qr) = qr(&self.client.link_url(&pin.code))?;
        Ok(SignIn { id: pin.id, code: pin.code, qr_size, qr, expires_in_s: pin.expires_in.unwrap_or(900) })
    }

    pub fn finish_sign_in(&self, id: u64) -> R<bool> {
        let Some(token) = self.client.check_pin(id)? else { return Ok(false) };
        let mut account = self.account.lock().unwrap();
        account.token = Some(token);
        account.save();
        Ok(true)
    }

    pub fn sign_out(&self) {
        let mut account = self.account.lock().unwrap();
        account.token = None;
        account.servers.clear();
        account.save();
        prefs::forget();
        let mut settings = self.settings.lock().unwrap();
        settings.home_server.clear();
        settings.recent_searches.clear();
        settings.save();
    }

    pub fn jellyfin_servers(&self) -> Vec<JellyfinServer> {
        let account = self.account.lock().unwrap();
        account.jellyfin.iter().map(|s| JellyfinServer { id: s.id.clone(), name: s.name.clone(), user: s.user.clone(), address: s.uri.clone() }).collect()
    }

    pub fn remove_jellyfin(&self, id: String) {
        let mut account = self.account.lock().unwrap();
        account.jellyfin.retain(|s| s.id != id);
        account.save();
    }

    pub fn jellyfin_probe(&self, address: String) -> R<JellyfinProbe> {
        let i = jellyfin::probe(&self.client, &address).map_err(|e| format!("Could not connect: {e}"))?;
        Ok(JellyfinProbe { uri: i.uri, name: i.name, id: i.id, version: i.version })
    }

    pub fn jellyfin_sign_in(&self, server: JellyfinProbe, username: String, password: String) -> R<()> {
        let srv = jellyfin::sign_in(&self.client, &server.into(), username.trim(), &password)?;
        self.add_jellyfin(srv);
        Ok(())
    }

    pub fn jellyfin_quick_connect(&self, server: JellyfinProbe) -> R<QuickConnectCode> {
        let qc = jellyfin::quick_connect_start(&self.client, &server.into())?;
        Ok(QuickConnectCode { code: qc.code, secret: qc.secret })
    }

    pub fn jellyfin_quick_connect_poll(&self, server: JellyfinProbe, secret: String) -> R<bool> {
        let info = server.into();
        if !jellyfin::quick_connect_ready(&self.client, &info, &secret)? {
            return Ok(false);
        }
        self.add_jellyfin(jellyfin::quick_connect_finish(&self.client, &info, &secret)?);
        Ok(true)
    }

    pub fn settings(&self) -> Settings {
        self.settings.lock().unwrap().clone()
    }

    pub fn set_settings(&self, settings: Settings) {
        settings.save();
        apply_log_level(&settings);
        let auto_remove = settings.auto_remove_watched;
        let changed = std::mem::replace(&mut *self.settings.lock().unwrap(), settings).auto_remove_watched != auto_remove;
        if changed {
            self.with_downloads(|d| d.set_auto_remove_watched(auto_remove));
        }
    }

    pub fn arrange_libraries(&self, mut sections: Vec<Section>) -> Vec<Section> {
        let settings = self.settings.lock().unwrap();
        sections.sort_by_cached_key(|s| settings.library_rank(&s.id));
        sections
    }

    pub fn up_next_in(&self, time_ms: u64, duration_ms: u64, credits_ms: Option<u64>, anchor_ms: Option<u64>) -> Option<UpNext> {
        let s = |ms: u64| ms as f64 / 1000.0;
        let mut anchor = anchor_ms.map(s);
        let left_s = self.settings.lock().unwrap().up_next_in(s(time_ms), s(duration_ms), credits_ms.map(s), &mut anchor)?;
        Some(UpNext { left_s, anchor_ms: anchor.map_or(time_ms, |a| (a * 1000.0) as u64) })
    }

    pub fn clear_cache(&self) {
        store::clear_cache();
    }

    pub fn migrate_android_prefs(&self, sidebar_compact: Option<bool>, home_server: Option<String>, quality: Option<i32>, scale: Option<String>) -> bool {
        let mut settings = self.settings.lock().unwrap();
        settings.migrate_android(sidebar_compact, home_server, quality, scale.as_deref());
        settings.save()
    }

    pub fn has_servers(&self) -> bool {
        self.account.lock().unwrap().has_servers()
    }

    pub fn servers(&self) -> Vec<ServerInfo> {
        let account = self.account.lock().unwrap();
        account.servers.iter().chain(&account.jellyfin).map(|s| ServerInfo { id: s.id.clone(), name: s.name.clone(), owned: s.owned, jellyfin: s.is_jellyfin() }).collect()
    }

    pub fn refresh_servers(&self) -> R<Vec<ServerInfo>> {
        let Some(token) = self.account.lock().unwrap().token.clone() else { return Ok(self.servers()) };
        let resources = self.client.servers(&token)?;
        let connected: Vec<Server> = std::thread::scope(|s| {
            let probes: Vec<_> = resources.iter().map(|r| s.spawn(|| self.client.connect(r, &token))).collect();
            probes.into_iter().filter_map(|p| p.join().ok()?.inspect_err(|e| log::warn!("{e}")).ok()).collect()
        });
        let mut account = self.account.lock().unwrap();
        account.servers = connected;
        account.save();
        drop(account);
        self.sync_download_servers();
        Ok(self.servers())
    }

    pub fn home(&self, server_id: String, cached: bool) -> R<Vec<Row>> {
        let srv = self.server(&server_id)?;
        let ctx = Ctx::new(self.client.clone(), if cached { Source::Cache } else { Source::Network });
        let hubs = srv.hubs(&ctx)?;
        Ok(hubs
            .iter()
            .map(|h| {
                let wide = h.wide();
                Row { title: h.title.clone(), resume: h.is_continue(), wide, cards: h.metadata.iter().map(|it| card(&srv, it, wide)).collect() }
            })
            .collect())
    }

    pub fn libraries(&self, cached: bool) -> Vec<Section> {
        let servers = self.account.lock().unwrap().all_servers();
        let ctx = Ctx::new(self.client.clone(), if cached { Source::Cache } else { Source::Network });
        std::thread::scope(|s| {
            let jobs: Vec<_> = servers.iter().map(|srv| (srv, s.spawn(|| srv.sections(&ctx)))).collect();
            jobs.into_iter()
                .filter_map(|(srv, job)| Some((srv, job.join().ok()?.inspect_err(|e| log::warn!("sections for {}: {e}", srv.name)).ok()?)))
                .flat_map(|(srv, dirs)| {
                    dirs.into_iter().map(|d| Section { id: library_key(&srv.id, &d.key), server: srv.id.clone(), server_name: srv.name.clone(), movies: d.kind == "movie", key: d.key, title: d.title })
                })
                .collect()
        })
    }

    pub fn library_page(&self, server_id: String, key: String, start: u64, size: u64) -> R<LibraryPage> {
        let srv = self.server(&server_id)?;
        let (items, total) = srv.section_items(&Ctx::new(self.client.clone(), Source::Network), &key, start as usize, size as usize, "titleSort")?;
        Ok(LibraryPage { cards: items.iter().map(|it| card(&srv, it, false)).collect(), total: total as u64 })
    }

    pub fn search(&self, query: String) -> Vec<Card> {
        let servers = self.account.lock().unwrap().all_servers();
        let ctx = Ctx::new(self.client.clone(), Source::Live);
        let mut hits: Vec<(f64, Card)> = std::thread::scope(|s| {
            let jobs: Vec<_> = servers.iter().map(|srv| (srv, s.spawn(|| srv.search(&ctx, &query)))).collect();
            jobs.into_iter()
                .filter_map(|(srv, job)| Some((srv, job.join().ok()?.inspect_err(|e| log::warn!("search on {}: {e}", srv.name)).ok()?)))
                .flat_map(|(srv, found)| found.into_iter().map(|(it, score)| (score, card(srv, &it, false))).collect::<Vec<_>>())
                .collect()
        });
        hits.sort_by(|a, b| b.0.total_cmp(&a.0));
        hits.into_iter().map(|(_, c)| c).collect()
    }

    pub fn qualities(&self) -> Vec<QualityOption> {
        QUALITIES.iter().map(|q| QualityOption { kbps: q.kbps, label: q.label.into(), bitrate: fmt_bitrate(q.kbps as u64) }).collect()
    }

    pub fn playback(&self, server_id: String, key: String, quality_kbps: u32, force_transcode: bool) -> R<Playback> {
        if !force_transcode && let Some(p) = self.local_playback(&server_id, &key) {
            return Ok(p);
        }
        let srv = self.server(&server_id)?;
        let it = resolve_playable(&srv, &Ctx::new(self.client.clone(), Source::Live), &key)?;
        let part = it.media.first().and_then(|m| m.part.first()).ok_or("This item has no playable media")?;
        let media = &it.media[0];
        let transcode = if force_transcode { Some(Quality::find(quality_kbps).unwrap_or(&QUALITIES[0])) } else { Quality::find(quality_kbps).filter(|q| !q.covers(media)) };
        let session = transcode.map(|_| {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            format!("plaivo-{nanos:x}")
        });
        let url = match (transcode, &session) {
            (Some(q), Some(s)) => {
                log::info!("transcoding {} at {} kbps", it.rating_key, q.kbps);
                srv.transcode_url(&self.client, &it.rating_key, media, q, s)
            }
            _ => srv.media_url(&part.key),
        };
        let subtitles = part
            .stream
            .iter()
            .filter(|st| st.stream_type == 3 && !st.key.is_empty())
            .map(|st| Subtitle { url: srv.media_url(&st.key), codec: st.codec.clone(), title: st.display_title.clone(), language: st.language_code.clone(), forced: st.forced })
            .collect();
        let local = self.settings.lock().unwrap().languages();
        let token = self.account.lock().unwrap().token.clone();
        let languages = prefs::for_server(&self.client, &srv, token.as_deref(), &local);
        let transcode_label = transcode.map(|q| format!("{} · {}", q.label, fmt_bitrate(q.kbps as u64))).unwrap_or_default();
        let file = part.file.rsplit(['/', '\\']).next().unwrap_or_default().to_owned();
        Ok(self.playback_for(&srv.id, &it, url, file, subtitles, session, transcode_label, languages))
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
        Ok(prefs::save(&self.client, token.as_deref(), &jellyfin, &languages, prefs::next_generation())?)
    }

    pub fn queue(&self, server_id: String, show_key: String) -> R<Vec<Card>> {
        let online = self.server(&server_id).and_then(|srv| Ok(srv.all_episodes(&Ctx::new(self.client.clone(), Source::Network), &show_key)?.iter().map(|it| card(&srv, it, true)).collect()));
        let Some(d) = self.downloads.get().filter(|_| online.is_err()) else { return online };
        let mut episodes: Vec<Item> = d.library().into_iter().filter(|(s, it)| *s == server_id && it.grandparent_rating_key == show_key).map(|(_, it)| it).collect();
        episodes.sort_by_key(|it| (it.parent_index, it.index));
        Ok(episodes.iter().map(|it| offline_card(d, &server_id, it, true)).collect())
    }

    pub fn stop_transcode(&self, server_id: String, session: String) {
        if let Ok(srv) = self.server(&server_id) {
            srv.stop_transcode(&self.client, &session);
        }
    }

    pub fn report(&self, server_id: String, key: String, state: String, time_ms: u64, duration_ms: u64, transcode_session: Option<String>) {
        let synced = self.server(&server_id).is_ok_and(|srv| srv.timeline(&self.client, &key, &state, time_ms, duration_ms, transcode_session.as_deref()));
        if state == "stopped"
            && let Some(d) = self.downloads.get()
            && d.local(&server_id, &key).is_some()
        {
            d.record_progress(&server_id, &key, &state, time_ms, duration_ms, synced);
        }
        if let Some(state) = play_state(&state) {
            self.scrobble(&server_id, &key, state, time_ms, duration_ms);
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
        let info = TrackerSignIn { url: s.url.clone(), user_code: s.user_code.clone(), qr_size, qr, interval_secs: s.interval_secs, expires_in_secs: s.expires_in_secs };
        *self.tracker_sign_in.lock().unwrap() = Some(s);
        Ok(info)
    }

    pub fn poll_tracker_sign_in(&self) -> TrackerPoll {
        let Some(mut s) = self.tracker_sign_in.lock().unwrap().take() else { return TrackerPoll::Failed { reason: "cancelled".into() } };
        match self.trackers.poll_sign_in(&mut s) {
            Poll::Pending => {
                let interval_secs = s.interval_secs;
                self.tracker_sign_in.lock().unwrap().get_or_insert(s);
                TrackerPoll::Pending { interval_secs }
            }
            Poll::Done => TrackerPoll::Done,
            Poll::Failed(reason) => TrackerPoll::Failed { reason },
        }
    }

    pub fn start_downloads(&self, root: String, listener: Box<dyn DownloadListener>) {
        if self.downloads.get().is_some() {
            return;
        }
        if self.downloads.set(Downloads::new(root.into(), self.client.clone(), move || listener.changed())).is_err() {
            return;
        }
        self.sync_download_servers();
        let auto_remove = self.settings.lock().unwrap().auto_remove_watched;
        self.with_downloads(|d| d.set_auto_remove_watched(auto_remove));
    }

    pub fn downloads(&self) -> Vec<Download> {
        let Some(d) = self.downloads.get() else { return Vec::new() };
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
            Some(k) if k.is_empty() => return Err("This item has no season or show to download".into()),
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

    pub fn offline_library(&self) -> Vec<Card> {
        let Some(d) = self.downloads.get() else { return Vec::new() };
        let mut items = d.library();
        items.sort_by_cached_key(|(_, it)| (it.card_text().0.to_lowercase(), it.parent_index, it.index));
        items.iter().map(|(server, it)| offline_card(d, server, it, false)).collect()
    }
}

#[uniffi::export]
pub fn move_library(mut settings: Settings, arranged: Vec<String>, id: String, step: i32) -> Settings {
    settings.move_library(&arranged, &id, step as isize);
    settings
}

#[uniffi::export]
pub fn pick_tracks(audio: Vec<TrackInfo>, text: Vec<TrackInfo>, languages: LanguagePrefs) -> TrackPick {
    let (a, t) = prefs::pick(&audio, &text, &languages);
    TrackPick { audio: a.map(|i| i as u32), text: t.map(|i| i as u32) }
}

#[uniffi::export]
pub fn languages() -> Vec<Language> {
    LANGUAGES.iter().map(|(name, codes)| Language { code: codes[0].into(), name: (*name).into() }).collect()
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
    SUBTITLE_COLORS.iter().map(|(hex, name)| SubtitleColor { hex: (*hex).into(), name: (*name).into() }).collect()
}

#[uniffi::export]
pub fn card_scale(density: u32) -> f32 {
    Settings { density, ..Default::default() }.card_scale()
}

fn download_parent<'a>(scope: &DownloadScope, it: &'a Item) -> Option<&'a String> {
    match (scope, it.kind.as_str()) {
        (DownloadScope::Item, _) | (DownloadScope::Season, "season") | (DownloadScope::Show, "show") => None,
        (DownloadScope::Season, _) | (DownloadScope::Show, "season") => Some(&it.parent_rating_key),
        (DownloadScope::Show, _) => Some(&it.grandparent_rating_key),
    }
}

fn qr(text: &str) -> R<(u32, Vec<bool>)> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).map_err(|e| format!("QR code: {e:?}"))?;
    let size = qr.size();
    Ok((size as u32, (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).map(|(x, y)| qr.get_module(x, y)).collect()))
}

fn play_state(state: &str) -> Option<PlayState> {
    match state {
        "playing" => Some(PlayState::Playing),
        "paused" => Some(PlayState::Paused),
        "stopped" => Some(PlayState::Stopped),
        _ => None,
    }
}

fn guid_ids(it: &Item) -> Ids {
    ids_from_guids(it.guids.iter().map(|g| g.id.as_str()))
}

fn tracker_media(it: &Item) -> Option<Media> {
    let index = |i: Option<i64>| i.and_then(|i| u32::try_from(i).ok());
    match it.kind.as_str() {
        "movie" => Some(Media { movie: true, ids: guid_ids(it), ..Default::default() }),
        "episode" => Some(Media { ids: guid_ids(it), season: index(it.parent_index), episode: index(it.index), ..Default::default() }),
        _ => None,
    }
}

fn apply_log_level(settings: &Settings) {
    log::set_max_level(if settings.debug_logging { log::LevelFilter::Debug } else { log::LevelFilter::Info });
}

impl Plaivo {
    fn add_jellyfin(&self, srv: Server) {
        log::info!("jellyfin: signed in to {} as {}", srv.name, srv.user);
        let mut account = self.account.lock().unwrap();
        account.add_jellyfin(srv);
        account.save();
        drop(account);
        self.sync_download_servers();
    }

    fn scrobble(&self, server: &str, key: &str, state: PlayState, time_ms: u64, duration_ms: u64) {
        if !Service::ALL.into_iter().map(|s| self.trackers.status(s)).any(|s| s.signed_in && s.scrobble) {
            return;
        }
        let Some((mut media, show)) = self.tracked.lock().unwrap().get(&(server.to_owned(), key.to_owned())).cloned() else { return };
        if let Some(show) = show {
            let Some(ids) = self.show_ids(server, &show) else { return };
            media.show_ids = ids;
        }
        self.trackers.on_progress(&media, state, time_ms, duration_ms);
    }

    fn show_ids(&self, server: &str, show: &str) -> Option<Ids> {
        let id = (server.to_owned(), show.to_owned());
        if let Some(ids) = self.show_ids.lock().unwrap().get(&id) {
            return Some(ids.clone());
        }
        let it = self.downloads.get().and_then(|d| d.item(server, show)).or_else(|| self.server(server).ok()?.metadata(&Ctx::new(self.client.clone(), Source::Network), show).ok());
        let ids = guid_ids(&it?);
        self.show_ids.lock().unwrap().insert(id, ids.clone());
        Some(ids)
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
            .map(|(path, st)| Subtitle { url: file_url(path), codec: st.codec.clone(), title: st.display_title.clone(), language: st.language_code.clone(), forced: st.forced })
            .collect();
        let languages = self.settings.lock().unwrap().languages();
        let file = local.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        log::info!("playing {key} from the downloaded file");
        Some(self.playback_for(server, &it, file_url(&local.file), file, subtitles, None, String::new(), languages))
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
    ) -> Playback {
        let (title, subtitle) = it.card_text();
        let start_ms = it.view_offset.filter(|&o| o > 5_000 && it.progress() < 0.95).unwrap_or(0);
        let settings = self.settings.lock().unwrap();
        let markers = it
            .marker
            .iter()
            .filter_map(|m| {
                let label = match m.kind.as_str() {
                    "intro" => "Skip Intro",
                    "credits" => "Skip Credits",
                    "commercial" => "Skip Ad",
                    _ => return None,
                };
                Some(SkipMarker { kind: m.kind.clone(), mode: settings.skip_mode(&m.kind), label: label.into(), start_ms: m.start_time_offset, end_ms: m.end_time_offset })
            })
            .collect();
        drop(settings);
        if let Some(m) = tracker_media(it) {
            let show = (!m.movie && !it.grandparent_rating_key.is_empty()).then(|| it.grandparent_rating_key.clone());
            self.tracked.lock().unwrap().insert((server.to_owned(), it.rating_key.clone()), (m, show));
        }
        let media = it.media.first();
        let resolution = match media.map(|m| m.video_resolution.to_ascii_lowercase()).unwrap_or_default().as_str() {
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
            show_key: (it.kind == "episode" && !it.grandparent_rating_key.is_empty()).then(|| it.grandparent_rating_key.clone()),
            file,
            container: media.map(|m| m.container.clone()).unwrap_or_default(),
            bitrate_kbps: media.and_then(|m| m.bitrate).unwrap_or(0),
            source: join(&[media.and_then(|m| m.bitrate).map(fmt_bitrate).unwrap_or_default(), resolution]),
            transcode_session,
            transcode_label,
            languages,
        }
    }

    fn server(&self, id: &str) -> R<Server> {
        self.account.lock().unwrap().server(id).cloned().ok_or_else(|| "Unknown server".into())
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
    licenses::LICENSES.iter().map(|l| RustLicense { name: l.name.into(), version: l.version.into(), license: l.license.into() }).collect()
}

#[uniffi::export]
pub fn rust_license_text(index: u32) -> String {
    licenses::LICENSES.get(index as usize).map(|l| l.parts().collect::<Vec<_>>().join("\n\n")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #[test]
    fn download_scope_picks_the_season_or_show() {
        use super::{DownloadScope as S, Item, download_parent};
        let episode = Item { kind: "episode".into(), parent_rating_key: "s".into(), grandparent_rating_key: "g".into(), ..Default::default() };
        let season = Item { kind: "season".into(), parent_rating_key: "g".into(), ..Default::default() };
        assert_eq!(download_parent(&S::Item, &episode), None);
        assert_eq!(download_parent(&S::Season, &episode).map(String::as_str), Some("s"));
        assert_eq!(download_parent(&S::Show, &episode).map(String::as_str), Some("g"));
        assert_eq!(download_parent(&S::Season, &season), None);
        assert_eq!(download_parent(&S::Show, &season).map(String::as_str), Some("g"));
    }

    #[test]
    fn tracker_media_maps_movies_and_episodes() {
        use super::{Item, PlayState, play_state, tracker_media};
        use plaivo_core::plex::Guid;
        let guids = |ids: &[&str]| ids.iter().map(|id| Guid { id: (*id).into() }).collect();
        let movie = Item { kind: "movie".into(), guids: guids(&["imdb://tt1", "tmdb://2", "local://x"]), ..Default::default() };
        let m = tracker_media(&movie).unwrap();
        assert!(m.movie && m.season.is_none() && m.order.is_none());
        assert_eq!(m.ids.get("imdb").map(String::as_str), Some("tt1"));
        assert_eq!(m.ids.get("tmdb").map(String::as_str), Some("2"));
        assert!(!m.ids.contains_key("local"));
        let episode = Item { kind: "episode".into(), parent_index: Some(2), index: Some(5), guids: guids(&["tvdb://9"]), ..Default::default() };
        let e = tracker_media(&episode).unwrap();
        assert_eq!((e.movie, e.season, e.episode, e.ids.get("tvdb").map(String::as_str)), (false, Some(2), Some(5), Some("9")));
        assert_eq!(tracker_media(&Item { kind: "episode".into(), index: Some(-1), ..Default::default() }).unwrap().episode, None);
        assert!(tracker_media(&Item { kind: "clip".into(), ..Default::default() }).is_none());
        assert_eq!(play_state("playing"), Some(PlayState::Playing));
        assert_eq!(play_state("paused"), Some(PlayState::Paused));
        assert_eq!(play_state("stopped"), Some(PlayState::Stopped));
        assert_eq!(play_state("buffering"), None);
    }

    #[test]
    fn license_list_matches_cargo_tree() {
        super::licenses::assert_matches_cargo_tree("plaivo-ffi");
    }
}
