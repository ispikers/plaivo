use crate::app::{Card, Choice, Nav, Page, Plaivo, card_for, join};
use crate::plex::{self, Ctx, Item, Media, Quality, Server, Source, resolve_playable};
use crate::prefs::{self, LanguagePrefs, TrackInfo};
use crate::settings::{Scale, SkipMode};
use crate::ui::theme::{self, Icon};
use crate::video_host::{self, Region};
use crate::downloads::Local;
use crate::ui::services::play_state;
use crate::{mpv, shaders, store};
use plaivo_core::trackers::{self, PlayState};
use futures::StreamExt;
use gpui::{Context, ScrollHandle, SharedString, Task};
use serde::Deserialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CONTROLS_FOR: Duration = Duration::from_secs(3);
const HINT_FOR: Duration = Duration::from_millis(1200);

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
    Settings,
    Quality,
    Speed,
    Output,
    Shader,
    Info,
    Queue,
}

pub const SPEEDS: &[f64] = &[0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

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
    tracks_picked: bool,
    pub media: Media,
    pub skips: Vec<Skip>,
    credits: Option<f64>,
    up_next_cancelled: bool,
    up_next_anchor: Option<f64>,
    advancing: bool,
    pub shader: String,
    pub shader_applied: &'static str,
    shader_failed: &'static str,
    transcode: Option<String>,
    local: bool,
    pub paused: bool,
    last_report: Option<Instant>,
    pub tracker: Option<trackers::Media>,
    external_subs: Vec<([String; 3], bool)>,
    pub title: SharedString,
    pub title_link: Option<Nav>,
    pub crumbs: Vec<Crumb>,
    pub queue: Vec<Card>,
    queue_show: String,
    pub queue_scroll: ScrollHandle,
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
    pub hint: Option<(Icon, SharedString)>,
    hint_task: Option<Task<()>>,
    clock_task: Option<Task<()>>,
}

