use crate::app::{Card, Choice, Nav, Page, Plaivo, card_for, join};
use crate::downloads::Local;
use crate::plex::{self, Ctx, Item, Media, Quality, Server, Source, resolve_playable};
use crate::prefs::{self, LanguagePrefs, LanguageRule, TrackInfo};
use crate::settings::{Scale, SkipMode, library_key};
use crate::ui::settings::quality_label;
use crate::ui::theme::{self, Icon};
use crate::video_host::{self, Region};
use crate::{mpv, shaders, store};
use futures::StreamExt;
use gpui::{Context, ScrollStrategy, SharedString, Task, UniformListScrollHandle};
use plaivo_core::prefs::language_label;
use plaivo_core::trackers::{self, PlayState};
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CONTROLS_FOR: Duration = Duration::from_secs(3);
const HINT_FOR: Duration = Duration::from_millis(1200);
const QUIT_REPORT_WAIT: Duration = Duration::from_secs(2);
const HOME_REFRESH: Duration = Duration::from_secs(120);
const CACHE_RENDER: Duration = Duration::from_millis(250);

#[derive(Deserialize)]
struct MpvTrack {
    id: i64,
    #[serde(rename = "type")]
    kind: String,
    title: Option<String>,
    lang: Option<String>,
    codec: Option<String>,
    #[serde(default)]
    selected: bool,
    #[serde(default)]
    forced: bool,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    external: bool,
    #[serde(rename = "external-filename")]
    external_filename: Option<String>,
    #[serde(rename = "demux-channel-count")]
    channels: Option<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Panel {
    Tracks,
    SubtitleSearch,
    Settings,
    Quality,
    Speed,
    Output,
    Shader,
    Info,
    Queue,
}

pub const SPEEDS: &[f64] = &[0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

#[derive(Default)]
pub struct SubSearch {
    pub language: String,
    pub results: Option<Result<Vec<plex::SubtitleResult>, String>>,
    pub downloading: Option<String>,
    pub generation: u64,
}

#[derive(Clone)]
pub struct Skip {
    pub label: &'static str,
    pub start: f64,
    pub end: f64,
    auto: bool,
}

pub type Crumb = (SharedString, Option<Nav>);

#[derive(Default)]
pub struct Player {
    pub server: Option<Server>,
    pub mpv: Option<mpv::Mpv>,
    pub host: Option<video_host::VideoHost>,
    pub active: bool,
    pub key: String,
    pub time: f64,
    pub duration: f64,
    pub cached: f64,
    pub speed: f64,
    pub audio_delay: f64,
    pub sub_delay: f64,
    languages: LanguagePrefs,
    rule: Option<LanguageRule>,
    tracks_picked: bool,
    pub media: Media,
    pub skips: Vec<Skip>,
    credits: Option<f64>,
    up_next_cancelled: bool,
    up_next_anchor: Option<f64>,
    advancing: bool,
    pub shader: String,
    pub quality: u32,
    library: String,
    pub shader_applied: &'static str,
    shader_failed: &'static str,
    transcode: Option<String>,
    local: bool,
    pub paused: bool,
    last_report: Option<Instant>,
    stop_sent: bool,
    reloads: u64,
    pub tracker: Option<trackers::Media>,
    external_subs: Vec<([String; 3], bool)>,
    pub title: SharedString,
    pub title_link: Option<Nav>,
    pub crumbs: Vec<Crumb>,
    pub queue: Vec<Card>,
    queue_show: (String, String),
    pub queue_scroll: UniformListScrollHandle,
    pub buffering: bool,
    pub volume: f64,
    pub muted: bool,
    pub audio_tracks: Vec<Choice>,
    pub sub_tracks: Vec<Choice>,
    pub video_ready: bool,
    pub mini: bool,
    pub controls: bool,
    pub panel: Option<Panel>,
    pub info: Vec<(SharedString, SharedString)>,
    pub devices: Vec<Choice>,
    hide_task: Option<Task<()>>,
    controls_until: Option<Instant>,
    play_gen: u64,
    pub opening: bool,
    pub sub_search: SubSearch,
    home_refreshed: Option<Instant>,
    cache_render: bool,
    pub hint: Option<(Icon, SharedString)>,
    hint_task: Option<Task<()>>,
    clock_task: Option<Task<()>>,
}

fn heading(it: &Item) -> (String, String) {
    let duration = it.duration.map(plex::fmt_duration).unwrap_or_default();
    match it.kind.as_str() {
        "episode" => (
            it.grandparent_title.clone(),
            join(&[it.episode_label(), it.title.clone(), duration]),
        ),
        _ => (
            it.title.clone(),
            join(&[it.year.map(|y| y.to_string()).unwrap_or_default(), duration]),
        ),
    }
}

fn queue_card(e: &Item, server: &str) -> Card {
    Card {
        meta: e
            .duration
            .map(plex::fmt_duration)
            .unwrap_or_default()
            .into(),
        ..card_for(e, true, server)
    }
}

impl Plaivo {
    fn ensure_player(&mut self, cx: &mut Context<Self>) -> Result<(), String> {
        if self.player.mpv.is_some() {
            return Ok(());
        }
        let host = video_host::create(self.window)?;
        let wid = host.wid().to_string();
        let conf_dir = store::config_dir().to_string_lossy().into_owned();
        let last_time = Arc::new(AtomicU64::new(0));
        let last_cache = Arc::new(AtomicU64::new(0));
        let device = self.audio_device();
        let [aspect, panscan] = self.settings.scale.mpv();
        let opts = [
            aspect,
            panscan,
            ("audio-device", device),
            ("wid", wid.as_str()),
            ("vo", "gpu-next"),
            ("hwdec", "auto-safe"),
            // Default lanczos upscaling was ~30% GPU for 1080p on an M4 Air; hermite ~16%, softer but no ringing.
            ("scale", "hermite"),
            ("target-colorspace-hint", "yes"),
            // Audio queued at the old speed is timed at the new one, so a speed change shifts video by up to this much (default 0.2s ≈ 5 frames).
            ("audio-buffer", "0.05"),
            ("idle", "yes"),
            ("keep-open", "no"),
            ("osc", "no"),
            ("osd-level", "0"),
            ("input-default-bindings", "no"),
            ("input-vo-keyboard", "no"),
            ("input-cursor", "no"),
            ("cursor-autohide", "no"),
            ("cache", "yes"),
            ("cache-secs", "60"),
            ("demuxer-max-bytes", "150MiB"),
            ("demuxer-max-back-bytes", "32MiB"),
            ("user-agent", "Plaivo"),
            ("audio-client-name", "Plaivo"),
            ("config-dir", conf_dir.as_str()),
            ("config", "yes"),
            ("ytdl", "no"),
            ("load-auto-profiles", "no"),
            ("load-stats-overlay", "no"),
            ("load-console", "no"),
            ("load-select", "no"),
            ("load-commands", "no"),
            ("load-positioning", "no"),
            (
                "gpu-context",
                if cfg!(target_os = "macos") {
                    "macvk"
                } else {
                    "auto"
                },
            ),
        ];
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let mpv = mpv::Mpv::new(&opts, move |e| {
            let throttle = |last: &AtomicU64, t: f64, step: f64| {
                if (t - f64::from_bits(last.load(Ordering::Relaxed))).abs() < step {
                    return true;
                }
                last.store(t.to_bits(), Ordering::Relaxed);
                false
            };
            match e {
                mpv::Event::Time(t) if throttle(&last_time, t, 0.25) => return,
                mpv::Event::Cached(t) if throttle(&last_cache, t, 1.0) => return,
                _ => {}
            }
            let _ = tx.unbounded_send(e);
        })?;
        cx.spawn(async move |this, cx| {
            while let Some(e) = rx.next().await {
                if this.update(cx, |this, cx| this.on_mpv(e, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        self.player.host = Some(host);
        self.player.mpv = Some(mpv);
        self.player.volume = 100.0;
        self.player.speed = 1.0;
        self.apply_sub_style();
        Ok(())
    }

    pub fn play(&mut self, server: String, key: String, cx: &mut Context<Self>) {
        self.play_with(server, key, None, false, cx);
    }

    fn play_with(
        &mut self,
        server: String,
        key: String,
        version: Option<usize>,
        from_start: bool,
        cx: &mut Context<Self>,
    ) {
        log::info!("play request {server}/{key}");
        if server == plex::DISCOVER_ID {
            return self.toast("This title isn't in any of your libraries.", cx);
        }
        if version.is_none()
            && self.playing()
            && self.player.key == key
            && self.player.server.as_ref().is_some_and(|s| s.id == server)
        {
            if self.player.paused {
                self.toggle_pause();
            }
            self.cancel_pending_play();
            return self.expand_player(cx);
        }
        self.player.play_gen += 1;
        let gen_ = self.player.play_gen;
        let downloaded = || {
            self.offline
                .iter()
                .any(|(s, it)| *s == server && it.rating_key == key)
        };
        let Some(srv) = self.server(&server).or_else(|| {
            downloaded().then(|| Server {
                id: server.clone(),
                ..Default::default()
            })
        }) else {
            return self.advance_failed();
        };
        self.show_opening(&server, &key, cx);
        let (c, requested) = (self.client.clone(), key.clone());
        let (token, local, downloads, rules) = (
            self.account.token.clone(),
            self.settings.languages(),
            self.downloads.clone(),
            self.settings.language_rules.clone(),
        );
        Self::bg(
            cx,
            move || {
                // Loads libmpv and its DLLs here instead of on the UI thread at first play.
                mpv::preload();
                if let Some(d) = &downloads
                    && let Some(file) = d.local(&srv.id, &key)
                    && let Some(it) = d.item(&srv.id, &key)
                {
                    let rule = prefs::language_rule(&srv.id, &it, &rules);
                    return (srv, Ok(it), local, rule, Some(file));
                }
                let it = resolve_playable(&srv, &Ctx::new(c.clone(), Source::Live), &key);
                let rule = it
                    .as_ref()
                    .ok()
                    .and_then(|it| prefs::language_rule(&srv.id, it, &rules));
                let languages = prefs::for_server(&c, &srv, token.as_deref(), &local);
                (srv, it, languages, rule, None)
            },
            move |this, (srv, r, languages, rule, file), cx| match r {
                _ if this.player.play_gen != gen_ => {}
                Ok(it) => {
                    if rule.is_some() {
                        log::info!("language override applies to {}: {rule:?}", it.rating_key);
                    }
                    this.player.languages = languages;
                    this.player.rule = rule;
                    let same = it.rating_key == requested;
                    let version = version.filter(|_| same);
                    if let Err(e) = this.start_item(srv, it, file, version, from_start && same, cx)
                    {
                        this.advance_failed();
                        this.abort_opening(cx);
                        this.toast(e, cx);
                    }
                }
                Err(e) => {
                    this.advance_failed();
                    this.abort_opening(cx);
                    this.toast(format!("Could not play: {e}"), cx);
                }
            },
        );
    }

    pub fn play_from_start(&mut self, server: String, key: String, cx: &mut Context<Self>) {
        if self.playing()
            && self.player.key == key
            && self.player.server.as_ref().is_some_and(|s| s.id == server)
        {
            self.seek_by(-self.player.time);
            return self.expand_player(cx);
        }
        self.play_with(server, key, None, true, cx);
    }

    pub fn play_version(
        &mut self,
        server: String,
        key: String,
        version: usize,
        cx: &mut Context<Self>,
    ) {
        self.play_with(server, key, Some(version), false, cx);
    }

    fn start_item(
        &mut self,
        srv: Server,
        mut it: Item,
        local: Option<Local>,
        version: Option<usize>,
        from_start: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(v) = version.filter(|v| *v < it.media.len() && local.is_none()) {
            it.media.swap(0, v);
        }
        let part = it
            .media
            .first()
            .and_then(|m| m.part.first())
            .ok_or("This item has no playable media")?;
        let resume = it.resume_ms().filter(|_| !from_start);
        let subs = match &local {
            Some(l) => l
                .subtitles
                .iter()
                .map(|(file, st)| {
                    (
                        [
                            file.to_string_lossy().into_owned(),
                            st.display_title.clone(),
                            st.language_code.clone(),
                        ],
                        st.forced,
                    )
                })
                .collect(),
            None => part
                .stream
                .iter()
                .filter(|st| st.is_external_subtitle())
                .map(|st| {
                    (
                        [
                            srv.media_url(&st.key),
                            st.display_title.clone(),
                            st.language_code.clone(),
                        ],
                        st.forced,
                    )
                })
                .collect(),
        };
        let (title, subtitle) = heading(&it);
        let media = it.media[0].clone();
        let library = library_key(&srv.id, &it.library_section_id);
        let (url, session) = match &local {
            Some(l) => (l.file.to_string_lossy().into_owned(), None),
            None => self.stream_url(
                &srv,
                &it.rating_key,
                &media,
                self.settings.quality_for(&library),
            ),
        };
        if self.playing() {
            self.report("stopped", cx);
            self.end_transcode(cx);
        }
        self.player.server = Some(srv);
        self.player.home_refreshed = None;
        self.launch(
            &url,
            &it.rating_key,
            &title,
            &subtitle,
            it.duration.unwrap_or(0) as f64 / 1000.0,
            resume.map(|ms| ms as f64 / 1000.0),
            subs,
            &library,
            cx,
        )?;
        self.player.local = local.is_some();
        self.resolve_tracker_media(&it, cx);
        let server = self
            .player
            .server
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_default();
        let at = resume.unwrap_or(0) as i64;
        self.watch_share(server, it.rating_key.clone(), Some(it.clone()), at, cx);
        let id = &self.player.server.as_ref().unwrap().id;
        let link = |key: &str, season: Option<&str>| {
            (!key.is_empty())
                .then(|| Nav::Detail(id.clone(), key.to_owned(), season.map(str::to_owned)))
        };
        let (title_link, crumbs): (_, Vec<Crumb>) = match it.kind.as_str() {
            "episode" => {
                let ep = link(&it.rating_key, None);
                let season = link(
                    &it.grandparent_rating_key,
                    Some(&it.parent_rating_key)
                        .filter(|k| !k.is_empty())
                        .map(|k| k.as_str()),
                );
                let parts = [
                    (it.parent_index.map(|s| format!("S{s}")), season),
                    (it.index.map(|e| format!("E{e}")), ep.clone()),
                    (Some(it.title.clone()), ep),
                    (it.duration.map(plex::fmt_duration), None),
                ];
                (
                    link(&it.grandparent_rating_key, None),
                    parts
                        .into_iter()
                        .filter_map(|(t, n)| Some((t.filter(|t| !t.is_empty())?.into(), n)))
                        .collect(),
                )
            }
            _ => (
                link(&it.rating_key, None),
                subtitle
                    .split(" · ")
                    .filter(|s| !s.is_empty())
                    .map(|s| (s.to_owned().into(), None))
                    .collect(),
            ),
        };
        if it.kind == "episode" {
            self.load_queue(it.grandparent_rating_key.clone(), cx);
            if self.player.local && self.player.queue.is_empty() {
                let server = self
                    .player
                    .server
                    .as_ref()
                    .map(|s| s.id.clone())
                    .unwrap_or_default();
                self.player.queue = crate::ui::downloads::offline_episodes(
                    &self.offline,
                    &server,
                    &it.grandparent_rating_key,
                )
                .into_iter()
                .map(|e| queue_card(e, &server))
                .collect();
            }
        } else {
            self.player.queue.clear();
            self.player.queue_show = Default::default();
        }
        let p = &mut self.player;
        p.title_link = title_link;
        p.crumbs = crumbs;
        p.transcode = session;
        p.media = media;
        p.credits = it
            .marker
            .iter()
            .find(|m| m.kind == "credits")
            .map(|m| m.start_time_offset as f64 / 1000.0);
        p.skips = it
            .marker
            .iter()
            .filter_map(|m| {
                let label = m.skip_label()?;
                let mode = self.settings.skip_mode(&m.kind);
                (mode != SkipMode::Off).then(|| Skip {
                    label,
                    start: m.start_time_offset as f64 / 1000.0,
                    end: m.end_time_offset as f64 / 1000.0,
                    auto: mode == SkipMode::Auto,
                })
            })
            .collect();
        Ok(())
    }

    fn load_queue(&mut self, show: String, cx: &mut Context<Self>) {
        let Some(srv) = self.player.server.clone() else {
            return;
        };
        let wanted = (srv.id.clone(), show);
        if self.player.queue_show != wanted {
            self.player.queue.clear();
            self.player.queue_show = wanted.clone();
        }
        let c = self.client.clone();
        Self::bg(
            cx,
            move || {
                srv.all_episodes(&Ctx::new(c, Source::Network), &wanted.1)
                    .map(|eps| (wanted, eps))
            },
            |this, r, _| match r {
                Ok((wanted, eps)) if this.player.queue_show == wanted => {
                    this.player.queue = eps.iter().map(|e| queue_card(e, &wanted.0)).collect();
                }
                Ok(_) => {}
                Err(e) => log::warn!("could not load episode queue: {e}"),
            },
        );
    }

    pub fn queue_index(&self) -> Option<usize> {
        self.player
            .queue
            .iter()
            .position(|c| c.key.as_ref() == self.player.key)
    }

    pub fn neighbor(&self, step: isize) -> Option<Card> {
        let i = self.queue_index()?.checked_add_signed(step)?;
        self.player.queue.get(i).cloned()
    }

    pub fn play_card(&mut self, card: Card, cx: &mut Context<Self>) {
        if card.key.as_ref() == self.player.key {
            return;
        }
        self.report("stopped", cx);
        self.player.buffering = true;
        self.player.advancing = true;
        self.play(card.server.to_string(), card.key.to_string(), cx);
    }

    pub(crate) fn cancel_pending_play(&mut self) {
        self.player.play_gen += 1;
        self.advance_failed();
    }

    fn known_item(&self, server: &str, key: &str) -> Option<Item> {
        let detail = self
            .detail
            .as_ref()
            .filter(|d| *d.server == *server)
            .map(|d| &d.item);
        let on_deck = detail
            .and_then(|it| it.on_deck.as_ref())
            .and_then(|d| d.metadata.as_ref());
        let extras = detail.map_or(&[][..], |it| it.extras());
        let episodes = self
            .episode_items
            .iter()
            .filter(|_| self.detail_id.0 == server);
        detail
            .into_iter()
            .chain(on_deck)
            .chain(extras)
            .chain(episodes)
            .find(|it| it.rating_key == key)
            .cloned()
    }

    fn known_card(&self, server: &str, key: &str) -> Option<Card> {
        let hubs = self
            .hubs
            .iter()
            .chain(self.lib_hubs.iter().flatten())
            .flat_map(|h| h.items.iter())
            .chain(self.hub_list.iter().flat_map(|l| l.cards.iter()));
        let library = self.library.iter().flat_map(|l| l.items.iter().flatten());
        let related = self
            .detail
            .iter()
            .flat_map(|d| d.related.iter().flat_map(|(_, cards)| cards.iter()));
        hubs.chain(library)
            .chain(&self.results)
            .chain(&self.player.queue)
            .chain(related)
            .find(|c| *c.key == *key && *c.server == *server)
            .cloned()
    }

    fn show_opening(&mut self, server: &str, key: &str, cx: &mut Context<Self>) {
        self.player.buffering = true;
        if self.playing() {
            return self.expand_player(cx);
        }
        let (title, subtitle, duration) = match self.known_item(server, key) {
            Some(it) => {
                let (title, subtitle) = heading(&it);
                (title, subtitle, it.duration.unwrap_or(0) as f64 / 1000.0)
            }
            None => self
                .known_card(server, key)
                .map_or_else(Default::default, |c| {
                    (c.title.to_string(), c.subtitle.to_string(), 0.0)
                }),
        };
        self.player.opening = true;
        self.player.video_ready = false;
        self.player.title = title.into();
        self.player.crumbs = subtitle
            .split(" · ")
            .filter(|s| !s.is_empty())
            .map(|s| (SharedString::from(s.to_owned()), None))
            .collect();
        self.player.time = 0.0;
        self.player.duration = duration;
        self.page = Page::Player;
        self.wake_controls(cx);
    }

    pub fn abort_opening(&mut self, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.player.opening) {
            return;
        }
        self.player.play_gen += 1;
        self.player.buffering = false;
        if let Some(nav) = self.nav.last().cloned() {
            self.navigate(nav, false, cx);
        }
    }

    fn advance_failed(&mut self) {
        let p = &mut self.player;
        p.up_next_cancelled |= p.advancing;
        p.stop_sent &= !p.advancing;
        if p.advancing {
            p.buffering = !p.video_ready
                || p.mpv
                    .as_ref()
                    .and_then(|m| m.get("paused-for-cache"))
                    .as_deref()
                    == Some("yes");
        }
        p.advancing = false;
    }

    pub fn up_next(&mut self) -> Option<(Card, f64)> {
        if self.watch_following() {
            return None;
        }
        let p = &mut self.player;
        if p.up_next_cancelled || p.advancing {
            return None;
        }
        let left =
            self.settings
                .up_next_in(p.time, p.duration, p.credits, &mut p.up_next_anchor)?;
        Some((self.neighbor(1)?, left))
    }

    pub fn play_up_next(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.neighbor(1) {
            self.play_card(next, cx);
        }
    }

    pub fn cancel_up_next(&mut self) {
        self.player.up_next_cancelled = true;
    }

    fn auto_skip(&mut self, cx: &mut Context<Self>) {
        if self.watch_holds() && !self.watch_controls() {
            return;
        }
        let t = self.player.time;
        let Some(s) = self
            .player
            .skips
            .iter_mut()
            .find(|s| s.auto && t >= s.start && t < s.end - 1.0)
        else {
            return;
        };
        s.auto = false;
        let (end, label) = (s.end, s.label.replacen("Skip", "Skipped", 1));
        self.player.time = end;
        self.seek_to(end);
        self.hint(Icon::FastForward, label, cx);
    }

    pub fn set_shader(&mut self, id: &str, cx: &mut Context<Self>) {
        self.player.shader = id.to_owned();
        self.sync_shader(cx);
    }

    fn sync_shader(&mut self, cx: &mut Context<Self>) {
        let Some(m) = &self.player.mpv else { return };
        let hdr = || {
            shaders::is_hdr(
                &m.get("video-params/primaries").unwrap_or_default(),
                &m.get("video-params/gamma").unwrap_or_default(),
            )
        };
        let preset =
            shaders::find(&self.player.shader).filter(|p| p.id != shaders::NVSCALER || !hdr());
        let id = preset.map_or("", |p| p.id);
        if id == self.player.shader_applied || (!id.is_empty() && id == self.player.shader_failed) {
            return;
        }
        let dir = shaders::dir();
        let files = preset.map_or(&[][..], |p| p.files);
        if let Some(missing) = files.iter().map(|f| dir.join(f)).find(|f| !f.is_file()) {
            self.player.shader_failed = id;
            return self.toast(format!("Shader file missing: {}", missing.display()), cx);
        }
        log::info!("shaders: {}", if id.is_empty() { "off" } else { id });
        m.command(&["change-list", "glsl-shaders", "clr", ""]);
        for f in files {
            m.command(&[
                "change-list",
                "glsl-shaders",
                "append",
                &dir.join(f).to_string_lossy(),
            ]);
        }
        self.player.shader_applied = id;
    }

    pub fn play_neighbor(&mut self, step: isize, cx: &mut Context<Self>) {
        if let Some(card) = self.neighbor(step) {
            self.play_card(card, cx);
        }
    }

    fn stream_url(
        &self,
        srv: &Server,
        key: &str,
        media: &Media,
        kbps: u32,
    ) -> (String, Option<String>) {
        let (url, transcode) =
            srv.stream_url(&self.client, key, media, kbps, false, "aac,ac3,eac3,mp3");
        (url, transcode.map(|(_, session)| session))
    }

    fn end_transcode(&mut self, cx: &mut Context<Self>) {
        if let (Some(session), Some(srv)) =
            (self.player.transcode.take(), self.player.server.clone())
        {
            let c = self.client.clone();
            Self::detach(cx, move || srv.stop_transcode(&c, &session));
        }
    }

    pub fn set_quality(&mut self, kbps: u32, cx: &mut Context<Self>) {
        self.settings.set_quality_for(&self.player.library, kbps);
        self.save_settings_soon(cx);
        self.apply_quality(kbps, cx);
    }

    pub fn set_default_quality(&mut self, kbps: u32, cx: &mut Context<Self>) {
        self.settings.quality = kbps;
        self.save_settings_soon(cx);
        if self.settings.quality_for(&self.player.library) == kbps {
            self.apply_quality(kbps, cx);
        }
    }

    fn apply_quality(&mut self, kbps: u32, cx: &mut Context<Self>) {
        if kbps == self.player.quality {
            return;
        }
        self.player.quality = kbps;
        let Some(srv) =
            self.player.server.clone().filter(|_| {
                self.playing() && !self.player.local && !self.player.media.part.is_empty()
            })
        else {
            return;
        };
        let (url, session) = self.stream_url(
            &srv,
            &self.player.key.clone(),
            &self.player.media.clone(),
            kbps,
        );
        let report = self
            .player
            .transcode
            .is_some()
            .then(|| self.report_job("stopped", true))
            .flatten();
        let old = self.player.transcode.take();
        let p = &mut self.player;
        p.stop_sent = false;
        p.transcode = session;
        p.buffering = true;
        p.cached = 0.0;
        p.reloads += 1;
        let (at, reload) = (p.time.to_string(), p.reloads);
        let load = move |this: &mut Self, _, _: &mut Context<Self>| {
            if this.player.reloads != reload {
                return;
            }
            this.mpv(|m| {
                m.set("start", &at);
                m.command(&["loadfile", &url, "replace"]);
            });
        };
        let Some(old) = old else {
            load(self, (), cx);
            return cx.notify();
        };
        self.player.video_ready = false;
        self.mpv(|m| m.command(&["stop"]));
        let c = self.client.clone();
        // Plex answers 400 to a new transcode while the old playback is still reported as active
        Self::bg(
            cx,
            move || {
                if let Some(job) = report {
                    job();
                }
                srv.stop_transcode(&c, &old);
            },
            load,
        );
        cx.notify();
    }

    pub fn set_scale(&mut self, scale: Scale, cx: &mut Context<Self>) {
        self.settings.scale = scale;
        self.save_settings_soon(cx);
        if let Some(m) = &self.player.mpv {
            for (k, v) in scale.mpv() {
                m.set(k, v);
            }
        }
    }

    pub fn set_speed(&mut self, speed: f64) {
        if self.watch_holds() {
            return self.watch_call(|wt| wt.set_rate(speed));
        }
        self.player.speed = speed;
        self.mpv(|m| m.set("speed", &speed.to_string()));
    }

    pub fn start_boost(&mut self) {
        if self.view.boost.is_none() {
            self.view.boost = Some(self.player.speed);
            self.set_speed(2.0);
        }
    }

    pub fn end_boost(&mut self) -> bool {
        let Some(prev) = self.view.boost.take() else {
            return false;
        };
        self.set_speed(prev);
        true
    }

    pub fn nudge_delay(&mut self, audio: bool, ms: f64) {
        let p = &mut self.player;
        let v = if audio {
            &mut p.audio_delay
        } else {
            &mut p.sub_delay
        };
        *v = if ms == 0.0 {
            0.0
        } else {
            (*v + ms).clamp(-10_000.0, 10_000.0)
        };
        let secs = (*v / 1000.0).to_string();
        self.mpv(|m| m.set(if audio { "audio-delay" } else { "sub-delay" }, &secs));
    }

    fn audio_device(&self) -> &str {
        if self.settings.audio_device.is_empty() {
            "auto"
        } else {
            &self.settings.audio_device
        }
    }

    pub fn set_audio_device(&mut self, name: &str, cx: &mut Context<Self>) {
        self.settings.audio_device = if name == "auto" {
            String::new()
        } else {
            name.to_owned()
        };
        self.save_settings_soon(cx);
        if let Some(m) = &self.player.mpv {
            m.set("audio-device", name);
        }
    }

    pub fn active_skip(&self) -> Option<&Skip> {
        let t = self.player.time;
        self.player
            .skips
            .iter()
            .find(|s| !s.auto && t >= s.start && t < s.end - 1.0)
    }

    pub fn skip_marker(&mut self) {
        if let Some(end) = self.active_skip().map(|s| s.end) {
            self.player.time = end;
            self.seek_to(end);
        }
    }

    pub fn open_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        match panel {
            Panel::Info => self.player.info = self.file_info(),
            Panel::Queue => {
                if let Some(i) = self.queue_index() {
                    self.player
                        .queue_scroll
                        .scroll_to_item(i, ScrollStrategy::Top);
                }
            }
            Panel::Output => {
                #[derive(Deserialize)]
                struct Device {
                    name: String,
                    description: String,
                }
                let list: Vec<Device> = self
                    .player
                    .mpv
                    .as_ref()
                    .and_then(|m| m.get("audio-device-list"))
                    .and_then(|j| serde_json::from_str(&j).ok())
                    .unwrap_or_default();
                let current = self.audio_device();
                self.player.devices = list
                    .into_iter()
                    .map(|d| Choice {
                        selected: d.name == current,
                        id: d.name.into(),
                        label: d.description.into(),
                        sub: "".into(),
                    })
                    .collect();
            }
            _ => {}
        }
        self.player.panel = Some(panel);
        self.wake_controls(cx);
        cx.notify();
    }

    fn file_info(&self) -> Vec<(SharedString, SharedString)> {
        let Some(m) = &self.player.mpv else {
            return Vec::new();
        };
        let get = |k: &str| m.get(k).unwrap_or_default();
        let media = &self.player.media;
        let file = media
            .part
            .first()
            .and_then(|p| p.file.rsplit(['/', '\\']).next())
            .unwrap_or_default()
            .to_owned();
        let mode = match (&self.player.transcode, Quality::find(self.player.quality)) {
            _ if self.player.local => "Downloaded".into(),
            (Some(_), Some(_)) => format!("Transcode · {}", quality_label(self.player.quality)),
            _ => "Direct play".into(),
        };
        let fps = get("container-fps")
            .parse::<f64>()
            .map(|f| format!("{f:.3} fps").replace(".000", ""))
            .unwrap_or_default();
        let kbps = |k: &str| {
            get(k)
                .parse::<f64>()
                .ok()
                .filter(|b| *b > 0.0)
                .map(|b| plex::fmt_bitrate((b / 1000.0) as u64))
                .unwrap_or_default()
        };
        let size = match (get("video-params/w"), get("video-params/h")) {
            (w, h) if !w.is_empty() && !h.is_empty() => format!("{w}×{h}"),
            _ => String::new(),
        };
        let hw = get("hwdec-current");
        let rate = get("audio-params/samplerate")
            .parse::<f64>()
            .map(|r| format!("{} kHz", r / 1000.0))
            .unwrap_or_default();
        let subtitles = if get("sid") == "no" {
            let any = media
                .part
                .first()
                .is_some_and(|p| p.stream.iter().any(|s| s.stream_type == 3));
            if any { "Off".into() } else { String::new() }
        } else {
            let title = get("current-tracks/sub/title");
            let lang = get("current-tracks/sub/lang");
            let lang = language_label(&lang).map_or(lang, str::to_owned);
            join(&[
                title.clone(),
                if lang == title { String::new() } else { lang },
                get("current-tracks/sub/codec").to_uppercase(),
                if get("current-tracks/sub/forced") == "yes" {
                    "Forced".into()
                } else {
                    String::new()
                },
            ])
        };
        let rows = [
            ("Playback", mode),
            ("File", file),
            (
                "Container",
                join(&[
                    get("file-format"),
                    media.bitrate.map(plex::fmt_bitrate).unwrap_or_default(),
                ]),
            ),
            (
                "Video",
                join(&[
                    get("video-format").to_uppercase(),
                    size,
                    fps,
                    kbps("video-bitrate"),
                ]),
            ),
            (
                "Color",
                join(&[
                    get("video-params/pixelformat"),
                    get("video-params/primaries"),
                    get("video-params/gamma"),
                ]),
            ),
            (
                "Decoder",
                if hw.is_empty() || hw == "no" {
                    "Software".into()
                } else {
                    format!("Hardware ({hw})")
                },
            ),
            (
                "Audio",
                join(&[
                    get("audio-codec-name").to_uppercase(),
                    get("audio-params/hr-channels"),
                    rate,
                    kbps("audio-bitrate"),
                ]),
            ),
            ("Output", get("current-ao")),
            ("Subtitles", subtitles),
        ];
        rows.into_iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.into(), v.into()))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        &mut self,
        url: &str,
        key: &str,
        title: &str,
        subtitle: &str,
        duration: f64,
        start_at: Option<f64>,
        subs: Vec<([String; 3], bool)>,
        library: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        log::info!(
            "launch '{title}' key={key} start={start_at:?} subs={}",
            subs.len()
        );
        self.ensure_player(cx)?;
        let p = &mut self.player;
        p.active = true;
        p.opening = false;
        p.key = key.to_owned();
        p.duration = duration;
        p.time = start_at.unwrap_or(0.0);
        p.paused = false;
        p.cached = 0.0;
        p.audio_delay = 0.0;
        p.sub_delay = 0.0;
        p.tracks_picked = false;
        p.media = Media::default();
        p.skips.clear();
        p.credits = None;
        p.up_next_cancelled = false;
        p.up_next_anchor = None;
        p.advancing = false;
        p.shader = self.settings.shader_for(library).to_owned();
        p.quality = self.settings.quality_for(library);
        p.library = library.to_owned();
        p.local = false;
        p.last_report = None;
        p.stop_sent = false;
        p.tracker = None;
        p.external_subs = subs;
        p.title = title.to_owned().into();
        p.title_link = None;
        p.crumbs = (!subtitle.is_empty())
            .then(|| (subtitle.to_owned().into(), None))
            .into_iter()
            .collect();
        p.buffering = true;
        p.video_ready = false;
        p.audio_tracks.clear();
        p.sub_tracks.clear();
        p.mini = false;
        p.panel = None;
        let mpv = p.mpv.as_ref().unwrap();
        mpv.set("audio-delay", "0");
        mpv.set("sub-delay", "0");
        mpv.set("start", &start_at.map_or("none".into(), |t| t.to_string()));
        mpv.set("force-media-title", title);
        mpv.set("alang", &prefs::language_codes(&p.languages.audio));
        mpv.set("slang", &prefs::language_codes(&p.languages.subtitles));
        mpv.set("aid", "auto");
        mpv.set("sid", "auto");
        mpv.set("pause", "no");
        mpv.command(&["loadfile", url, "replace"]);
        let host = p.host.as_ref().unwrap();
        host.set_region(None);
        host.set_visible(true);
        self.page = Page::Player;
        self.trim_images(true, cx);
        self.sync_shader(cx);
        self.wake_controls(cx);
        Ok(())
    }

    pub fn wake_controls(&mut self, cx: &mut Context<Self>) {
        let p = &mut self.player;
        p.controls_until = Some(Instant::now() + CONTROLS_FOR);
        if p.controls && p.hide_task.is_some() {
            return;
        }
        p.controls = true;
        p.hide_task = Some(cx.spawn(async move |this, cx| {
            let mut wait = CONTROLS_FOR;
            loop {
                cx.background_executor().timer(wait).await;
                match this.update(cx, |this, cx| this.hide_controls(cx)) {
                    Ok(Some(left)) => wait = left,
                    _ => break,
                }
            }
        }));
        cx.notify();
    }

    fn hide_controls(&mut self, cx: &mut Context<Self>) -> Option<Duration> {
        let p = &mut self.player;
        let left = p.controls_until.map_or(Duration::ZERO, |t| {
            t.saturating_duration_since(Instant::now())
        });
        if !left.is_zero() {
            return Some(left);
        }
        if p.paused || p.panel.is_some() {
            return Some(CONTROLS_FOR);
        }
        p.controls = false;
        cx.notify();
        None
    }

    pub fn controls_shown(&self) -> bool {
        let p = &self.player;
        self.page == Page::Player && (p.controls || p.paused || p.panel.is_some())
    }

    fn show_cursor(&mut self) {
        if std::mem::take(&mut self.view.cursor_hidden)
            && let Some(host) = &self.player.host
        {
            host.set_cursor_hidden(false);
        }
    }

    fn shrink_player(&mut self, cx: &mut Context<Self>) {
        self.trim_images(false, cx);
        self.show_cursor();
        self.player.mini = true;
        self.player.panel = None;
        let (x, bottom, w, h) = theme::MINI_HOLE;
        if let Some(host) = &self.player.host {
            host.set_region(Some(Region { x, bottom, w, h }));
        }
        if self.home_stale() {
            self.player.home_refreshed = Some(Instant::now());
            self.load_home(cx);
        }
    }

    fn home_stale(&self) -> bool {
        self.player
            .home_refreshed
            .is_none_or(|t| t.elapsed() >= HOME_REFRESH)
    }

    pub fn minimize_player(&mut self, cx: &mut Context<Self>) {
        if !self.playing() {
            return self.abort_opening(cx);
        }
        self.shrink_player(cx);
        if let Some(nav) = self.nav.last().cloned() {
            self.navigate(nav, false, cx);
        }
    }

    pub fn open_from_player(&mut self, nav: Nav, cx: &mut Context<Self>) {
        if self.playing() && !self.player.mini {
            self.shrink_player(cx);
        }
        self.navigate(nav, true, cx);
    }

    pub fn expand_player(&mut self, cx: &mut Context<Self>) {
        if !self.playing() {
            return;
        }
        self.player.mini = false;
        self.page = Page::Player;
        self.trim_images(true, cx);
        if let Some(host) = &self.player.host {
            host.set_region(None);
        }
        self.wake_controls(cx);
        cx.notify();
    }

    pub fn playing(&self) -> bool {
        self.player.active
    }

    fn report(&mut self, state: &str, cx: &mut Context<Self>) {
        let refresh = state == "stopped" || (self.player.mini && self.home_stale());
        let Some(job) = self.report_job(state, true) else {
            return;
        };
        if !refresh {
            return Self::detach(cx, job);
        }
        self.player.home_refreshed = Some(Instant::now());
        Self::bg(cx, job, |this, _, cx| this.load_home(cx));
    }

    // Quit can't wait on a dead server: local files skip the network, streams get a short grace period.
    pub fn report_stopped_blocking(&mut self) {
        let Some(job) = self.report_job("stopped", !self.player.local) else {
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            job();
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(QUIT_REPORT_WAIT);
    }

    fn report_job(&mut self, state: &str, network: bool) -> Option<impl FnOnce() + Send + 'static> {
        let srv = self.player.server.clone()?;
        if !self.playing() || self.player.key.is_empty() || self.player.stop_sent {
            return None;
        }
        let c = self.client.clone();
        let p = &mut self.player;
        p.last_report = Some(Instant::now());
        p.stop_sent = state == "stopped";
        let offline = self.downloads.clone().filter(|_| p.local);
        let (key, state, t, d, session) = (
            p.key.clone(),
            state.to_owned(),
            (p.time * 1000.0) as u64,
            (p.duration * 1000.0) as u64,
            p.transcode.clone(),
        );
        if let Some(m) = &p.tracker
            && let Some(s) = PlayState::parse(&state)
        {
            self.services.trackers.on_progress(m, s, t, d);
        }
        Some(move || {
            let synced = network && srv.timeline(&c, &key, &state, t, d, session.as_deref());
            if let Some(downloads) = offline {
                downloads.record_progress(&srv.id, &key, &state, t, d, synced);
            }
        })
    }

    fn on_mpv(&mut self, e: mpv::Event, cx: &mut Context<Self>) {
        let notify = match e {
            mpv::Event::Time(t) => {
                let shown = |this: &mut Self| {
                    (
                        this.controls_shown().then_some(this.player.time as u64),
                        this.active_skip().is_some(),
                        this.up_next().map(|(_, left)| left.max(0.).ceil() as u64),
                    )
                };
                let before = shown(self);
                let second = self.player.time as u64;
                self.player.time = t;
                if self.player.mini && t as u64 != second {
                    cx.emit(crate::ui::shell::MiniTick);
                }
                if self
                    .player
                    .last_report
                    .is_none_or(|r| r.elapsed() > Duration::from_secs(10))
                {
                    let state = if self.player.paused {
                        "paused"
                    } else {
                        "playing"
                    };
                    self.report(state, cx);
                }
                self.watch_feed(false);
                self.auto_skip(cx);
                if self.up_next().is_some_and(|(_, left)| left <= 0.0) {
                    self.play_up_next(cx);
                }
                shown(self) != before
            }
            mpv::Event::Duration(d) => {
                self.player.duration = d;
                self.watch_feed(true);
                true
            }
            mpv::Event::Cached(t) => {
                self.player.cached = t;
                // A refill after a seek sends dozens of these a second; coalesce them into one render.
                if self.controls_shown() && !std::mem::replace(&mut self.player.cache_render, true)
                {
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(CACHE_RENDER).await;
                        this.update(cx, |this, cx| {
                            this.player.cache_render = false;
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                false
            }
            mpv::Event::Pause(p) => {
                self.player.paused = p;
                self.report(if p { "paused" } else { "playing" }, cx);
                self.watch_feed(true);
                if !p {
                    self.wake_controls(cx);
                }
                true
            }
            mpv::Event::Buffering(b) => {
                self.player.buffering = b;
                self.watch_feed(true);
                true
            }
            mpv::Event::Volume(v) => {
                self.player.volume = v;
                true
            }
            mpv::Event::Mute(m) => {
                self.player.muted = m;
                true
            }
            mpv::Event::Loaded => {
                self.player.buffering = false;
                if let Some(mpv) = &self.player.mpv {
                    for ([url, title, lang], _) in &self.player.external_subs {
                        mpv.command(&["sub-add", url, "auto", title, lang]);
                    }
                }
                self.auto_tracks();
                true
            }
            mpv::Event::Tracks => {
                self.refresh_tracks();
                true
            }
            mpv::Event::Started => {
                self.player.video_ready = true;
                self.sync_shader(cx);
                self.watch_feed(true);
                true
            }
            mpv::Event::Ended { eof, error } => {
                log::info!("playback ended eof={eof} error={error:?}");
                if self.playing() && !self.player.advancing && (eof || error.is_some()) {
                    if let Some(e) = error {
                        self.toast(format!("Playback error: {e}"), cx);
                    }
                    match self.neighbor(1).filter(|_| {
                        eof && self.settings.autoplay_next
                            && !self.player.up_next_cancelled
                            && !self.watch_following()
                    }) {
                        Some(next) => self.play_card(next, cx),
                        None => self.stop_playback(cx),
                    }
                }
                true
            }
        };
        if notify {
            cx.notify();
        }
    }

    fn refresh_tracks(&mut self) {
        let Some(mpv) = &self.player.mpv else { return };
        let tracks: Vec<MpvTrack> = mpv
            .get("track-list")
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let describe = |t: &MpvTrack| Choice {
            id: t.id.to_string().into(),
            label: t
                .title
                .clone()
                .or_else(|| t.lang.clone())
                .unwrap_or_else(|| format!("Track {}", t.id))
                .into(),
            sub: join(&[
                t.lang.clone().unwrap_or_default().to_uppercase(),
                t.codec.clone().unwrap_or_default().to_uppercase(),
                match t.channels {
                    Some(1) => "Mono".into(),
                    Some(2) => "Stereo".into(),
                    Some(6) => "5.1".into(),
                    Some(8) => "7.1".into(),
                    Some(n) => format!("{n}ch"),
                    None => String::new(),
                },
                if t.forced {
                    "Forced".into()
                } else {
                    String::new()
                },
                if t.external {
                    "External".into()
                } else {
                    String::new()
                },
            ])
            .into(),
            selected: t.selected,
        };
        self.player.audio_tracks = tracks
            .iter()
            .filter(|t| t.kind == "audio")
            .map(describe)
            .collect();
        let subs: Vec<Choice> = tracks
            .iter()
            .filter(|t| t.kind == "sub")
            .map(describe)
            .collect();
        let off = Choice {
            id: "no".into(),
            label: "Off".into(),
            sub: "".into(),
            selected: !subs.iter().any(|s| s.selected),
        };
        self.player.sub_tracks = std::iter::once(off).chain(subs).collect();
    }

    pub fn stop_playback(&mut self, cx: &mut Context<Self>) {
        if !self.playing() {
            return self.abort_opening(cx);
        }
        log::info!(
            "stop playback at {:.1}s / {:.1}s",
            self.player.time,
            self.player.duration
        );
        self.trim_images(false, cx);
        self.show_cursor();
        self.report("stopped", cx);
        self.end_transcode(cx);
        let p = &mut self.player;
        p.active = false;
        p.key.clear();
        p.server = None;
        p.mini = false;
        p.panel = None;
        p.hide_task = None;
        p.queue.clear();
        p.queue_show = Default::default();
        if let Some(mpv) = &p.mpv {
            mpv.command(&["stop"]);
        }
        if let Some(h) = &p.host {
            h.set_visible(false);
        }
        self.watch_feed(true);
        match self.nav.last().cloned() {
            Some(Nav::Home) => {
                self.navigate(Nav::Home, false, cx);
                self.load_home(cx);
            }
            Some(nav) => self.navigate(nav, false, cx),
            None => {}
        }
    }

    pub fn mpv(&self, f: impl FnOnce(&mpv::Mpv)) {
        if self.playing()
            && let Some(m) = &self.player.mpv
        {
            f(m);
        }
    }

    pub fn seek_to(&self, t: f64) {
        if self.watch_holds() {
            return self.watch_call(|wt| wt.seek((t.max(0.0) * 1000.0) as i64));
        }
        self.mpv(|m| m.command(&["seek", &t.to_string(), "absolute"]));
    }

    pub fn preview_seek(&self, t: f64) {
        if !self.watch_holds() {
            self.mpv(|m| m.command(&["seek", &t.to_string(), "absolute+keyframes"]));
        }
    }

    pub fn seek_by(&self, d: f64) {
        if self.watch_holds() {
            return self.seek_to(self.player.time + d);
        }
        self.mpv(|m| m.command(&["seek", &d.to_string(), "relative"]));
    }

    pub fn toggle_pause(&self) {
        if self.watch_holds() {
            return self.watch_call(|wt| {
                if self.player.paused {
                    wt.play()
                } else {
                    wt.pause()
                }
            });
        }
        self.mpv(|m| m.command(&["cycle", "pause"]));
    }

    pub fn set_volume(&mut self, v: f64) {
        let v = v.clamp(0.0, 100.0).round();
        if v == self.player.volume.round() && !self.player.muted {
            return;
        }
        let unmute = self.player.muted;
        self.player.volume = v;
        self.player.muted = false;
        self.mpv(|m| {
            if unmute {
                m.set("mute", "no");
            }
            m.set("volume", &v.to_string());
        });
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        self.mpv(|m| m.command(&["cycle", "mute"]));
        self.player.muted = !self.player.muted;
        self.hint_volume(cx);
    }

    pub fn hint(&mut self, icon: Icon, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.player.hint = Some((icon, text.into()));
        self.player.hint_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(HINT_FOR).await;
            this.update(cx, |this, cx| {
                this.player.hint = None;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn hint_volume(&mut self, cx: &mut Context<Self>) {
        let p = &self.player;
        let icon = if p.muted || p.volume == 0.0 {
            Icon::VolumeOff
        } else {
            Icon::Volume
        };
        let text = if p.muted {
            "Muted".to_owned()
        } else {
            format!("{}%", p.volume.round())
        };
        self.hint(icon, text, cx);
    }

    pub fn tick_clock(&mut self, cx: &mut Context<Self>) {
        if self.player.clock_task.is_some() {
            return;
        }
        let into_minute = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64 % 60_000);
        self.player.clock_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(60_000 - into_minute))
                .await;
            this.update(cx, |this, cx| {
                this.player.clock_task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn add_external_subtitle(&mut self, url: String, title: String, lang: String) {
        self.mpv(|m| m.command(&["sub-add", &url, "select", &title, &lang]));
        self.player.tracks_picked = true;
        self.player.external_subs.push(([url, title, lang], false));
    }

    pub fn can_search_subtitles(&self) -> bool {
        !self.player.local
            && self
                .player
                .server
                .as_ref()
                .is_some_and(|s| s.can_search_subtitles())
    }

    pub fn preferred_subtitle_language(&self) -> &str {
        &self.player.languages.subtitles
    }

    pub fn select_track(&mut self, kind: &str, id: &str) {
        self.player.tracks_picked = true;
        self.mpv(|m| m.set(kind, id));
    }

    fn auto_tracks(&self) {
        let Some(mpv) = self
            .player
            .mpv
            .as_ref()
            .filter(|_| !self.player.tracks_picked)
        else {
            return;
        };
        let tracks: Vec<MpvTrack> = mpv
            .get("track-list")
            .and_then(|j| serde_json::from_str(&j).ok())
            .unwrap_or_default();
        let of = |kind: &str| tracks.iter().filter(|t| t.kind == kind).collect::<Vec<_>>();
        let (audio, subs) = (of("audio"), of("sub"));
        let external_forced = |t: &MpvTrack| {
            self.player
                .external_subs
                .iter()
                .any(|(s, forced)| *forced && t.external_filename.as_deref() == Some(s[0].as_str()))
        };
        let info = |list: &[&MpvTrack]| {
            list.iter()
                .map(|t| TrackInfo {
                    language: t.lang.clone().unwrap_or_default(),
                    forced: t.forced || external_forced(t),
                    is_default: t.default,
                })
                .collect::<Vec<_>>()
        };
        let (a, s) = prefs::pick_with(
            &info(&audio),
            &info(&subs),
            &self.player.languages,
            self.player.rule.as_ref(),
        );
        log::debug!(
            "auto tracks for {:?}: audio {a:?}, subtitles {s:?}",
            self.player.languages
        );
        if let Some(i) = a {
            mpv.set("aid", &audio[i].id.to_string());
        }
        mpv.set("sid", &s.map_or("no".into(), |i| subs[i].id.to_string()));
    }

    pub fn apply_sub_style(&self) {
        let Some(m) = &self.player.mpv else { return };
        let s = &self.settings;
        let back = s.subtitle_background.min(100) * 255 / 100;
        let style = [
            ("sub-scale", (s.subtitle_size as f64 / 100.0).to_string()),
            ("sub-color", format!("#{}", s.subtitle_color)),
            ("sub-border-size", s.subtitle_outline.to_string()),
            ("sub-border-color", "#000000".into()),
            ("sub-back-color", format!("#{back:02X}000000")),
            (
                "sub-border-style",
                if back > 0 {
                    "background-box"
                } else {
                    "outline-and-shadow"
                }
                .into(),
            ),
            ("sub-pos", s.subtitle_position.min(100).to_string()),
        ];
        for (k, v) in style {
            m.set(k, &v);
        }
    }
}
