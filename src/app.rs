use crate::downloads::{Downloads, Entry, State};
use crate::images::{self, Lru};
use crate::player::Player;
use crate::plex::{
    self, Client, Ctx, DISCOVER_ID, Item, R, SORTS, Server, Source, initial_sort, sort_items,
};
use crate::settings::{BLUR_PARAM, CardPrefs, EpisodePoster, Settings, ViewMode, library_key};
use crate::store;
use crate::ui::add_server::AddServer;
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::services::Services;
use crate::update::Update;
use crate::watch::Watch;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use gpui::{
    AppContext, Bounds, Context, Entity, FocusHandle, Pixels, PromptLevel, RenderImage,
    ScrollHandle, SharedString, Task, UniformListScrollHandle, Window,
};
pub use plaivo_core::{Account, first, join};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const PAGE: usize = 120;
const IMAGE_BUDGET: usize = 192 << 20;
const PLAYER_IMAGE_BUDGET: usize = 48 << 20;
const LIBRARY_CACHE: usize = 4;
const SUGGESTIONS: usize = 40;
const SPLASH_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_WAIT: Duration = Duration::from_millis(1500);
pub const IMAGE_FADE: Duration = Duration::from_millis(300);
const FIRST_ROW_IMAGES: usize = 6;
const TOAST_FOR: Duration = Duration::from_secs(5);
const LOCAL_IMAGE: &str = "#local";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Splash,
    Auth,
    Servers,
    Home,
    Library,
    HubList,
    Search,
    Detail,
    Settings,
    Licenses,
    Logs,
    AddServer,
    Downloads,
    Watch,
    Player,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Nav {
    Home,
    Servers,
    Library(String, String),
    Collection(String, String, String),
    Person(String, String, String),
    Hub(String, String, String, bool),
    Watchlist,
    Search,
    Detail(String, String, Option<String>),
    Settings(Option<Category>),
    Licenses(Option<usize>),
    Logs,
    AddServer,
    Downloads(Option<(String, String)>),
    Watch,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    General,
    Libraries,
    Appearance,
    Playback,
    Shaders,
    Audio,
    AudioLanguage,
    SubtitleLanguage,
    LanguageOverride,
    PlaybackOverride,
    Downloads,
    Services,
    WatchTogether,
    Connections,
    Advanced,
    About,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Shape {
    Poster,
    Wide,
    Backdrop,
    Logo,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Thumb {
    pub server: String,
    pub path: String,
    pub blur: bool,
}

static CARD_PREFS: std::sync::Mutex<CardPrefs> = std::sync::Mutex::new(CardPrefs {
    poster: EpisodePoster::Thumbnail,
    numbers: true,
    spoilers: false,
});

pub fn set_card_prefs(s: &Settings) {
    *CARD_PREFS.lock().unwrap_or_else(|e| e.into_inner()) = s.card_prefs();
}

fn card_prefs() -> CardPrefs {
    *CARD_PREFS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn wide_hub(h: &plex::Hub) -> bool {
    card_prefs().wide_hub(h)
}

#[derive(Clone, Default, Debug)]
pub struct Card {
    pub key: SharedString,
    pub kind: SharedString,
    pub title: SharedString,
    pub subtitle: SharedString,
    pub summary: SharedString,
    pub meta: SharedString,
    pub progress: f32,
    pub watched: bool,
    pub unwatched: u64,
    pub parent: SharedString,
    pub server: SharedString,
    pub resume: bool,
    pub thumb: Option<Thumb>,
}

#[derive(Clone, Default)]
pub struct Choice {
    pub id: SharedString,
    pub label: SharedString,
    pub sub: SharedString,
    pub selected: bool,
}

pub type Slide = Option<(f32, f32, Instant)>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShelfKey {
    Hub(bool, usize),
    Detail(usize),
}

pub struct Hub {
    pub title: SharedString,
    pub key: SharedString,
    pub server: SharedString,
    pub resume: bool,
    pub wide: bool,
    pub items: Vec<Card>,
    pub scroll: ScrollHandle,
    pub slide: Slide,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Listing {
    Items,
    Collections,
    Collection,
    Watchlist,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LibTab {
    Recommended,
    Browse,
    Collections,
}

pub const LIB_TABS: [(LibTab, &str); 3] = [
    (LibTab::Recommended, "Recommended"),
    (LibTab::Browse, "Browse"),
    (LibTab::Collections, "Collections"),
];

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct LibKey {
    pub server: String,
    pub section: String,
    pub sort: String,
    pub listing: Listing,
}

pub struct HubList {
    pub server: String,
    pub key: String,
    pub title: SharedString,
    pub resume: bool,
    pub sort: String,
    items: Vec<Item>,
    pub cards: Vec<Card>,
    pub loaded: bool,
    pub scroll: UniformListScrollHandle,
}

impl HubList {
    fn rebuild(&mut self) {
        let mut items: Vec<&Item> = self.items.iter().collect();
        sort_items(&mut items, &self.sort);
        self.cards = items
            .into_iter()
            .map(|it| Card {
                resume: self.resume,
                ..card_for(it, false, &self.server)
            })
            .collect();
    }

    pub fn sorts(&self) -> Vec<(&'static str, &'static str)> {
        plex::hub_sorts(&self.items, &self.key)
    }
}

pub struct Library {
    pub key: LibKey,
    pub title: SharedString,
    pub items: Vec<Option<Card>>,
    pages: HashSet<usize>,
    pub scroll: UniformListScrollHandle,
}

#[derive(Default)]
pub struct DetailView {
    pub server: SharedString,
    pub eyebrow: SharedString,
    pub title: SharedString,
    pub chips: Vec<SharedString>,
    pub genres: Vec<SharedString>,
    pub summary: SharedString,
    pub play_label: SharedString,
    pub play_key: SharedString,
    pub progress: f32,
    pub art: Option<Thumb>,
    pub is_show: bool,
    pub item: Item,
    pub logo: Option<Thumb>,
    pub ratings: Vec<(plex::RatingSource, SharedString)>,
    pub media_line: SharedString,
    pub info: Vec<(SharedString, SharedString)>,
    pub cast: Vec<CastMember>,
    pub extras: Vec<Card>,
    pub related: Vec<(SharedString, Vec<Card>)>,
    pub watchlisted: Option<bool>,
    pub photo: Option<Thumb>,
}

#[derive(Clone)]
pub struct CastMember {
    pub id: String,
    pub name: SharedString,
    pub role: SharedString,
    pub thumb: Option<Thumb>,
}

pub struct SectionInfo {
    pub id: String,
    pub server: String,
    pub key: String,
    pub title: SharedString,
    pub kind: String,
}

#[derive(Default)]
pub struct Splash {
    pub servers: Vec<(String, String, Status)>,
    probes_done: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Pending,
    Ok,
    Error,
}

#[derive(Default)]
pub struct ViewState {
    pub sidebar_anim: Option<Instant>,
    pub hero_prev: Option<(usize, Instant)>,
    pub hero_h: f32,
    pub hero_drag: Option<f32>,
    pub hero_acc: f32,
    pub hero_cooldown: Option<Instant>,
    pub timeline: Bounds<Pixels>,
    pub timeline_hover: Option<f32>,
    pub seek_drag: Option<f32>,
    pub seek_preview: Option<Instant>,
    pub volume: Bounds<Pixels>,
    pub volume_drag: bool,
    pub volume_wheel: f32,
    pub shelf_hover: Option<ShelfKey>,
    pub shelf_arrow_hover: Option<ShelfKey>,
    pub cursor_hidden: bool,
    pub boost: Option<f64>,
    pub space_down: bool,
    pub mouse_hold: Option<Task<()>>,
}

pub struct Plaivo {
    pub focus: FocusHandle,
    pub view: ViewState,
    pub window: RawWindowHandle,
    pub scale: f32,
    pub viewport_w: u32,
    pub account: Account,
    pub settings: Settings,
    pub client: Client,
    pub servers: Vec<Server>,
    pub sections: Vec<SectionInfo>,
    pub page: Page,
    pub nav: Vec<Nav>,
    pub settings_page: Option<Category>,
    pub license: Option<usize>,
    pub add_server: AddServer,
    pub services: Services,
    pub watch: Watch,
    pub updates: Update,
    pub licenses_scroll: UniformListScrollHandle,
    pub logs_scroll: UniformListScrollHandle,
    pub log_selection: Option<(usize, usize)>,
    pub toast: Option<SharedString>,
    toast_task: Option<Task<()>>,
    pub auth_waiting: bool,
    pub auth_error: Option<SharedString>,
    pin_gen: Arc<AtomicU64>,
    pub splash: Option<Splash>,
    probe: Option<(Vec<Server>, usize, bool)>,
    pub servers_loading: bool,
    pub servers_error: Option<SharedString>,
    pub hero: Vec<Card>,
    pub hero_index: usize,
    hero_task: Option<Task<()>>,
    pub hubs: Vec<Hub>,
    home_continue: plex::Hub,
    remote_continue: HashMap<String, Vec<Item>>,
    pub home_error: Option<SharedString>,
    pub home_ready: bool,
    pub suggestions: Vec<Card>,
    pub library: Option<Library>,
    library_cache: Vec<Library>,
    pub lib_tab: LibTab,
    pub watchlist_shows: bool,
    pub lib_hubs: Option<Vec<Hub>>,
    lib_hubs_for: (String, String),
    pub lib_hubs_scroll: ScrollHandle,
    pub hub_list: Option<HubList>,
    pub sort_open: bool,
    pub search: Entity<TextInput>,
    pub results: Vec<Card>,
    pub searching: bool,
    search_gen: u64,
    search_task: Option<Task<()>>,
    pub grid_scroll: UniformListScrollHandle,
    pub detail: Option<DetailView>,
    pub menu: Option<crate::ui::item_menu::ItemMenu>,
    pub dialog: Option<crate::ui::item_menu::Dialog>,
    pub rule_draft: Option<crate::ui::language_rules::RuleDraft>,
    pub playback_draft: Option<crate::ui::playback_rules::PlaybackDraft>,
    pub detail_id: (String, String),
    owned_check: Option<String>,
    pub seasons: Vec<Choice>,
    pub season_items: Vec<Item>,
    pub season: Option<String>,
    pub episodes: Vec<Card>,
    pub episode_items: Vec<Item>,
    pub downloads: Option<Arc<Downloads>>,
    downloads_tx: UnboundedSender<()>,
    opening_downloads: bool,
    pub offline_cards: Vec<Card>,
    pub offline_scroll: UniformListScrollHandle,
    pub download_entries: Vec<Entry>,
    pub offline: Vec<(String, Item)>,
    pub offline_view: Option<(String, String)>,
    pub detail_scroll: ScrollHandle,
    pub detail_shelves: Vec<(ScrollHandle, Slide)>,
    pub summary_expanded: bool,
    pub arrivals: HashMap<String, Instant>,
    pub settings_scroll: ScrollHandle,
    pub license_scroll: ScrollHandle,
    pub home_scroll: ScrollHandle,
    queue: Arc<images::Queue>,
    pub lru: Lru,
    image_key: String,
    requested: HashMap<String, u64>,
    pub(crate) settings_save: Option<Task<()>>,
    failed: HashSet<String>,
    pub player: Player,
}

pub fn open_url(url: &str) {
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(url).spawn();
    if let Err(e) = r {
        log::error!("could not open browser: {e}");
    }
}

pub fn download_root(dir: &str) -> PathBuf {
    if !dir.is_empty() {
        return dir.into();
    }
    store::config_dir().join("downloads")
}

fn open_downloads(root: PathBuf, client: Client, tx: UnboundedSender<()>) -> Arc<Downloads> {
    Arc::new(Downloads::new(root, client, move || {
        let _ = tx.unbounded_send(());
    }))
}

pub fn thumb(server: &str, path: &str) -> Option<Thumb> {
    (!path.is_empty()).then(|| Thumb {
        server: server.to_owned(),
        path: path.to_owned(),
        blur: false,
    })
}

pub fn card_for(it: &Item, wide: bool, server: &str) -> Card {
    let prefs = card_prefs();
    let (title, subtitle) = prefs.text(it);
    let art = prefs.art(it, wide);
    let blur = prefs.blur(it, art);
    Card {
        key: it.rating_key.clone().into(),
        kind: it.kind.clone().into(),
        title: title.into(),
        subtitle: subtitle.into(),
        progress: it.progress(),
        watched: it.watched(),
        unwatched: it.unwatched(),
        parent: if it.kind == "season" {
            &it.parent_rating_key
        } else {
            &it.grandparent_rating_key
        }
        .clone()
        .into(),
        server: server.to_owned().into(),
        thumb: thumb(server, art).map(|t| Thumb { blur, ..t }),
        ..Default::default()
    }
}

fn hub_rows(hubs: &[plex::Hub], server: &str, old: &[Hub]) -> Vec<Hub> {
    hubs.iter()
        .map(|h| {
            let wide = wide_hub(h);
            let resume = h.is_continue();
            let scroll = old
                .iter()
                .find(|o| o.resume == resume && (resume || (o.key == h.key && o.title == h.title)))
                .map(|o| o.scroll.clone())
                .unwrap_or_default();
            let items = h
                .metadata
                .iter()
                .map(|it| Card {
                    resume,
                    ..card_for(it, wide, server)
                })
                .collect();
            Hub {
                title: h.title.clone().into(),
                key: h.key.clone().into(),
                server: server.to_owned().into(),
                resume,
                wide,
                items,
                scroll,
                slide: None,
            }
        })
        .collect()
}

pub fn episode_card(e: &Item, server: &str) -> Card {
    let mut card = card_for(e, true, server);
    let hide = card_prefs().spoiler(e);
    if e.kind == "episode" {
        card.title = format!("{}. {}", e.index.unwrap_or(0), e.title).into();
        card.thumb = thumb(server, &e.thumb).map(|t| Thumb { blur: hide, ..t });
    }
    card.summary = if hide {
        SharedString::default()
    } else {
        e.summary.clone().into()
    };
    card.meta = join(&[
        e.duration.map(plex::fmt_duration).unwrap_or_default(),
        e.originally_available_at.clone(),
    ])
    .into();
    card
}

fn hero_card(it: &Item, server: &str) -> Card {
    let mut card = card_for(it, true, server);
    let kind = match it.kind.as_str() {
        "movie" => "Movie",
        "show" => "TV Show",
        _ => "",
    };
    card.title = first(&[&it.grandparent_title, &it.title]).to_owned().into();
    card.meta = join(&[
        kind.into(),
        it.audience_rating
            .map(|r| format!("★ {r:.1}"))
            .unwrap_or_default(),
        it.content_rating.clone(),
        it.year.map(|y| y.to_string()).unwrap_or_default(),
    ])
    .into();
    card.summary = it.summary.clone().into();
    card.thumb = thumb(server, first(&[&it.art, &it.grandparent_art]));
    card
}

fn title_rank(title: &str, q: &str) -> u8 {
    let (t, q) = (title.to_lowercase(), q.to_lowercase());
    if t == q {
        0
    } else if t.starts_with(&q) {
        1
    } else if t
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| !w.is_empty() && w.starts_with(&q))
    {
        2
    } else if t.contains(&q) {
        3
    } else {
        4
    }
}

fn rank_search(
    q: &str,
    home: &str,
    local: Vec<(String, Vec<(Item, f64)>)>,
    discover: Vec<Item>,
) -> Vec<(String, Item)> {
    let mut seen = HashSet::new();
    let mut owned_guids = HashSet::new();
    let mut lib: Vec<(u8, bool, f64, String, Item)> = Vec::new();
    for (server, hits) in local {
        for (it, score) in hits {
            let it = it.as_show();
            if !matches!(it.kind.as_str(), "movie" | "show")
                || !seen.insert((server.clone(), it.rating_key.clone()))
            {
                continue;
            }
            if !it.guid.is_empty() && !owned_guids.insert(it.guid.clone()) {
                continue;
            }
            lib.push((
                title_rank(&it.title, q),
                server != home,
                -score,
                server.clone(),
                it,
            ));
        }
    }
    lib.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)));
    let owned_titles: HashSet<(String, Option<i64>)> = lib
        .iter()
        .map(|e| (e.4.title.to_lowercase(), e.4.year))
        .collect();
    let mut out: Vec<(String, Item)> = lib.into_iter().map(|e| (e.3, e.4)).collect();
    out.extend(
        discover
            .into_iter()
            .filter(|it| matches!(it.kind.as_str(), "movie" | "show"))
            .filter(|it| {
                !owned_guids.contains(&it.guid)
                    && !owned_titles.contains(&(it.title.to_lowercase(), it.year))
            })
            .map(|it| (DISCOVER_ID.to_owned(), it)),
    );
    out
}

pub fn shape_size(shape: Shape, scale: f32, width: u32) -> (u32, u32) {
    let round = |w: f32| ((w * scale / 50.0).ceil() * 50.0) as u32;
    match shape {
        Shape::Poster => {
            let w = round(210.0);
            (w, w * 3 / 2)
        }
        Shape::Wide => {
            let w = round(380.0);
            (w, w * 9 / 16)
        }
        Shape::Backdrop => {
            let w = width.clamp(1280, 3840) / 320 * 320;
            (w, w * 9 / 16)
        }
        Shape::Logo => (round(480.0), round(180.0)),
    }
}

fn image_key(buf: &mut String, t: &Thumb, (w, h): (u32, u32)) {
    use std::fmt::Write;
    buf.clear();
    let _ = write!(
        buf,
        "{}{}@{w}x{h}{}",
        t.server,
        t.path,
        if t.blur { "#blur" } else { "" }
    );
}

pub use plex::fmt_resolution;

pub fn poll_until<T>(
    attempt: &AtomicU64,
    current: u64,
    every: Duration,
    deadline: Instant,
    mut step: impl FnMut() -> Option<T>,
) -> Option<T> {
    while Instant::now() < deadline {
        std::thread::sleep(every);
        if attempt.load(Ordering::SeqCst) != current {
            return None;
        }
        if let Some(v) = step() {
            return Some(v);
        }
    }
    None
}

type ImageRx = futures::channel::mpsc::UnboundedReceiver<(String, Option<Arc<RenderImage>>)>;

impl Plaivo {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        account: Account,
        settings: Settings,
        client: Client,
        queue: Arc<images::Queue>,
        images_rx: ImageRx,
    ) -> Self {
        let raw = window.window_handle().expect("window handle").as_raw();
        set_card_prefs(&settings);
        let search = cx.new(|cx| TextInput::new("Search movies and shows", cx));
        cx.subscribe(
            &search,
            |this: &mut Self, _, ev: &InputEvent, cx| match ev {
                InputEvent::Changed(q) => this.search_debounced(q.to_string(), cx),
                InputEvent::Submit(q) => this.search(q.to_string(), cx),
            },
        )
        .detach();
        cx.spawn(async move |this, cx| {
            let mut rx = images_rx;
            while let Some((key, img)) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.on_image(key, img, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let add_server = AddServer::new(cx);
        let watch = Watch::new(&settings.watch_name, &settings.watch_relay, cx);
        let (downloads_tx, mut downloads_rx) = futures::channel::mpsc::unbounded();
        cx.spawn(async move |this, cx| {
            while downloads_rx.next().await.is_some() {
                while downloads_rx.try_recv().is_ok() {}
                if this
                    .update(cx, |this, cx| this.downloads_changed(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let mut s = Self {
            focus: cx.focus_handle(),
            view: ViewState::default(),
            window: raw,
            scale: window.scale_factor(),
            viewport_w: (f32::from(window.viewport_size().width) * window.scale_factor()) as u32,
            account,
            settings,
            client,
            servers: Vec::new(),
            sections: Vec::new(),
            page: Page::Auth,
            nav: Vec::new(),
            settings_page: None,
            license: None,
            add_server,
            services: Services::start(),
            watch,
            updates: Update::start(cx),
            licenses_scroll: UniformListScrollHandle::new(),
            logs_scroll: UniformListScrollHandle::new(),
            log_selection: None,
            toast: None,
            toast_task: None,
            auth_waiting: false,
            auth_error: None,
            pin_gen: Arc::new(AtomicU64::new(0)),
            splash: None,
            probe: None,
            servers_loading: false,
            servers_error: None,
            hero: Vec::new(),
            hero_index: 0,
            hero_task: None,
            hubs: Vec::new(),
            home_continue: plex::Hub::default(),
            remote_continue: HashMap::new(),
            home_error: None,
            home_ready: false,
            suggestions: Vec::new(),
            library: None,
            library_cache: Vec::new(),
            lib_tab: LibTab::Recommended,
            watchlist_shows: false,
            lib_hubs: None,
            lib_hubs_for: Default::default(),
            lib_hubs_scroll: ScrollHandle::new(),
            hub_list: None,
            sort_open: false,
            search,
            results: Vec::new(),
            searching: false,
            search_gen: 0,
            search_task: None,
            grid_scroll: UniformListScrollHandle::new(),
            detail: None,
            menu: None,
            dialog: None,
            rule_draft: None,
            playback_draft: None,
            detail_id: Default::default(),
            owned_check: None,
            seasons: Vec::new(),
            season_items: Vec::new(),
            season: None,
            episodes: Vec::new(),
            episode_items: Vec::new(),
            download_entries: Vec::new(),
            downloads: None,
            downloads_tx,
            opening_downloads: false,
            offline_cards: Vec::new(),
            offline_scroll: UniformListScrollHandle::new(),
            offline: Vec::new(),
            offline_view: None,
            detail_scroll: ScrollHandle::new(),
            detail_shelves: Vec::new(),
            summary_expanded: false,
            arrivals: HashMap::new(),
            settings_scroll: ScrollHandle::new(),
            license_scroll: ScrollHandle::new(),
            home_scroll: ScrollHandle::new(),
            queue,
            lru: Lru::new(IMAGE_BUDGET),
            image_key: String::new(),
            requested: HashMap::new(),
            settings_save: None,
            failed: HashSet::new(),
            player: Player::default(),
        };
        // Quit before Windows destroys the HWND; tearing down GPUI's composition target afterwards logs spurious errors.
        window.on_window_should_close(cx, |_window, cx| {
            log::info!("window closed, shutting down");
            // Shutdown (stop report, mpv teardown) takes a moment; with mpv gone the window would show through.
            #[cfg(windows)]
            if let Ok(h) = _window.window_handle() {
                crate::video_host::hide_window(h.as_raw());
            }
            #[cfg(target_os = "macos")]
            cx.hide();
            cx.quit();
            false
        });
        // Saved once at quit, not on every resize step.
        let placement = Rc::new(RefCell::new(crate::placement::current(window, cx)));
        cx.observe_window_bounds(window, {
            let placement = placement.clone();
            move |_, window, cx| {
                if let Some(p) = crate::placement::current(window, cx) {
                    *placement.borrow_mut() = Some(p);
                }
            }
        })
        .detach();
        cx.on_app_quit(move |this, _| {
            this.report_stopped_blocking();
            this.watch.shutdown();
            this.save_settings_now();
            if let Some(p) = &*placement.borrow() {
                crate::placement::save(p);
            }
            async {}
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        window.focus(&s.focus, cx);
        s.open_downloads(cx);
        if s.account.has_servers() {
            s.begin_splash(cx);
            s.set_servers(cx);
            s.discover(cx);
        }
        s
    }

    pub fn bg<T: Send + 'static>(
        cx: &mut Context<Self>,
        job: impl FnOnce() -> T + Send + 'static,
        done: impl FnOnce(&mut Self, T, &mut Context<Self>) + 'static,
    ) {
        let task = cx.background_executor().spawn(async move { job() });
        cx.spawn(async move |this, cx| {
            let v = task.await;
            this.update(cx, |this, cx| {
                done(this, v, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn detach(cx: &mut Context<Self>, job: impl FnOnce() + Send + 'static) {
        cx.background_executor()
            .spawn(async move { job() })
            .detach();
    }

    pub fn with_downloads(
        &self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&Downloads) + Send + 'static,
    ) {
        let Some(d) = self.downloads.clone() else {
            return;
        };
        Self::detach(cx, move || f(&d));
    }

    fn open_downloads(&mut self, cx: &mut Context<Self>) {
        self.opening_downloads = true;
        let old = self.downloads.take();
        let root = download_root(&self.settings.download_dir);
        let (client, tx) = (self.client.clone(), self.downloads_tx.clone());
        Self::bg(
            cx,
            move || {
                drop(old);
                open_downloads(root, client, tx)
            },
            |this, d, cx| {
                let current = download_root(&this.settings.download_dir) == d.root();
                this.downloads = Some(d);
                this.opening_downloads = false;
                if !current {
                    return this.open_downloads(cx);
                }
                let (servers, auto) = (this.servers.clone(), this.settings.auto_remove_watched);
                this.with_downloads(cx, move |d| {
                    d.set_servers(&servers);
                    if auto {
                        d.set_auto_remove_watched(true);
                    }
                });
                this.downloads_changed(cx);
                if this.page == Page::Home && this.home_error.is_some() && this.has_downloads() {
                    this.navigate(Nav::Downloads(None), true, cx);
                }
            },
        );
    }

    pub fn set_download_dir(&mut self, dir: String, cx: &mut Context<Self>) {
        if download_root(&dir) == download_root(&self.settings.download_dir) {
            return;
        }
        self.settings.download_dir = dir;
        self.save_settings_soon(cx);
        self.offline_view = None;
        if !self.opening_downloads {
            self.open_downloads(cx);
        }
        self.downloads_changed(cx);
    }

    pub fn downloads_changed(&mut self, cx: &mut Context<Self>) {
        let entries = self
            .downloads
            .as_ref()
            .map(|d| d.snapshot())
            .unwrap_or_default();
        let detail_before = (self.page == Page::Detail).then(|| self.detail_download_states());
        let changed = entries != self.download_entries;
        if changed {
            self.download_entries = entries;
        }
        let done = self
            .download_entries
            .iter()
            .filter(|e| e.state == State::Done)
            .map(|e| (e.server.as_str(), e.key.as_str()));
        let library_changed = !done.eq(self
            .offline
            .iter()
            .map(|(s, it)| (s.as_str(), it.rating_key.as_str())));
        if library_changed {
            self.offline = self
                .downloads
                .as_ref()
                .map(|d| d.library())
                .unwrap_or_default();
            self.offline_cards = crate::ui::downloads::offline_cards(&self.offline);
        }
        let visible = match self.page {
            Page::Downloads => changed || library_changed,
            Page::Detail => detail_before != Some(self.detail_download_states()),
            _ => false,
        };
        if visible {
            cx.notify();
        }
    }

    pub fn has_downloads(&self) -> bool {
        self.download_entries.iter().any(|e| e.state == State::Done)
    }

    pub(crate) fn load<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        job: impl Fn(&Ctx) -> R<T> + Send + 'static,
        done: impl Fn(&mut Self, R<T>, &mut Context<Self>) + 'static,
    ) {
        let client = self.client.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        cx.background_executor()
            .spawn(async move {
                let cached = job(&Ctx::new(client.clone(), Source::Cache)).ok();
                let had_cache = cached.is_some();
                if let Some(v) = cached {
                    let _ = tx.unbounded_send(Ok(v));
                }
                let net = Ctx::new(client, Source::Network);
                let fresh = job(&net);
                if !(had_cache && (fresh.is_err() || !net.changed())) {
                    let _ = tx.unbounded_send(fresh);
                }
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while let Some(r) = rx.next().await {
                let applied = this.update(cx, |this, cx| {
                    done(this, r, cx);
                    cx.notify();
                });
                if applied.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    pub fn toast(&mut self, msg: impl Into<SharedString>, cx: &mut Context<Self>) {
        let msg = msg.into();
        log::info!("toast: {msg}");
        self.toast = Some(msg);
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TOAST_FOR).await;
            this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn server(&self, id: &str) -> Option<Server> {
        if id == DISCOVER_ID {
            return self
                .account
                .token
                .as_deref()
                .map(|t| self.client.discover(t));
        }
        self.servers.iter().find(|s| s.id == id).cloned()
    }

    pub fn home_server(&self) -> Option<Server> {
        self.server(&self.settings.home_server)
            .or_else(|| self.servers.first().cloned())
    }

    pub fn size_for(&self, shape: Shape) -> (u32, u32) {
        shape_size(
            shape,
            self.scale * self.settings.card_scale(),
            self.viewport_w,
        )
    }

    fn image_url(&self, server: &str, path: &str, (w, h): (u32, u32)) -> Option<String> {
        if path.starts_with("http") {
            let resizer = |id: &str| {
                self.servers
                    .iter()
                    .find(|s| s.id == id && !s.is_jellyfin() && !plex::is_down(&s.id))
            };
            let proxy = resizer(server)
                .or_else(|| resizer(&self.settings.home_server))
                .or_else(|| {
                    self.servers
                        .iter()
                        .find(|s| !s.is_jellyfin() && !plex::is_down(&s.id))
                });
            return Some(match proxy {
                Some(srv) => srv.image_url(path, w, h),
                None => plex::direct_image_url(path, 780),
            });
        }
        match self.servers.iter().find(|s| s.id == server) {
            Some(s) => Some(s.image_url(path, w, h)),
            None => self.server(server).map(|s| s.image_url(path, w, h)),
        }
    }

    pub fn image(&mut self, thumb: &Option<Thumb>, shape: Shape) -> Option<Arc<RenderImage>> {
        let t = thumb.as_ref()?;
        let size = self.size_for(shape);
        image_key(&mut self.image_key, t, size);
        if let Some(img) = self.lru.get(&self.image_key) {
            return Some(img);
        }
        let frame = self.lru.frame();
        if let Some(wanted) = self.requested.get_mut(&self.image_key) {
            *wanted = frame;
            return None;
        }
        let local = (shape != Shape::Backdrop)
            .then(|| self.downloads.as_ref()?.image(&t.server, &t.path))
            .flatten();
        if local.is_some() {
            self.image_key.push_str(LOCAL_IMAGE);
        }
        if let Some(wanted) = self.requested.get_mut(&self.image_key) {
            *wanted = frame;
            return None;
        }
        if self.failed.contains(&self.image_key) {
            return None;
        }
        let url = match &local {
            Some(file) => file.to_string_lossy().into_owned(),
            // Plex's transcoder flattens alpha to black unless the path says clearLogo.
            None if shape == Shape::Logo && t.path.starts_with("http") => {
                plex::direct_image_url(&t.path, size.0)
            }
            None => {
                self.image_url(&t.server, &t.path, size)? + if t.blur { BLUR_PARAM } else { "" }
            }
        };
        let key = self.image_key.clone();
        self.requested.insert(key.clone(), frame);
        let job = images::Job {
            key,
            url,
            local: local.is_some(),
            size,
        };
        let requested = &self.requested;
        if let Some(dropped) = self.queue.push(job, |k| requested.get(k) == Some(&frame)) {
            self.requested.remove(&dropped.key);
        }
        None
    }

    // Fetches without a fade-in, so its arrival doesn't animate frames while it's off screen.
    pub fn prefetch(&mut self, thumb: &Option<Thumb>, shape: Shape) {
        if thumb.is_some() {
            self.image(thumb, shape);
            self.arrivals.remove(&self.image_key);
        }
    }

    pub fn image_alpha(&self) -> f32 {
        self.arrivals.get(&self.image_key).map_or(1., |t| {
            (t.elapsed().as_secs_f32() / IMAGE_FADE.as_secs_f32()).min(1.)
        })
    }

    pub fn trim_images(&mut self, full_player: bool, cx: &mut Context<Self>) {
        for img in self.lru.set_budget(if full_player {
            PLAYER_IMAGE_BUDGET
        } else {
            IMAGE_BUDGET
        }) {
            cx.drop_image(img, None);
        }
    }

    pub fn confirm_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete all Plaivo data?",
            Some("Signs you out of every server and service and deletes your settings, cache, logs and downloads, then restarts Plaivo. Downloads saved to a folder you picked are kept."),
            &["Delete and restart", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update(cx, |this, cx| match store::request_reset() {
                Ok(()) => cx.restart(),
                Err(e) => this.toast(format!("Couldn't delete Plaivo's data: {e}"), cx),
            })
            .ok();
        })
        .detach();
    }

    pub fn clear_image_cache(&mut self, cx: &mut Context<Self>) {
        for img in self.lru.drain() {
            cx.drop_image(img, None);
        }
        self.failed.clear();
        Self::detach(cx, store::clear_cache);
        self.toast("Image cache cleared", cx);
    }

    fn on_image(&mut self, key: String, img: Option<Arc<RenderImage>>, cx: &mut Context<Self>) {
        self.requested.remove(&key);
        match img {
            Some(img) => {
                let key = key
                    .strip_suffix(LOCAL_IMAGE)
                    .map_or(key.clone(), str::to_owned);
                self.arrivals.insert(key.clone(), Instant::now());
                for old in self.lru.insert(key, img) {
                    cx.drop_image(old, None);
                }
            }
            None => {
                self.failed.insert(key);
            }
        }
        self.check_splash(cx);
        cx.notify();
    }

    fn image_settled(&self, thumb: &Option<Thumb>, shape: Shape) -> bool {
        thumb.as_ref().is_none_or(|t| {
            let mut key = String::new();
            image_key(&mut key, t, self.size_for(shape));
            self.lru.contains(&key)
                || self.failed.contains(&key)
                || self.failed.contains(&(key + LOCAL_IMAGE))
        })
    }

    fn begin_splash(&mut self, cx: &mut Context<Self>) {
        self.splash = Some(Splash::default());
        self.nav = vec![Nav::Home];
        self.page = Page::Splash;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SPLASH_TIMEOUT).await;
            this.update(cx, |this, cx| this.finish_splash(cx)).ok();
        })
        .detach();
    }

    fn check_splash(&mut self, cx: &mut Context<Self>) {
        let Some(sp) = &self.splash else { return };
        if !sp.probes_done {
            return log::debug!("splash: waiting for server probes");
        }
        if self.home_server().is_some_and(|s| plex::is_down(&s.id)) {
            return self.finish_splash(cx);
        }
        if !self.home_ready {
            return log::debug!("splash: waiting for home");
        }
        let mut wanted: Vec<_> = self
            .hero
            .first()
            .map(|c| (c.thumb.clone(), Shape::Backdrop))
            .into_iter()
            .collect();
        if let Some(h) = self.hubs.first() {
            let shape = if h.wide { Shape::Wide } else { Shape::Poster };
            wanted.extend(
                h.items
                    .iter()
                    .take(FIRST_ROW_IMAGES)
                    .map(|c| (c.thumb.clone(), shape)),
            );
        }
        wanted.retain(|(thumb, shape)| !self.image_settled(thumb, *shape));
        if wanted.is_empty() {
            return self.finish_splash(cx);
        }
        log::debug!("splash: waiting for {} home image(s)", wanted.len());
        // Home isn't rendered behind the splash, so re-queue anything dropped since build_home (e.g. by discovery clearing `failed`).
        for (thumb, shape) in &wanted {
            self.image(thumb, *shape);
        }
    }

    fn finish_splash(&mut self, cx: &mut Context<Self>) {
        if self.splash.take().is_none() {
            return;
        }
        log::info!("startup complete, home ready: {}", self.home_ready);
        if self.page == Page::Player {
            return;
        }
        if self.servers.is_empty() {
            self.nav = vec![Nav::Servers];
            self.navigate(Nav::Servers, false, cx);
        } else if (self.home_error.is_some() || self.servers_error.is_some())
            && self.has_downloads()
        {
            self.navigate(Nav::Downloads(None), true, cx);
        } else {
            self.navigate(Nav::Home, false, cx);
        }
    }

    pub fn sign_in(&mut self, cx: &mut Context<Self>) {
        let gen_ = self.pin_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.auth_waiting = true;
        self.auth_error = None;
        let (c, pin_gen) = (self.client.clone(), self.pin_gen.clone());
        Self::bg(
            cx,
            move || -> R<Option<String>> {
                let pin = c.create_pin(true)?;
                log::info!(
                    "sign-in: PIN {} created, waiting for browser approval",
                    pin.id
                );
                open_url(&c.auth_url(&pin.code));
                poll_until(
                    &pin_gen,
                    gen_,
                    Duration::from_secs(1),
                    Instant::now() + Duration::from_secs(600),
                    || c.check_pin(pin.id).transpose(),
                )
                .transpose()
            },
            move |this, r, cx| {
                if this.pin_gen.load(Ordering::SeqCst) != gen_ {
                    return;
                }
                this.auth_waiting = false;
                match r {
                    Ok(Some(token)) => {
                        log::info!("sign-in: approved");
                        this.account.token = Some(token);
                        this.account.save();
                        this.begin_splash(cx);
                        this.discover(cx);
                    }
                    Ok(None) => {}
                    Err(e) => this.auth_error = Some(format!("Sign-in failed: {e}").into()),
                }
            },
        );
        cx.notify();
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        self.pin_gen.fetch_add(1, Ordering::SeqCst);
        self.auth_waiting = false;
        if !self.servers.is_empty()
            && let Some(nav) = self.nav.last().cloned()
        {
            self.navigate(nav, false, cx);
        }
        cx.notify();
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        log::info!("signing out and clearing caches");
        self.stop_playback(cx);
        self.watch_leave(cx);
        plaivo_core::sign_out(&mut self.account, &mut self.settings);
        self.watch.account_name = None;
        self.probe = None;
        self.servers_loading = false;
        self.servers.clear();
        self.sections.clear();
        self.nav.clear();
        self.hero.clear();
        self.hubs.clear();
        self.suggestions.clear();
        self.results.clear();
        self.library = None;
        self.library_cache.clear();
        self.lib_hubs = None;
        self.lib_hubs_for = Default::default();
        self.hub_list = None;
        self.detail = None;
        self.home_ready = false;
        for img in self.lru.drain() {
            cx.drop_image(img, None);
        }
        self.servers_changed(cx);
        if self.page != Page::Auth {
            self.nav = vec![Nav::Home];
            self.navigate(Nav::Home, false, cx);
        }
    }

    pub fn servers_changed(&mut self, cx: &mut Context<Self>) {
        self.set_servers(cx);
        let mut known: HashSet<String> = self.servers.iter().map(|s| s.id.clone()).collect();
        if self.account.token.is_some() {
            known.insert(DISCOVER_ID.to_owned());
        }
        if self
            .player
            .server
            .as_ref()
            .is_some_and(|s| !known.contains(&s.id))
        {
            self.stop_playback(cx);
        }
        let stale_page = match self.page {
            Page::Library => match self.nav.last() {
                Some(Nav::Library(server, _) | Nav::Collection(server, _, _)) => {
                    !known.contains(server)
                }
                Some(Nav::Watchlist) => !known.contains(DISCOVER_ID),
                _ => true,
            },
            Page::HubList => self
                .hub_list
                .as_ref()
                .is_none_or(|l| !known.contains(&l.server)),
            Page::Detail => !known.contains(&self.detail_id.0),
            _ => false,
        };
        self.nav.retain(|n| match n {
            Nav::Library(server, _)
            | Nav::Collection(server, _, _)
            | Nav::Person(server, _, _)
            | Nav::Hub(server, _, _, _)
            | Nav::Detail(server, _, _) => known.contains(server),
            Nav::Watchlist => known.contains(DISCOVER_ID),
            _ => true,
        });
        self.library_cache.retain(|l| known.contains(&l.key.server));
        if self
            .library
            .as_ref()
            .is_some_and(|l| !known.contains(&l.key.server))
        {
            self.library = None;
        }
        if !known.contains(&self.lib_hubs_for.0) {
            self.lib_hubs = None;
            self.lib_hubs_for = Default::default();
        }
        if self
            .hub_list
            .as_ref()
            .is_some_and(|l| !known.contains(&l.server))
        {
            self.hub_list = None;
        }
        if !known.contains(&self.detail_id.0) {
            self.detail = None;
            self.detail_id = Default::default();
        }
        if self.servers.is_empty() && self.account.token.is_none() {
            self.nav.clear();
            self.page = Page::Auth;
        } else if stale_page {
            if self.nav.is_empty() {
                self.nav.push(Nav::Home);
            }
            let nav = self.nav.last().cloned().unwrap_or(Nav::Home);
            self.navigate(nav, false, cx);
        }
        cx.notify();
    }

    fn discover(&mut self, cx: &mut Context<Self>) {
        let Some(token) = self.account.token.clone() else {
            if let Some(sp) = &mut self.splash {
                sp.probes_done = true;
            }
            return;
        };
        self.servers_loading = true;
        let c = self.client.clone();
        let started = token.clone();
        Self::bg(
            cx,
            move || c.servers(&token).map(|r| (r, token)),
            move |this, r, cx| {
                if this.account.token.as_ref() != Some(&started) {
                    return;
                }
                match r {
                    Ok((resources, token)) => this.probe_servers(resources, token, cx),
                    Err(e) => this.discovery_done(Vec::new(), Some(e), cx),
                }
            },
        );
    }

    fn probe_servers(
        &mut self,
        resources: Vec<plex::Resource>,
        token: String,
        cx: &mut Context<Self>,
    ) {
        log::info!(
            "discovery: {} server(s) on account: {}",
            resources.len(),
            resources
                .iter()
                .map(|r| r.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if resources.is_empty() {
            return self.discovery_done(
                Vec::new(),
                Some("No Plex Media Servers found on this account.".into()),
                cx,
            );
        }
        let total = resources.len();
        if let Some(sp) = &mut self.splash {
            sp.servers = resources
                .iter()
                .map(|r| (r.client_identifier.clone(), r.name.clone(), Status::Pending))
                .collect();
        }
        self.probe = Some((Vec::new(), total, false));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PROBE_WAIT).await;
            this.update(cx, |this, cx| this.probe_deadline(cx)).ok();
        })
        .detach();
        for res in resources {
            let (c, t, started) = (self.client.clone(), token.clone(), token.clone());
            let id = res.client_identifier.clone();
            let last = self
                .account
                .servers
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.uri.clone());
            Self::bg(
                cx,
                move || c.connect(&res, &t, last.as_deref()),
                move |this, r, cx| {
                    if this.account.token.as_ref() != Some(&started) {
                        return;
                    }
                    match &r {
                        Ok(srv) => log::info!("discovery: {} reachable at {}", srv.name, srv.uri),
                        Err(e) => log::warn!("discovery: {e}"),
                    }
                    let ok = r.is_ok();
                    if let Some(row) = this
                        .splash
                        .as_mut()
                        .and_then(|sp| sp.servers.iter_mut().find(|x| x.0 == id))
                    {
                        row.2 = if ok { Status::Ok } else { Status::Error };
                    }
                    let previous = this.account.servers.iter().find(|s| s.id == id).cloned();
                    let Some(probe) = &mut this.probe else { return };
                    probe.2 |= ok;
                    probe.0.extend(r.ok().or(previous));
                    probe.1 -= 1;
                    if probe.1 == 0 {
                        let (mut list, _, any_ok) = this.probe.take().unwrap();
                        list.sort_by_key(|x| !x.owned);
                        this.discovery_done(if any_ok { list } else { Vec::new() }, None, cx);
                    } else {
                        this.check_splash(cx);
                    }
                },
            );
        }
    }

    fn probe_deadline(&mut self, cx: &mut Context<Self>) {
        let home = self.home_server().map(|s| s.id);
        let Some(sp) = self.splash.as_mut().filter(|sp| !sp.probes_done) else {
            return;
        };
        for (id, name, status) in &sp.servers {
            if *status == Status::Pending && home.as_ref() != Some(id) {
                log::info!("discovery: {name} has not answered yet, hiding it until it does");
                plex::set_down(id, true);
            }
        }
        sp.probes_done = true;
        self.check_splash(cx);
        cx.notify();
    }

    fn discovery_done(&mut self, list: Vec<Server>, error: Option<String>, cx: &mut Context<Self>) {
        self.servers_loading = false;
        if let Some(sp) = &mut self.splash {
            sp.probes_done = true;
        }
        if list.is_empty() {
            let e = error.unwrap_or_else(|| "None of your Plex servers are reachable.".into());
            if self.servers.is_empty() {
                self.servers_error = Some(e.into());
                self.finish_splash(cx);
            } else {
                log::warn!("server discovery failed, keeping cached connections: {e}");
                self.servers_error = Some(e.into());
                self.check_splash(cx);
            }
            return;
        }
        self.servers_error = None;
        self.failed.clear();
        if list != self.account.servers {
            self.account.servers = list;
            self.account.save();
            self.set_servers(cx);
        }
        self.check_splash(cx);
    }

    pub fn set_servers(&mut self, cx: &mut Context<Self>) {
        let previous_home = self.home_server();
        self.servers = self.account.all_servers();
        let list = self.servers.clone();
        self.with_downloads(cx, move |d| d.set_servers(&list));
        if self.server(&self.settings.home_server).is_none()
            && let Some(first) = self.servers.first()
        {
            self.settings.home_server = first.id.clone();
            self.save_settings_soon(cx);
        }
        self.sections
            .retain(|sec| self.servers.iter().any(|s| s.id == sec.server));
        self.load_sections(cx);
        let home = self.home_server();
        if previous_home.as_ref().map(|s| &s.id) != home.as_ref().map(|s| &s.id) {
            self.home_ready = false;
        }
        if previous_home != home || !self.home_ready {
            self.load_home(cx);
        }
    }

    pub fn pick_home_server(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.server(id).is_none() {
            return;
        }
        self.settings.home_server = id.to_owned();
        self.save_settings_soon(cx);
        self.home_ready = false;
        self.load_home(cx);
        if self.page == Page::Servers {
            self.nav = vec![Nav::Home];
            self.navigate(Nav::Home, false, cx);
        }
        cx.notify();
    }

    fn load_sections(&mut self, cx: &mut Context<Self>) {
        for srv in self.servers.clone() {
            let id = srv.id.clone();
            self.load(
                cx,
                move |c| srv.sections(c),
                move |this, r, _| match r {
                    Ok(dirs) => {
                        this.sections.retain(|x| x.server != id);
                        this.sections.extend(dirs.into_iter().map(|d| SectionInfo {
                            id: library_key(&id, &d.key),
                            server: id.clone(),
                            key: d.key,
                            title: d.title.into(),
                            kind: d.kind,
                        }));
                    }
                    Err(e) => log::warn!("sections for {id}: {e}"),
                },
            );
        }
    }

    pub fn sections_in_order(&self, hidden: bool) -> Vec<(&str, Vec<&SectionInfo>)> {
        let home = self
            .servers
            .iter()
            .find(|s| s.id == self.settings.home_server)
            .or(self.servers.first())
            .map(|s| s.id.as_str());
        let mut order: Vec<&Server> = self.servers.iter().collect();
        order.sort_by_key(|s| Some(s.id.as_str()) != home);
        order
            .into_iter()
            .map(|srv| {
                let shown = hidden || !plex::is_down(&srv.id);
                let mut rows: Vec<&SectionInfo> = self
                    .sections
                    .iter()
                    .filter(|x| {
                        shown
                            && x.server == srv.id
                            && (hidden || !self.settings.library_hidden(&x.id))
                    })
                    .collect();
                rows.sort_by_key(|x| self.settings.library_rank(&x.id));
                (srv.name.as_str(), rows)
            })
            .filter(|(_, rows)| !rows.is_empty())
            .collect()
    }

    pub fn navigate(&mut self, nav: Nav, push: bool, cx: &mut Context<Self>) {
        log::debug!("navigate {nav:?}");
        if push && self.nav.last() != Some(&nav) {
            self.nav.push(nav.clone());
        }
        self.sort_open = false;
        self.add_server.leave(cx);
        if nav != Nav::Settings(Some(Category::Services)) {
            self.services.leave();
        }
        match nav {
            Nav::Home => self.page = Page::Home,
            Nav::Servers => self.page = Page::Servers,
            Nav::Search => self.page = Page::Search,
            Nav::Settings(category) => {
                if self.settings_page != category || self.page != Page::Settings {
                    self.settings_scroll = ScrollHandle::new();
                }
                self.page = Page::Settings;
                self.settings_page = category;
            }
            Nav::Licenses(license) => {
                self.license_scroll = ScrollHandle::new();
                self.page = Page::Licenses;
                self.license = license;
            }
            Nav::Logs => {
                self.page = Page::Logs;
                self.log_selection = None;
                self.logs_scroll.scroll_to_item(
                    crate::logging::len().saturating_sub(1),
                    gpui::ScrollStrategy::Top,
                );
            }
            Nav::AddServer => {
                self.page = Page::AddServer;
                self.add_server.reset(cx);
            }
            Nav::Downloads(view) => {
                self.page = Page::Downloads;
                self.offline_view = view;
            }
            Nav::Watch => self.page = Page::Watch,
            Nav::Watchlist => {
                self.page = Page::Library;
                let section = if self.watchlist_shows { "2" } else { "1" };
                let sort = self
                    .library
                    .iter()
                    .chain(&self.library_cache)
                    .find(|l| l.key.listing == Listing::Watchlist && l.key.section == section)
                    .map(|l| l.key.sort.clone());
                self.show_library(
                    LibKey {
                        server: DISCOVER_ID.to_owned(),
                        section: section.to_owned(),
                        sort: sort.unwrap_or_else(|| "addedAt:desc".to_owned()),
                        listing: Listing::Watchlist,
                    },
                    SharedString::default(),
                    cx,
                );
            }
            Nav::Library(server, section) => {
                self.page = Page::Library;
                match self.lib_tab {
                    LibTab::Recommended => self.load_lib_hubs(server, section, cx),
                    LibTab::Browse => {
                        let sort = self
                            .library
                            .iter()
                            .chain(&self.library_cache)
                            .find(|l| {
                                l.key.listing == Listing::Items
                                    && l.key.server == server
                                    && l.key.section == section
                            })
                            .map(|l| l.key.sort.clone());
                        self.show_library(
                            LibKey {
                                server,
                                section,
                                sort: sort.unwrap_or_else(|| SORTS[0].0.to_owned()),
                                listing: Listing::Items,
                            },
                            SharedString::default(),
                            cx,
                        );
                    }
                    LibTab::Collections => self.show_library(
                        LibKey {
                            server,
                            section,
                            sort: String::new(),
                            listing: Listing::Collections,
                        },
                        SharedString::default(),
                        cx,
                    ),
                }
            }
            Nav::Collection(server, key, title) => {
                self.page = Page::Library;
                self.show_library(
                    LibKey {
                        server,
                        section: key,
                        sort: String::new(),
                        listing: Listing::Collection,
                    },
                    title.into(),
                    cx,
                );
            }
            Nav::Person(server, id, name) => {
                self.page = Page::Detail;
                self.load_person(server, id, name, cx);
            }
            Nav::Hub(server, key, title, resume) => {
                self.page = Page::HubList;
                if self
                    .hub_list
                    .as_ref()
                    .is_none_or(|l| l.server != server || l.key != key)
                {
                    self.hub_list = Some(HubList {
                        server,
                        sort: initial_sort(&key),
                        key,
                        title: title.into(),
                        resume,
                        items: Vec::new(),
                        cards: Vec::new(),
                        loaded: false,
                        scroll: UniformListScrollHandle::new(),
                    });
                }
                self.load_hub_list(cx);
            }
            Nav::Detail(server, key, season) => {
                self.page = Page::Detail;
                self.load_detail(server, key, season, cx);
            }
        }
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        if self.nav.len() > 1 {
            self.nav.pop();
        }
        if let Some(nav) = self.nav.last().cloned() {
            self.navigate(nav, false, cx);
        }
    }

    pub fn open(&mut self, card: &Card, cx: &mut Context<Self>) {
        if card.key.is_empty() {
            return;
        }
        if card.resume {
            return self.play(card.server.to_string(), card.key.to_string(), cx);
        }
        if self.page == Page::Downloads {
            return self.navigate(
                Nav::Downloads(Some((card.server.to_string(), card.key.to_string()))),
                true,
                cx,
            );
        }
        let q = self.search.read(cx).text().trim().to_owned();
        if self.page == Page::Search && !q.is_empty() {
            self.settings.remember_search(&q);
            self.save_settings_soon(cx);
        }
        let nav = match card.kind.as_ref() {
            "season" if !card.parent.is_empty() => Nav::Detail(
                card.server.to_string(),
                card.parent.to_string(),
                Some(card.key.to_string()),
            ),
            "collection" => Nav::Collection(
                card.server.to_string(),
                card.key.to_string(),
                card.title.to_string(),
            ),
            _ => Nav::Detail(card.server.to_string(), card.key.to_string(), None),
        };
        self.navigate(nav, true, cx);
    }

    pub fn refresh_home(&mut self, cx: &mut Context<Self>) {
        self.load_sections(cx);
        self.load_home(cx);
    }

    pub fn load_home(&mut self, cx: &mut Context<Self>) {
        let Some(srv) = self.home_server() else {
            return;
        };
        self.home_error = None;
        self.failed.clear();
        if !self.home_ready {
            self.hero.clear();
            self.hubs.clear();
        }
        let id = srv.id.clone();
        let home = id.clone();
        self.load(
            cx,
            move |c| srv.hubs(c),
            move |this, r, cx| {
                if this.home_server().map(|h| h.id) != Some(id.clone()) {
                    return;
                }
                match r {
                    Ok(hubs) => this.build_home(&id, hubs, cx),
                    Err(e) if !this.home_ready => {
                        let name = this.server(&id).map(|x| x.name).unwrap_or_default();
                        this.home_error = Some(format!("Could not reach {name}: {e}").into());
                        this.finish_splash(cx);
                    }
                    Err(e) => log::warn!("home refresh failed: {e}"),
                }
            },
        );
        for other in self.servers.iter().filter(|s| s.id != home).cloned() {
            let sid = other.id.clone();
            self.load(
                cx,
                move |c| other.continue_items(c),
                move |this, r, _| {
                    match r {
                        Ok(items) => this.remote_continue.insert(sid.clone(), items),
                        Err(_) => this.remote_continue.remove(&sid),
                    };
                    if this.home_ready {
                        this.apply_continue();
                    }
                },
            );
        }
    }

    fn apply_continue(&mut self) {
        let Some(home) = self.home_server() else {
            return;
        };
        let row = plex::continue_row(
            &home.id,
            &self.home_continue,
            self.servers
                .iter()
                .filter_map(|s| Some((s.id.as_str(), &self.remote_continue.get(&s.id)?[..]))),
            &card_prefs(),
        );
        let old = self
            .hubs
            .first()
            .is_some_and(|h| h.resume)
            .then(|| self.hubs.remove(0));
        let Some(row) = row else {
            return;
        };
        let (scroll, slide) = old.map(|o| (o.scroll, o.slide)).unwrap_or_default();
        self.hubs.insert(
            0,
            Hub {
                title: row.title.to_owned().into(),
                key: row.key.to_owned().into(),
                server: home.id.clone().into(),
                resume: true,
                wide: row.wide,
                items: row
                    .items
                    .iter()
                    .map(|&(s, it)| Card {
                        resume: true,
                        ..card_for(it, row.wide, s)
                    })
                    .collect(),
                scroll,
                slide,
            },
        );
    }

    fn build_home(&mut self, server: &str, hubs: Vec<plex::Hub>, cx: &mut Context<Self>) {
        self.hero = plex::pick_hero(&hubs)
            .into_iter()
            .map(|it| hero_card(it, server))
            .collect();
        self.hero_index = self.hero_index.min(self.hero.len().saturating_sub(1));
        self.hubs = hub_rows(&hubs, server, &self.hubs);
        let mut seen = HashSet::new();
        self.suggestions = hubs
            .iter()
            .filter(|h| !h.is_continue())
            .flat_map(|h| h.metadata.iter())
            .filter(|it| {
                matches!(it.kind.as_str(), "movie" | "show") && seen.insert(it.rating_key.clone())
            })
            .take(SUGGESTIONS)
            .map(|it| card_for(it, false, server))
            .collect();
        self.home_continue = hubs
            .into_iter()
            .find(plex::Hub::is_continue)
            .unwrap_or_default();
        self.apply_continue();
        self.home_ready = true;
        for c in self.hero.clone().iter().take(2) {
            self.image(&c.thumb, Shape::Backdrop);
        }
        if let Some((shape, firsts)) = self.hubs.first().map(|h| {
            (
                if h.wide { Shape::Wide } else { Shape::Poster },
                h.items
                    .iter()
                    .take(FIRST_ROW_IMAGES)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        }) {
            for c in firsts {
                self.image(&c.thumb, shape);
            }
        }
        if self.hero_task.is_none() {
            self.start_hero_rotation(cx);
        }
        self.check_splash(cx);
    }

    pub fn start_hero_rotation(&mut self, cx: &mut Context<Self>) {
        self.hero_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(8)).await;
                let alive = this.update(cx, |this, cx| {
                    let hero_visible = f32::from(-this.home_scroll.offset().y) < this.view.hero_h;
                    if this.page == Page::Home && this.hero.len() > 1 && hero_visible {
                        this.view.hero_prev = Some((this.hero_index, Instant::now()));
                        this.hero_index = (this.hero_index + 1) % this.hero.len();
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    pub fn step_hero(&mut self, d: isize, cx: &mut Context<Self>) {
        let n = self.hero.len() as isize;
        if n > 1 {
            self.view.hero_prev = Some((self.hero_index, Instant::now()));
            self.hero_index = ((self.hero_index as isize + d).rem_euclid(n)) as usize;
            self.start_hero_rotation(cx);
            cx.notify();
        }
    }

    pub fn set_watchlist_tab(&mut self, shows: bool, cx: &mut Context<Self>) {
        self.watchlist_shows = shows;
        self.navigate(Nav::Watchlist, false, cx);
    }

    pub fn set_lib_tab(&mut self, tab: LibTab, cx: &mut Context<Self>) {
        self.lib_tab = tab;
        if let Some(nav @ Nav::Library(..)) = self.nav.last().cloned() {
            self.navigate(nav, false, cx);
        }
    }

    fn load_lib_hubs(&mut self, server: String, section: String, cx: &mut Context<Self>) {
        let Some(srv) = self.server(&server) else {
            return;
        };
        let kind = self
            .sections
            .iter()
            .find(|s| s.server == server && s.key == section)
            .map(|s| s.kind.clone())
            .unwrap_or_default();
        let key = (server, section.clone());
        if self.lib_hubs_for != key {
            self.lib_hubs = None;
            self.lib_hubs_scroll = ScrollHandle::new();
            self.lib_hubs_for = key.clone();
        }
        self.load(
            cx,
            move |c| srv.section_hubs(c, &section, &kind),
            move |this, r, cx| {
                if this.lib_hubs_for != key {
                    return;
                }
                match r {
                    Ok(hubs) => {
                        this.lib_hubs = Some(hub_rows(
                            &hubs,
                            &key.0,
                            this.lib_hubs.as_deref().unwrap_or_default(),
                        ))
                    }
                    Err(e) => {
                        this.lib_hubs.get_or_insert_default();
                        this.toast(format!("Could not load recommendations: {e}"), cx);
                    }
                }
            },
        );
    }

    fn load_hub_list(&mut self, cx: &mut Context<Self>) {
        let Some(l) = &self.hub_list else { return };
        let Some(srv) = self.server(&l.server) else {
            return;
        };
        let (server, key) = (l.server.clone(), l.key.clone());
        let k = key.clone();
        self.load(
            cx,
            move |c| srv.hub_items(c, &k),
            move |this, r, cx| {
                let Some(l) = this
                    .hub_list
                    .as_mut()
                    .filter(|l| l.server == server && l.key == key)
                else {
                    return;
                };
                l.loaded = true;
                match r {
                    Ok(items) => {
                        l.items = items;
                        l.rebuild();
                    }
                    Err(e) => this.toast(format!("Could not load list: {e}"), cx),
                }
            },
        );
    }

    pub fn set_hub_sort(&mut self, sort: &str, cx: &mut Context<Self>) {
        self.sort_open = false;
        if let Some(l) = &mut self.hub_list {
            l.sort = sort.to_owned();
            l.rebuild();
        }
        cx.notify();
    }

    pub fn set_sort(&mut self, sort: &str, cx: &mut Context<Self>) {
        self.sort_open = false;
        if let Some(mut key) = self.library.as_ref().map(|l| l.key.clone()) {
            key.sort = sort.to_owned();
            self.show_library(key, SharedString::default(), cx);
        }
    }

    fn show_library(&mut self, key: LibKey, title: SharedString, cx: &mut Context<Self>) {
        if self.library.as_ref().is_some_and(|l| l.key == key) {
            return self.refresh_library_page(0, cx);
        }
        if let Some(cur) = self.library.take() {
            self.library_cache.retain(|l| l.key != cur.key);
            self.library_cache.push(cur);
            if self.library_cache.len() > LIBRARY_CACHE {
                self.library_cache.remove(0);
            }
        }
        self.library = Some(match self.library_cache.iter().position(|l| l.key == key) {
            Some(i) => self.library_cache.remove(i),
            None => Library {
                key,
                title,
                items: vec![None; 48],
                pages: HashSet::new(),
                scroll: UniformListScrollHandle::new(),
            },
        });
        self.refresh_library_page(0, cx);
    }

    pub fn ensure_library_rows(&mut self, start: usize, end: usize, cx: &mut Context<Self>) {
        let Some(lib) = &self.library else { return };
        if lib.pages.is_empty() || end <= start {
            return;
        }
        let pages: Vec<usize> = (start / PAGE..=(end - 1) / PAGE)
            .filter(|p| {
                !lib.pages.contains(p)
                    && p * PAGE < lib.items.len()
                    && lib.items[p * PAGE..(p * PAGE + PAGE).min(lib.items.len())]
                        .iter()
                        .any(Option::is_none)
            })
            .collect();
        for p in pages {
            self.refresh_library_page(p, cx);
        }
    }

    fn refresh_library_page(&mut self, page: usize, cx: &mut Context<Self>) {
        let Some(lib) = &mut self.library else { return };
        let first_load = lib.pages.is_empty();
        lib.pages.insert(page);
        let key = lib.key.clone();
        let Some(srv) = self.server(&key.server) else {
            return;
        };
        let k = key.clone();
        let summaries = self.settings.view_mode == ViewMode::List;
        let fetch = move |c: &Ctx| match k.listing {
            Listing::Items => {
                srv.section_items(c, &k.section, page * PAGE, PAGE, &k.sort, summaries)
            }
            Listing::Collections => srv.collections(c, &k.section, page * PAGE, PAGE),
            Listing::Collection => {
                srv.collection_items(c, &k.section, page * PAGE, PAGE, summaries)
            }
            Listing::Watchlist => srv.watchlist(c, &k.section, page * PAGE, PAGE, &k.sort),
        };
        self.load(cx, fetch, move |this, r, cx| {
            let current = this.library.as_ref().is_some_and(|l| l.key == key);
            let Some(lib) = this
                .library
                .as_mut()
                .filter(|l| l.key == key)
                .or_else(|| this.library_cache.iter_mut().find(|l| l.key == key))
            else {
                return;
            };
            match r {
                Ok((items, total)) => {
                    if page == 0 && (first_load || lib.items.len() != total) {
                        lib.items.resize(total, None);
                    }
                    for (i, it) in items.iter().enumerate() {
                        if let Some(slot) = lib.items.get_mut(page * PAGE + i) {
                            let meta = [
                                it.content_rating.clone(),
                                it.year.map(|y| y.to_string()).unwrap_or_default(),
                                it.duration.map(plex::fmt_duration).unwrap_or_default(),
                                it.audience_rating
                                    .map(|r| format!("★ {r:.1}"))
                                    .unwrap_or_default(),
                            ];
                            *slot = Some(Card {
                                summary: it.summary.clone().into(),
                                meta: join(&meta).into(),
                                ..card_for(it, false, &key.server)
                            });
                        }
                    }
                }
                Err(e) => {
                    lib.pages.remove(&page);
                    if current {
                        this.toast(format!("Could not load library: {e}"), cx);
                    }
                }
            }
        });
    }

    fn search_debounced(&mut self, q: String, cx: &mut Context<Self>) {
        self.search_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            this.update(cx, |this, cx| this.search(q, cx)).ok();
        }));
    }

    pub fn search(&mut self, q: String, cx: &mut Context<Self>) {
        self.search_gen += 1;
        let gen_ = self.search_gen;
        let q = q.trim().to_owned();
        if q.chars().count() < 2 {
            self.results.clear();
            self.searching = false;
            cx.notify();
            return;
        }
        self.searching = true;
        let servers: Vec<Server> = self
            .servers
            .iter()
            .filter(|s| !plex::is_down(&s.id))
            .cloned()
            .collect();
        let (c, token) = (
            self.client.clone(),
            self.account.token.clone().unwrap_or_default(),
        );
        Self::bg(
            cx,
            move || {
                let discover = {
                    let (c, q) = (c.clone(), q.clone());
                    std::thread::spawn(move || {
                        if token.is_empty() {
                            Ok(Vec::new())
                        } else {
                            c.discover_search(&q, &token)
                        }
                    })
                };
                let local: Vec<_> = servers
                    .into_iter()
                    .map(|srv| {
                        let (c, q) = (c.clone(), q.clone());
                        std::thread::spawn(move || {
                            (srv.id.clone(), srv.search(&Ctx::new(c, Source::Live), &q))
                        })
                    })
                    .collect();
                let local: Vec<_> = local.into_iter().filter_map(|h| h.join().ok()).collect();
                (
                    q,
                    local,
                    discover
                        .join()
                        .unwrap_or_else(|_| Err("search thread panicked".into())),
                )
            },
            move |this, (q, local, discover), cx| {
                if this.search_gen != gen_ {
                    return;
                }
                let mut errors = Vec::new();
                let local: Vec<(String, Vec<(Item, f64)>)> = local
                    .into_iter()
                    .filter_map(|(srv, r)| {
                        r.map_err(|e| errors.push(e)).ok().map(|hits| (srv, hits))
                    })
                    .collect();
                let home = this.home_server().map(|h| h.id).unwrap_or_default();
                this.results = rank_search(&q, &home, local, discover.unwrap_or_default())
                    .iter()
                    .map(|(server, it)| {
                        let mut c = card_for(it, false, server);
                        if server == DISCOVER_ID {
                            c.subtitle =
                                join(&[c.subtitle.to_string(), "Not in your library".into()])
                                    .into();
                        }
                        c
                    })
                    .collect();
                this.searching = false;
                if this.results.is_empty()
                    && let Some(e) = errors.first()
                {
                    this.toast(format!("Search failed: {e}"), cx);
                }
            },
        );
        cx.notify();
    }

    fn load_detail(
        &mut self,
        server: String,
        key: String,
        season: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(srv) = self.server(&server) else {
            return;
        };
        if self.detail_id != (server.clone(), key.clone()) {
            self.clear_detail();
            self.owned_check = (server == DISCOVER_ID).then(|| key.clone());
        }
        if season.is_some() {
            self.season = season;
        }
        self.detail_id = (server.clone(), key.clone());
        let wanted = self.detail_id.clone();
        self.load(
            cx,
            move |c| -> R<(Item, Vec<Item>)> {
                let it = srv.metadata(c, &key)?;
                let seasons = if it.kind == "show" {
                    srv.children(c, &key)?
                } else {
                    Vec::new()
                };
                Ok((it, seasons))
            },
            move |this, r, cx| {
                if this.detail_id != wanted {
                    return;
                }
                match r {
                    Ok((it, seasons)) if wanted.1 == it.rating_key => {
                        this.apply_detail(&server, it, seasons, cx)
                    }
                    Ok(_) => {}
                    Err(e) => this.toast(format!("Could not load details: {e}"), cx),
                }
            },
        );
    }

    fn clear_detail(&mut self) {
        self.detail = None;
        self.seasons.clear();
        self.season_items.clear();
        self.episodes.clear();
        self.episode_items.clear();
        self.season = None;
        self.detail_scroll = ScrollHandle::new();
        self.detail_shelves.clear();
        self.summary_expanded = false;
        self.owned_check = None;
    }

    fn load_person(&mut self, server: String, id: String, name: String, cx: &mut Context<Self>) {
        let Some(srv) = self.server(&server) else {
            return;
        };
        let key = format!("person:{id}");
        if self.detail_id != (server.clone(), key.clone()) {
            self.clear_detail();
        }
        self.detail_id = (server.clone(), key);
        let wanted = self.detail_id.clone();
        let discover = self.server(DISCOVER_ID);
        let servers = if server == DISCOVER_ID {
            self.live_servers()
        } else {
            Vec::new()
        };
        let who = name.clone();
        self.load(
            cx,
            move |c| plex::person_page(&srv, c, &id, &who, discover.as_ref(), &servers),
            move |this, r, cx| {
                if this.detail_id != wanted {
                    return;
                }
                match r {
                    Ok((info, items)) => this.apply_person(&wanted.0, &name, info, items),
                    Err(e) => this.toast(format!("Could not load {name}: {e}"), cx),
                }
            },
        );
    }

    fn apply_person(
        &mut self,
        server: &str,
        name: &str,
        info: plex::PersonInfo,
        items: Vec<(String, Item)>,
    ) {
        let art = items
            .iter()
            .map(|(s, it)| (s.as_str(), it))
            .chain(info.known_for.iter().map(|it| (DISCOVER_ID, it)))
            .map(|(s, it)| (s, first(&[&it.art, &it.grandparent_art])))
            .find(|(_, a)| !a.is_empty())
            .and_then(|(s, a)| thumb(s, a));
        let known_for: Vec<Card> = info
            .known_for
            .iter()
            .map(|it| card_for(it, false, DISCOVER_ID))
            .collect();
        let library = items.iter().map(|(s, it)| card_for(it, false, s)).collect();
        let related = [
            ("Media in Library".into(), library),
            ("Known For".into(), known_for),
        ]
        .into_iter()
        .filter(|(_, cards)| !cards.is_empty())
        .collect();
        let info_rows = [
            ("Born", info.born),
            ("Died", info.died),
            ("Birthplace", info.birthplace),
        ]
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        self.detail = Some(DetailView {
            server: server.to_owned().into(),
            title: name.to_owned().into(),
            summary: info.summary.into(),
            art,
            photo: thumb(server, &info.thumb),
            info: info_rows,
            related,
            item: Item {
                kind: "person".into(),
                ..Default::default()
            },
            ..Default::default()
        });
    }

    fn apply_detail(&mut self, server: &str, it: Item, seasons: Vec<Item>, cx: &mut Context<Self>) {
        let summary = plaivo_core::detail::summary(&it, server != DISCOVER_ID);
        let extras = it
            .extras()
            .iter()
            .map(|x| Card {
                resume: true,
                subtitle: x
                    .duration
                    .map(plex::fmt_duration)
                    .unwrap_or_default()
                    .into(),
                ..card_for(x, true, server)
            })
            .collect();
        let watchlisted = self
            .detail
            .as_ref()
            .filter(|d| d.item.rating_key == it.rating_key)
            .and_then(|d| d.watchlisted);
        let related = self
            .detail
            .as_mut()
            .filter(|d| d.item.rating_key == it.rating_key)
            .map(|d| std::mem::take(&mut d.related))
            .unwrap_or_default();
        let fresh = related.is_empty();
        self.detail = Some(DetailView {
            logo: thumb(server, it.logo()),
            ratings: it
                .rating_labels()
                .into_iter()
                .map(|(s, v)| (s, v.into()))
                .collect(),
            media_line: it.media_line(0).into(),
            info: summary
                .info
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
            cast: it
                .role
                .iter()
                .take(40)
                .map(|p| CastMember {
                    id: p.id.clone(),
                    name: p.tag.clone().into(),
                    role: p.role.clone().into(),
                    thumb: thumb(server, &p.thumb),
                })
                .collect(),
            extras,
            related,
            watchlisted,
            photo: None,
            server: server.to_owned().into(),
            eyebrow: summary.eyebrow.into(),
            title: it.title.clone().into(),
            chips: summary.chips.into_iter().map(Into::into).collect(),
            genres: it
                .genre
                .iter()
                .take(4)
                .map(|t| t.tag.clone().into())
                .collect(),
            summary: it.summary.clone().into(),
            play_label: summary.play_label.into(),
            play_key: summary.play_key.into(),
            progress: summary.progress,
            art: thumb(server, first(&[&it.art, &it.grandparent_art])),
            is_show: it.kind == "show",
            item: it.clone(),
        });
        if fresh {
            self.load_related(server, &it, cx);
        }
        if it.kind == "show" {
            let pick = plaivo_core::detail::pick_season(&it, &seasons, self.season.as_deref());
            self.season_items = seasons;
            if let Some(k) = pick {
                self.select_season(k, cx);
            }
        }
    }

    pub fn select_season(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(srv) = self.server(&self.detail_id.0) else {
            return;
        };
        self.season = Some(key.clone());
        self.seasons = self
            .season_items
            .iter()
            .map(|x| Choice {
                id: x.rating_key.clone().into(),
                label: x.title.clone().into(),
                selected: x.rating_key == key,
                ..Default::default()
            })
            .collect();
        let server = srv.id.clone();
        let k = key.clone();
        self.load(
            cx,
            move |c| srv.children(c, &k),
            move |this, r, cx| {
                if this.season.as_deref() != Some(key.as_str()) || this.detail_id.0 != server {
                    return;
                }
                match r {
                    Ok(eps) => {
                        this.episodes = eps.iter().map(|e| episode_card(e, &server)).collect();
                        this.episode_items = eps;
                    }
                    Err(e) => this.toast(format!("Could not load episodes: {e}"), cx),
                }
            },
        );
        cx.notify();
    }

    pub fn cards_changed(&mut self, cx: &mut Context<Self>) {
        set_card_prefs(&self.settings);
        self.save_settings_soon(cx);
        self.library = None;
        self.library_cache.clear();
        self.lib_hubs = None;
        self.lib_hubs_for = Default::default();
        self.hub_list = None;
        self.load_home(cx);
        cx.notify();
    }

    fn live_servers(&self) -> Vec<Server> {
        let home = self.home_server().map(|h| h.id).unwrap_or_default();
        let mut servers: Vec<Server> = self
            .servers
            .iter()
            .filter(|s| !plex::is_down(&s.id))
            .cloned()
            .collect();
        servers.sort_by_key(|s| s.id != home);
        servers
    }

    fn open_owned_copy(&mut self, it: &Item, cx: &mut Context<Self>) {
        let servers = self.live_servers();
        let (client, key, it) = (self.client.clone(), it.rating_key.clone(), it.clone());
        Self::bg(
            cx,
            move || plaivo_core::watch_together::find_owned(&it, &servers, &client),
            move |this, found, cx| {
                let Some((server, local)) = found else { return };
                if this.page != Page::Detail || this.detail_id != (DISCOVER_ID.to_owned(), key) {
                    return;
                }
                let nav = Nav::Detail(server, local.rating_key, None);
                if let Some(last) = this.nav.last_mut() {
                    *last = nav.clone();
                }
                this.navigate(nav, false, cx);
            },
        );
    }

    fn load_related(&mut self, server: &str, it: &Item, cx: &mut Context<Self>) {
        self.fetch_watchlisted(it, cx);
        if self.owned_check.take_if(|k| *k == it.rating_key).is_some() {
            self.open_owned_copy(it, cx);
        }
        let Some(srv) = self
            .server(server)
            .filter(|_| server != DISCOVER_ID && matches!(it.kind.as_str(), "movie" | "show"))
        else {
            return;
        };
        let (server, key) = (server.to_owned(), it.rating_key.clone());
        let k = key.clone();
        self.load(
            cx,
            move |c| srv.related(c, &k),
            move |this, r, _| {
                let Some(d) = this
                    .detail
                    .as_mut()
                    .filter(|d| d.item.rating_key == key && *d.server == server)
                else {
                    return;
                };
                match r {
                    Ok(hubs) => {
                        d.related = hubs
                            .into_iter()
                            .take(3)
                            .map(|h| {
                                (
                                    h.title.into(),
                                    h.metadata
                                        .iter()
                                        .map(|it| card_for(it, false, &server))
                                        .collect(),
                                )
                            })
                            .collect()
                    }
                    Err(e) => log::debug!("related for {key}: {e}"),
                }
            },
        );
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.settings.sidebar_compact = !self.settings.sidebar_compact;
        self.save_settings_soon(cx);
        self.view.sidebar_anim = Some(Instant::now());
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, kind: &str, title: &str, guid: &str) -> Item {
        Item {
            rating_key: key.into(),
            kind: kind.into(),
            title: title.into(),
            guid: guid.into(),
            ..Default::default()
        }
    }

    #[test]
    fn title_matches_rank_above_incidental_hits() {
        let local = vec![(
            "home".to_owned(),
            vec![
                (item("1", "movie", "Sherlock Gnomes", "g1"), 0.99),
                (item("2", "show", "Dexter's Laboratory", "g2"), 0.5),
                (item("3", "show", "Dexter", "g3"), 0.4),
            ],
        )];
        let keys: Vec<_> = rank_search("dexter", "home", local, vec![])
            .into_iter()
            .map(|(_, i)| i.rating_key)
            .collect();
        assert_eq!(keys, ["3", "2", "1"]);
    }

    #[test]
    fn episodes_fold_into_one_show_and_owned_discover_hits_drop() {
        let mut ep = item("10", "episode", "Pilot", "");
        ep.grandparent_rating_key = "2".into();
        ep.grandparent_title = "Dexter's Laboratory".into();
        let local = vec![(
            "home".to_owned(),
            vec![
                (item("2", "show", "Dexter's Laboratory", "g2"), 0.5),
                (ep.clone(), 0.4),
                (ep, 0.3),
            ],
        )];
        let discover = vec![
            item("x", "show", "Dexter's Laboratory", "g2"),
            item("y", "show", "Dexter: New Blood", "g9"),
        ];
        let out = rank_search("dexter", "home", local, discover);
        let keys: Vec<_> = out
            .iter()
            .map(|(s, i)| format!("{s}/{}", i.rating_key))
            .collect();
        assert_eq!(keys, ["home/2", "discover/y"]);
    }

    #[test]
    fn word_prefix_beats_substring() {
        assert!(title_rank("The Dexter Files", "dex") < title_rank("Index", "dex"));
        assert_eq!(title_rank("Dexter", "DEXTER"), 0);
    }

    #[test]
    fn resolutions_read_like_labels() {
        assert_eq!(fmt_resolution("1080"), "1080p");
        assert_eq!(fmt_resolution("720p"), "720p");
        assert_eq!(fmt_resolution("4k"), "4K");
        assert_eq!(fmt_resolution("sd"), "SD");
        assert_eq!(fmt_resolution(""), "");
    }

    #[test]
    fn polling_stops_on_a_result_a_cancel_or_the_deadline() {
        let attempt = AtomicU64::new(1);
        let later = || Instant::now() + Duration::from_secs(5);
        let mut n = 0;
        assert_eq!(
            poll_until(&attempt, 1, Duration::ZERO, later(), || {
                n += 1;
                (n == 3).then_some(n)
            }),
            Some(3)
        );
        assert_eq!(
            poll_until(&attempt, 2, Duration::ZERO, later(), || Some(1)),
            None
        );
        assert_eq!(
            poll_until(&attempt, 1, Duration::ZERO, Instant::now(), || Some(1)),
            None
        );
    }

    #[test]
    fn hub_rows_keep_scroll_by_identity_not_position() {
        use gpui::{point, px};
        let hub = |id: &str, key: &str, title: &str| plex::Hub {
            id: id.into(),
            key: key.into(),
            title: title.into(),
            ..Default::default()
        };
        let old = hub_rows(&[hub("a", "/a", "A"), hub("b", "/b", "B")], "s", &[]);
        old[1].scroll.set_offset(point(px(0.), px(-40.)));
        let mut shifted = hub_rows(
            &[hub("continueWatching", "", "Continue Watching")],
            "s",
            &[],
        );
        shifted.extend(old);
        let new = hub_rows(&[hub("a", "/a", "A"), hub("b", "/b", "B")], "s", &shifted);
        let offsets: Vec<_> = new.iter().map(|h| h.scroll.offset().y).collect();
        assert_eq!(offsets, [px(0.), px(-40.)]);
    }

    #[test]
    fn image_sizes_round_up_to_50px_steps() {
        assert_eq!(shape_size(Shape::Poster, 1.0, 0), (250, 375));
        assert_eq!(shape_size(Shape::Poster, 1.5, 0), (350, 525));
        assert_eq!(shape_size(Shape::Backdrop, 1.0, 5000), (3840, 2160));
    }
}