fn queue_card(e: &Item, server: &str) -> Card {
    Card { meta: e.duration.map(plex::fmt_duration).unwrap_or_default().into(), ..card_for(e, true, server) }
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
        let device = if self.settings.audio_device.is_empty() { "auto" } else { self.settings.audio_device.as_str() };
        let [aspect, panscan] = self.settings.scale.mpv();
        let opts = [
            aspect,
            panscan,
            ("audio-device", device),
            ("wid", wid.as_str()),
            ("vo", "gpu-next"),
            ("hwdec", "auto-safe"),
            ("target-colorspace-hint", "yes"),
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
            ("gpu-context", if cfg!(target_os = "macos") { "macvk" } else { "auto" }),
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
        log::info!("play request {server}/{key}");
        if server == plex::DISCOVER_ID {
            return self.toast("This title isn't in any of your libraries.", cx);
        }
        if self.playing() && self.player.key == key && self.player.server.as_ref().is_some_and(|s| s.id == server) {
            if self.player.paused {
                self.mpv(|m| m.set("pause", "no"));
            }
            return self.expand_player(cx);
        }
        let downloaded = || self.offline.iter().any(|(s, it)| *s == server && it.rating_key == key);
        let Some(srv) = self.server(&server).or_else(|| downloaded().then(|| Server { id: server.clone(), ..Default::default() })) else { return };
        let c = self.client.clone();
        let (token, local, downloads) = (self.account.token.clone(), self.settings.languages(), self.downloads.clone());
        Self::bg(
            cx,
            move || {
                if let Some(d) = &downloads
                    && let Some(file) = d.local(&srv.id, &key)
                    && let Some(it) = d.item(&srv.id, &key)
                {
                    return (srv, Ok(it), local, Some(file));
                }
                let it = resolve_playable(&srv, &Ctx::new(c.clone(), Source::Live), &key);
                let languages = prefs::for_server(&c, &srv, token.as_deref(), &local);
                (srv, it, languages, None)
            },
            |this, (srv, r, languages, file), cx| match r {
                Ok(it) => {
                    this.player.languages = languages;
                    if let Err(e) = this.start_item(srv, it, file, cx) {
                        this.advance_failed();
                        this.toast(e, cx);
                    }
                }
                Err(e) => {
                    this.advance_failed();
                    this.toast(format!("Could not play: {e}"), cx);
                }
            },
        );
    }

    fn start_item(&mut self, srv: Server, it: Item, local: Option<Local>, cx: &mut Context<Self>) -> Result<(), String> {
        let part = it.media.first().and_then(|m| m.part.first()).ok_or("This item has no playable media")?;
        let offset = it.view_offset.unwrap_or(0) as f64 / 1000.0;
        let resume = offset > 5.0 && it.progress() < 0.95;
        let subs = match &local {
            Some(l) => l.subtitles.iter().map(|(file, st)| ([file.to_string_lossy().into_owned(), st.display_title.clone(), st.language_code.clone()], st.forced)).collect(),
            None => part
                .stream
                .iter()
                .filter(|st| st.stream_type == 3 && !st.key.is_empty())
                .map(|st| ([srv.media_url(&st.key), st.display_title.clone(), st.language_code.clone()], st.forced))
                .collect(),
        };
        let duration = it.duration.map(plex::fmt_duration).unwrap_or_default();
        let (title, subtitle) = match it.kind.as_str() {
            "episode" => (it.grandparent_title.clone(), join(&[it.episode_label(), it.title.clone(), duration])),
            _ => (it.title.clone(), join(&[it.year.map(|y| y.to_string()).unwrap_or_default(), duration])),
        };
        let media = it.media[0].clone();
        let (url, session) = match &local {
            Some(l) => (l.file.to_string_lossy().into_owned(), None),
            None => self.stream_url(&srv, &it.rating_key, &media),
        };
        self.player.server = Some(srv);
        self.launch(&url, &it.rating_key, &title, &subtitle, it.duration.unwrap_or(0) as f64 / 1000.0, resume.then_some(offset), subs, cx)?;
        self.player.local = local.is_some();
        self.resolve_tracker_media(&it, cx);
        let id = &self.player.server.as_ref().unwrap().id;
        let link = |key: &str, season: Option<&str>| (!key.is_empty()).then(|| Nav::Detail(id.clone(), key.to_owned(), season.map(str::to_owned)));
        let (title_link, crumbs): (_, Vec<Crumb>) = match it.kind.as_str() {
            "episode" => {
                let ep = link(&it.rating_key, None);
                let season = link(&it.grandparent_rating_key, Some(&it.parent_rating_key).filter(|k| !k.is_empty()).map(|k| k.as_str()));
                let parts = [
                    (it.parent_index.map(|s| format!("S{s}")), season),
                    (it.index.map(|e| format!("E{e}")), ep.clone()),
                    (Some(it.title.clone()), ep),
                    (it.duration.map(plex::fmt_duration), None),
                ];
                (link(&it.grandparent_rating_key, None), parts.into_iter().filter_map(|(t, n)| Some((t.filter(|t| !t.is_empty())?.into(), n))).collect())
            }
            _ => (link(&it.rating_key, None), subtitle.split(" · ").filter(|s| !s.is_empty()).map(|s| (s.to_owned().into(), None)).collect()),
        };
        if it.kind == "episode" {
            self.load_queue(it.grandparent_rating_key.clone(), cx);
            if self.player.local && self.player.queue.is_empty() {
                let server = self.player.server.as_ref().map(|s| s.id.clone()).unwrap_or_default();
                self.player.queue = self.offline_episodes(&server, &it.grandparent_rating_key).into_iter().map(|e| queue_card(e, &server)).collect();
            }
        } else {
            self.player.queue.clear();
            self.player.queue_show.clear();
        }
        let p = &mut self.player;
        p.title_link = title_link;
        p.crumbs = crumbs;
        p.transcode = session;
        p.media = media;
        p.credits = it.marker.iter().find(|m| m.kind == "credits").map(|m| m.start_time_offset as f64 / 1000.0);
        p.skips = it
            .marker
            .iter()
            .filter_map(|m| {
                let label = match m.kind.as_str() {
                    "intro" => "Skip Intro",
                    "credits" => "Skip Credits",
                    "commercial" => "Skip Ad",
                    _ => return None,
                };
                let mode = self.settings.skip_mode(&m.kind);
                (mode != SkipMode::Off).then(|| Skip { label, start: m.start_time_offset as f64 / 1000.0, end: m.end_time_offset as f64 / 1000.0, auto: mode == SkipMode::Auto })
            })
            .collect();
        Ok(())
    }

    fn load_queue(&mut self, show: String, cx: &mut Context<Self>) {
        let Some(srv) = self.player.server.clone() else { return };
        if self.player.queue_show != show {
            self.player.queue.clear();
            self.player.queue_show = show.clone();
        }
        let c = self.client.clone();
        Self::bg(
            cx,
            move || srv.all_episodes(&Ctx::new(c, Source::Network), &show).map(|eps| (srv.id, show, eps)),
            |this, r, _| match r {
                Ok((server, show, eps)) if this.player.queue_show == show => {
                    this.player.queue = eps.iter().map(|e| queue_card(e, &server)).collect();
                }
                Ok(_) => {}
                Err(e) => log::warn!("could not load episode queue: {e}"),
            },
        );
    }

    pub fn queue_index(&self) -> Option<usize> {
        self.player.queue.iter().position(|c| c.key.as_ref() == self.player.key)
    }

    pub fn neighbor(&self, step: isize) -> Option<Card> {
        let i = self.queue_index()?.checked_add_signed(step)?;
        self.player.queue.get(i).cloned()
    }

    pub fn play_card(&mut self, card: Card, cx: &mut Context<Self>) {
        if card.key.as_ref() == self.player.key {
            return;
        }
        self.report("stopped");
        self.player.buffering = true;
        self.player.advancing = true;
        self.play(card.server.to_string(), card.key.to_string(), cx);
    }

    fn advance_failed(&mut self) {
        let p = &mut self.player;
        p.up_next_cancelled |= p.advancing;
        p.advancing = false;
    }

    pub fn up_next(&mut self) -> Option<(Card, f64)> {
        let p = &mut self.player;
        if p.up_next_cancelled || p.advancing {
            return None;
        }
        let left = self.settings.up_next_in(p.time, p.duration, p.credits, &mut p.up_next_anchor)?;
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
        let t = self.player.time;
        let Some(s) = self.player.skips.iter_mut().find(|s| s.auto && t >= s.start && t < s.end - 1.0) else { return };
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
        let hdr = || shaders::is_hdr(&m.get("video-params/primaries").unwrap_or_default(), &m.get("video-params/gamma").unwrap_or_default());
        let preset = shaders::find(&self.player.shader).filter(|p| p.id != shaders::NVSCALER || !hdr());
        let id = preset.map_or("", |p| p.id);
        if id == self.player.shader_applied || id == self.player.shader_failed {
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
            m.command(&["change-list", "glsl-shaders", "append", &dir.join(f).to_string_lossy()]);
        }
        self.player.shader_applied = id;
    }

    pub fn play_neighbor(&mut self, step: isize, cx: &mut Context<Self>) {
        if let Some(card) = self.neighbor(step) {
            self.play_card(card, cx);
        }
    }

    fn stream_url(&self, srv: &Server, key: &str, media: &Media) -> (String, Option<String>) {
        match Quality::find(self.settings.quality).filter(|q| !q.covers(media)) {
            Some(q) => {
                let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
                let session = format!("plaivo-{nanos:x}");
                log::info!("transcoding {key} at {} kbps", q.kbps);
                (srv.transcode_url(&self.client, key, media, q, &session), Some(session))
            }
            None => (srv.media_url(&media.part[0].key), None),
        }
    }

    fn end_transcode(&mut self) {
        if let (Some(session), Some(srv)) = (self.player.transcode.take(), self.player.server.clone()) {
            let c = self.client.clone();
            std::thread::spawn(move || srv.stop_transcode(&c, &session));
        }
    }

    pub fn set_quality(&mut self, kbps: u32, cx: &mut Context<Self>) {
        if kbps == self.settings.quality {
            return;
        }
        self.settings.quality = kbps;
        self.settings.save();
        let Some(srv) = self.player.server.clone().filter(|_| self.playing() && !self.player.local && !self.player.media.part.is_empty()) else { return };
        let (url, session) = self.stream_url(&srv, &self.player.key.clone(), &self.player.media.clone());
        self.end_transcode();
        let p = &mut self.player;
        p.transcode = session;
        p.buffering = true;
        p.cached = 0.0;
        let at = p.time.to_string();
        self.mpv(|m| {
            m.set("start", &at);
            m.command(&["loadfile", &url, "replace"]);
        });
        cx.notify();
    }

    pub fn set_scale(&mut self, scale: Scale) {
        self.settings.scale = scale;
        self.settings.save();
        if let Some(m) = &self.player.mpv {
            for (k, v) in scale.mpv() {
                m.set(k, v);
            }
        }
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.player.speed = speed;
        self.mpv(|m| m.set("speed", &speed.to_string()));
    }

    pub fn nudge_delay(&mut self, audio: bool, ms: f64) {
        let p = &mut self.player;
        let v = if audio { &mut p.audio_delay } else { &mut p.sub_delay };
        *v = if ms == 0.0 { 0.0 } else { (*v + ms).clamp(-10_000.0, 10_000.0) };
        let secs = (*v / 1000.0).to_string();
        self.mpv(|m| m.set(if audio { "audio-delay" } else { "sub-delay" }, &secs));
    }

    pub fn set_audio_device(&mut self, name: &str) {
        self.settings.audio_device = if name == "auto" { String::new() } else { name.to_owned() };
        self.settings.save();
        if let Some(m) = &self.player.mpv {
            m.set("audio-device", name);
        }
    }

    pub fn active_skip(&self) -> Option<&Skip> {
        let t = self.player.time;
        self.player.skips.iter().find(|s| !s.auto && t >= s.start && t < s.end - 1.0)
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
                    self.player.queue_scroll.scroll_to_item(i);
                }
            }
            Panel::Output => {
                #[derive(Deserialize)]
                struct Device {
                    name: String,
                    description: String,
                }
                let list: Vec<Device> = self.player.mpv.as_ref().and_then(|m| m.get("audio-device-list")).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
                let current = if self.settings.audio_device.is_empty() { "auto" } else { self.settings.audio_device.as_str() };
                self.player.devices = list
                    .into_iter()
                    .map(|d| Choice { selected: d.name == current, id: d.name.into(), label: d.description.into(), sub: "".into() })
                    .collect();
            }
            _ => {}
        }
        self.player.panel = Some(panel);
        self.wake_controls(cx);
    }

    fn file_info(&self) -> Vec<(SharedString, SharedString)> {
        let Some(m) = &self.player.mpv else { return Vec::new() };
        let get = |k: &str| m.get(k).unwrap_or_default();
        let media = &self.player.media;
        let file = media.part.first().and_then(|p| p.file.rsplit(['/', '\\']).next()).unwrap_or_default().to_owned();
        let mode = match (&self.player.transcode, Quality::find(self.settings.quality)) {
            _ if self.player.local => "Downloaded".into(),
            (Some(_), Some(q)) => format!("Transcode · {} · {}", q.label, plex::fmt_bitrate(q.kbps as u64)),
            _ => "Direct play".into(),
        };
        let fps = get("container-fps").parse::<f64>().map(|f| format!("{f:.3} fps").replace(".000", "")).unwrap_or_default();
        let kbps = |k: &str| get(k).parse::<f64>().ok().filter(|b| *b > 0.0).map(|b| plex::fmt_bitrate((b / 1000.0) as u64)).unwrap_or_default();
        let size = match (get("video-params/w"), get("video-params/h")) {
            (w, h) if !w.is_empty() && !h.is_empty() => format!("{w}×{h}"),
            _ => String::new(),
        };
        let hw = get("hwdec-current");
        let rate = get("audio-params/samplerate").parse::<f64>().map(|r| format!("{} kHz", r / 1000.0)).unwrap_or_default();
        let rows = [
            ("Playback", mode),
            ("File", file),
            ("Container", join(&[get("file-format"), media.bitrate.map(plex::fmt_bitrate).unwrap_or_default()])),
            ("Video", join(&[get("video-format").to_uppercase(), size, fps, kbps("video-bitrate")])),
            ("Color", join(&[get("video-params/pixelformat"), get("video-params/primaries"), get("video-params/gamma")])),
            ("Decoder", if hw.is_empty() || hw == "no" { "Software".into() } else { format!("Hardware ({hw})") }),
            ("Audio", join(&[get("audio-codec-name").to_uppercase(), get("audio-params/hr-channels"), rate, kbps("audio-bitrate")])),
            ("Output", get("current-ao")),
        ];
        rows.into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.into(), v.into())).collect()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn launch(&mut self, url: &str, key: &str, title: &str, subtitle: &str, duration: f64, start_at: Option<f64>, subs: Vec<([String; 3], bool)>, cx: &mut Context<Self>) -> Result<(), String> {
        log::info!("launch '{title}' key={key} start={start_at:?} subs={}", subs.len());
        self.ensure_player(cx)?;
        self.end_transcode();
        let p = &mut self.player;
        p.active = true;
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
        p.shader = self.settings.shader.clone();
        p.local = false;
        p.last_report = None;
        p.tracker = None;
        p.external_subs = subs;
        p.title = title.to_owned().into();
        p.title_link = None;
        p.crumbs = (!subtitle.is_empty()).then(|| (subtitle.to_owned().into(), None)).into_iter().collect();
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
        self.player.controls = true;
        self.player.hide_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CONTROLS_FOR).await;
            this.update(cx, |this, cx| {
                if !this.player.paused && this.player.panel.is_none() {
                    this.player.controls = false;
                    cx.notify();
                }
            })
            .ok();
        }));
        cx.notify();
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
    }

    pub fn minimize_player(&mut self, cx: &mut Context<Self>) {
        if !self.playing() {
            return;
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
    }

    pub fn playing(&self) -> bool {
        self.player.active
    }

    fn report(&mut self, state: &str) {
        let Some(srv) = self.player.server.clone() else { return };
        if !self.playing() || self.player.key.is_empty() {
            return;
        }
        let c = self.client.clone();
        let p = &mut self.player;
        p.last_report = Some(Instant::now());
        let offline = self.downloads.clone().filter(|_| p.local);
        let (key, state, t, d, session) = (p.key.clone(), state.to_owned(), (p.time * 1000.0) as u64, (p.duration * 1000.0) as u64, p.transcode.clone());
        if let Some(m) = &p.tracker {
            self.services.trackers.on_progress(m, play_state(&state), t, d);
        }
        std::thread::spawn(move || {
            let synced = srv.timeline(&c, &key, &state, t, d, session.as_deref());
            if let Some(downloads) = offline.filter(|_| state == "stopped" || !synced) {
                downloads.record_progress(&srv.id, &key, &state, t, d, synced);
            }
        });
    }

    pub fn report_stopped_blocking(&self) {
        let p = &self.player;
        if p.active
            && !p.key.is_empty()
            && let Some(srv) = &p.server
        {
            let (t, d) = ((p.time * 1000.0) as u64, (p.duration * 1000.0) as u64);
            if let Some(m) = &p.tracker {
                self.services.trackers.on_progress(m, PlayState::Stopped, t, d);
            }
            let synced = srv.timeline(&self.client, &p.key, "stopped", t, d, p.transcode.as_deref());
            if let Some(downloads) = self.downloads.as_ref().filter(|_| p.local) {
                downloads.record_progress(&srv.id, &p.key, "stopped", t, d, synced);
            }
        }
    }

    fn on_mpv(&mut self, e: mpv::Event, cx: &mut Context<Self>) {
        match e {
            mpv::Event::Time(t) => {
                self.player.time = t;
                if self.player.last_report.is_none_or(|r| r.elapsed() > Duration::from_secs(10)) {
                    let state = if self.player.paused { "paused" } else { "playing" };
                    self.report(state);
                }
                self.auto_skip(cx);
                if self.up_next().is_some_and(|(_, left)| left <= 0.0) {
                    self.play_up_next(cx);
                }
            }
            mpv::Event::Duration(d) => self.player.duration = d,
            mpv::Event::Cached(t) => self.player.cached = t,
            mpv::Event::Pause(p) => {
                self.player.paused = p;
                self.report(if p { "paused" } else { "playing" });
                if !p {
                    self.wake_controls(cx);
                }
            }
            mpv::Event::Buffering(b) => self.player.buffering = b,
            mpv::Event::Volume(v) => self.player.volume = v,
            mpv::Event::Mute(m) => self.player.muted = m,
            mpv::Event::Loaded => {
                self.player.buffering = false;
                if let Some(mpv) = &self.player.mpv {
                    for ([url, title, lang], _) in &self.player.external_subs {
                        mpv.command(&["sub-add", url, "auto", title, lang]);
                    }
                }
                self.auto_tracks();
            }
            mpv::Event::Tracks => self.refresh_tracks(),
            mpv::Event::Started => {
                self.player.video_ready = true;
                self.sync_shader(cx);
            }
            mpv::Event::Ended { eof, error } => {
                log::info!("playback ended eof={eof} error={error:?}");
                if self.playing() && !self.player.advancing && (eof || error.is_some()) {
                    if let Some(e) = error {
                        self.toast(format!("Playback error: {e}"), cx);
                    }
                    match self.neighbor(1).filter(|_| eof && self.settings.autoplay_next && !self.player.up_next_cancelled) {
                        Some(next) => self.play_card(next, cx),
                        None => self.stop_playback(cx),
                    }
                }
            }
        }
        cx.notify();
    }

    fn refresh_tracks(&mut self) {
        let Some(mpv) = &self.player.mpv else { return };
        let tracks: Vec<MpvTrack> = mpv.get("track-list").and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        let describe = |t: &MpvTrack| Choice {
            id: t.id.to_string().into(),
            label: t.title.clone().or_else(|| t.lang.clone()).unwrap_or_else(|| format!("Track {}", t.id)).into(),
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
                if t.forced { "Forced".into() } else { String::new() },
                if t.external { "External".into() } else { String::new() },
            ])
            .into(),
            selected: t.selected,
        };
        self.player.audio_tracks = tracks.iter().filter(|t| t.kind == "audio").map(describe).collect();
        let subs: Vec<Choice> = tracks.iter().filter(|t| t.kind == "sub").map(describe).collect();
        let off = Choice { id: "no".into(), label: "Off".into(), sub: "".into(), selected: !subs.iter().any(|s| s.selected) };
        self.player.sub_tracks = std::iter::once(off).chain(subs).collect();
    }

    pub fn stop_playback(&mut self, cx: &mut Context<Self>) {
        if !self.playing() {
            return;
        }
        log::info!("stop playback at {:.1}s / {:.1}s", self.player.time, self.player.duration);
        self.trim_images(false, cx);
        self.show_cursor();
        self.report("stopped");
        self.end_transcode();
        let p = &mut self.player;
        p.active = false;
        p.key.clear();
        p.server = None;
        p.mini = false;
        p.panel = None;
        p.hide_task = None;
        p.queue.clear();
        p.queue_show.clear();
        if let Some(mpv) = &p.mpv {
            mpv.command(&["stop"]);
        }
        if let Some(h) = &p.host {
            h.set_visible(false);
        }
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
        self.mpv(|m| m.command(&["seek", &t.to_string(), "absolute"]));
    }

    pub fn seek_by(&self, d: f64) {
        self.mpv(|m| m.command(&["seek", &d.to_string(), "relative"]));
    }

    pub fn toggle_pause(&self) {
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
        let icon = if p.muted || p.volume == 0.0 { Icon::VolumeOff } else { Icon::Volume };
        let text = if p.muted { "Muted".to_owned() } else { format!("{}%", p.volume.round()) };
        self.hint(icon, text, cx);
    }

    pub fn tick_clock(&mut self, cx: &mut Context<Self>) {
        if self.player.clock_task.is_some() {
            return;
        }
        let into_minute = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64 % 60_000);
        self.player.clock_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(60_000 - into_minute)).await;
            this.update(cx, |this, cx| {
                this.player.clock_task = None;
                cx.notify();
            })
            .ok();
        }));
    }

    pub fn select_track(&mut self, kind: &str, id: &str) {
        self.player.tracks_picked = true;
        self.mpv(|m| m.set(kind, id));
    }

    fn auto_tracks(&self) {
        let Some(mpv) = self.player.mpv.as_ref().filter(|_| !self.player.tracks_picked) else { return };
        let tracks: Vec<MpvTrack> = mpv.get("track-list").and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
        let of = |kind: &str| tracks.iter().filter(|t| t.kind == kind).collect::<Vec<_>>();
        let (audio, subs) = (of("audio"), of("sub"));
        let external_forced = |t: &MpvTrack| self.player.external_subs.iter().any(|(s, forced)| *forced && t.external_filename.as_deref() == Some(s[0].as_str()));
        let info = |list: &[&MpvTrack]| list.iter().map(|t| TrackInfo { language: t.lang.clone().unwrap_or_default(), forced: t.forced || external_forced(t), is_default: t.default }).collect::<Vec<_>>();
        let (a, s) = prefs::pick(&info(&audio), &info(&subs), &self.player.languages);
        log::debug!("auto tracks for {:?}: audio {a:?}, subtitles {s:?}", self.player.languages);
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
            ("sub-border-style", if back > 0 { "background-box" } else { "outline-and-shadow" }.into()),
            ("sub-pos", s.subtitle_position.min(100).to_string()),
        ];
        for (k, v) in style {
            m.set(k, &v);
        }
    }
}
