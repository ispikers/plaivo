use plaivo_core::plex::{Client, Ctx, Item, QUALITIES, Quality, Server, Source, fmt_bitrate, resolve_playable};
use plaivo_core::{Account, first, join, store};
use qrcodegen::{QrCode, QrCodeEcc};
use std::sync::{Arc, Mutex};

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

#[derive(uniffi::Record)]
pub struct SignIn {
    pub id: u64,
    pub code: String,
    pub qr_size: u32,
    pub qr: Vec<bool>,
    pub expires_in_s: u64,
}

#[derive(uniffi::Record)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub owned: bool,
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
}

#[derive(uniffi::Record)]
pub struct QualityOption {
    pub kbps: u32,
    pub label: String,
    pub bitrate: String,
}

#[derive(uniffi::Record)]
pub struct SkipMarker {
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
}

#[derive(uniffi::Object)]
pub struct Plaivo {
    client: Client,
    account: Mutex<Account>,
}

fn card(srv: &Server, it: &Item, wide: bool) -> Card {
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
        server: srv.id.clone(),
        key: it.rating_key.clone(),
        kind: it.kind.clone(),
        title,
        subtitle,
        image: (!art.is_empty()).then(|| srv.image_url(art, w, h)),
        backdrop: (!backdrop.is_empty()).then(|| srv.image_url(backdrop, 1920, 1080)),
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
        Arc::new(Self { client: Client::new(account.client_id.clone()), account: Mutex::new(account) })
    }

    pub fn signed_in(&self) -> bool {
        self.account.lock().unwrap().token.is_some()
    }

    pub fn start_sign_in(&self) -> R<SignIn> {
        let pin = self.client.create_pin(false)?;
        let link = self.client.link_url(&pin.code);
        let qr = QrCode::encode_text(&link, QrCodeEcc::Medium).map_err(|e| format!("QR code: {e:?}"))?;
        let size = qr.size();
        let modules = (0..size).flat_map(|y| (0..size).map(move |x| (x, y))).map(|(x, y)| qr.get_module(x, y)).collect();
        Ok(SignIn { id: pin.id, code: pin.code, qr_size: size as u32, qr: modules, expires_in_s: pin.expires_in.unwrap_or(900) })
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
    }

    pub fn servers(&self) -> Vec<ServerInfo> {
        self.account.lock().unwrap().servers.iter().map(|s| ServerInfo { id: s.id.clone(), name: s.name.clone(), owned: s.owned }).collect()
    }

    pub fn refresh_servers(&self) -> R<Vec<ServerInfo>> {
        let token = self.account.lock().unwrap().token.clone().ok_or("Not signed in")?;
        let resources = self.client.servers(&token)?;
        let connected: Vec<Server> = std::thread::scope(|s| {
            let probes: Vec<_> = resources.iter().map(|r| s.spawn(|| self.client.connect(r, &token))).collect();
            probes.into_iter().filter_map(|p| p.join().ok()?.inspect_err(|e| log::warn!("{e}")).ok()).collect()
        });
        let mut account = self.account.lock().unwrap();
        account.servers = connected;
        account.save();
        drop(account);
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
        let servers = self.account.lock().unwrap().servers.clone();
        let ctx = Ctx::new(self.client.clone(), if cached { Source::Cache } else { Source::Network });
        std::thread::scope(|s| {
            let jobs: Vec<_> = servers.iter().map(|srv| (srv, s.spawn(|| srv.sections(&ctx)))).collect();
            jobs.into_iter()
                .filter_map(|(srv, job)| Some((srv, job.join().ok()?.inspect_err(|e| log::warn!("sections for {}: {e}", srv.name)).ok()?)))
                .flat_map(|(srv, dirs)| {
                    dirs.into_iter().map(|d| Section { server: srv.id.clone(), server_name: srv.name.clone(), movies: d.kind == "movie", key: d.key, title: d.title })
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
        let servers = self.account.lock().unwrap().servers.clone();
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
                srv.transcode_url(&self.client, &it.rating_key, q, s)
            }
            _ => srv.media_url(&part.key),
        };
        let (title, subtitle) = it.card_text();
        let start_ms = it.view_offset.filter(|&o| o > 5_000 && it.progress() < 0.95).unwrap_or(0);
        let subtitles = part
            .stream
            .iter()
            .filter(|st| st.stream_type == 3 && !st.key.is_empty())
            .map(|st| Subtitle { url: srv.media_url(&st.key), codec: st.codec.clone(), title: st.display_title.clone(), language: st.language_code.clone() })
            .collect();
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
                Some(SkipMarker { label: label.into(), start_ms: m.start_time_offset, end_ms: m.end_time_offset })
            })
            .collect();
        let resolution = match media.video_resolution.to_ascii_lowercase().as_str() {
            "" => String::new(),
            "4k" => "4K".into(),
            "sd" => "SD".into(),
            r if r.chars().all(|c| c.is_ascii_digit()) => format!("{r}p"),
            r => r.to_uppercase(),
        };
        let source = join(&[media.bitrate.map(fmt_bitrate).unwrap_or_default(), resolution]);
        let transcode_label = transcode.map(|q| format!("{} · {}", q.label, fmt_bitrate(q.kbps as u64))).unwrap_or_default();
        let media = it.media.first();
        Ok(Playback {
            url,
            server: srv.id.clone(),
            key: it.rating_key.clone(),
            title,
            subtitle,
            start_ms,
            duration_ms: it.duration.unwrap_or(0),
            subtitles,
            markers,
            show_key: (it.kind == "episode" && !it.grandparent_rating_key.is_empty()).then(|| it.grandparent_rating_key.clone()),
            file: part.file.rsplit(['/', '\\']).next().unwrap_or_default().to_owned(),
            container: media.map(|m| m.container.clone()).unwrap_or_default(),
            bitrate_kbps: media.and_then(|m| m.bitrate).unwrap_or(0),
            source,
            transcode_session: session,
            transcode_label,
        })
    }

    pub fn queue(&self, server_id: String, show_key: String) -> R<Vec<Card>> {
        let srv = self.server(&server_id)?;
        let episodes = srv.all_episodes(&Ctx::new(self.client.clone(), Source::Network), &show_key)?;
        Ok(episodes.iter().map(|it| card(&srv, it, true)).collect())
    }

    pub fn stop_transcode(&self, server_id: String, session: String) {
        if let Ok(srv) = self.server(&server_id) {
            srv.stop_transcode(&self.client, &session);
        }
    }

    pub fn report(&self, server_id: String, key: String, state: String, time_ms: u64, duration_ms: u64) {
        if let Ok(srv) = self.server(&server_id) {
            srv.timeline(&self.client, &key, &state, time_ms, duration_ms);
        }
    }
}

impl Plaivo {
    fn server(&self, id: &str) -> R<Server> {
        self.account.lock().unwrap().servers.iter().find(|s| s.id == id).cloned().ok_or_else(|| "Unknown server".into())
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
            m.level() <= Level::Info
        }

        fn log(&self, r: &Record) {
            let prio = match r.level() {
                Level::Error => 6,
                Level::Warn => 5,
                Level::Info => 4,
                Level::Debug => 3,
                Level::Trace => 2,
            };
            if let Ok(text) = CString::new(r.args().to_string()) {
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
