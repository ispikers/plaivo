use crate::app::{Page, Plaivo};
use crate::plex::{Client, Ctx, Item, Server, Source};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::theme::Icon;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use gpui::{AppContext, Context, Entity, SharedString};
use plaivo_core::watch_together::{
    Connection, EndReason, Event, Match, MatchState, Media, MediaFile, MediaKind, PartyError, PartyState, PlaivoExt, PlayerCommand, PlayerFeed, Relays, Series, Update, WatchTogether, ids_from_guids,
    normalize_code,
};
use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

const CLIENT: &str = concat!("Plaivo/", env!("CARGO_PKG_VERSION"));
const FEED_EVERY: Duration = Duration::from_millis(400);
const SAME_LENGTH_MS: i64 = 2000;
const CANDIDATES: usize = 5;
const NAME_MAX: usize = 32;

pub struct Watch {
    wt: Option<WatchTogether>,
    generation: u64,
    tx: UnboundedSender<(u64, Update)>,
    pub state: PartyState,
    pub starting: bool,
    creating: bool,
    pub error: Option<SharedString>,
    epoch: Option<i64>,
    file: Option<(String, String)>,
    share: Option<(Media, i64)>,
    pub unavailable: bool,
    last_feed: Option<Instant>,
    attached: bool,
    pub custom_relay: bool,
    pub relay_error: Option<SharedString>,
    pub code: Entity<TextInput>,
    pub name: Entity<TextInput>,
    pub relay: Entity<TextInput>,
}

impl Watch {
    pub fn new(name: &str, relay: &str, cx: &mut Context<Plaivo>) -> Self {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(u64, Update)>();
        cx.spawn(async move |this, cx| {
            while let Some((generation, update)) = rx.next().await {
                if this.update(cx, |this, cx| this.on_watch(generation, update, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
        let input = |placeholder: &str, text: &str, cx: &mut Context<Plaivo>| {
            cx.new(|cx| {
                let mut input = TextInput::new(placeholder, cx);
                input.set_text(text, cx);
                input
            })
        };
        let code = input("Room code, like ABC-DEF", "", cx);
        let name_input = input("Your Plex or Jellyfin name", name, cx);
        let relay_input = input("https://relay.example.com", relay, cx);
        cx.subscribe(&code, |this: &mut Plaivo, _, ev: &InputEvent, cx| match ev {
            InputEvent::Submit(code) => this.watch_join(code.to_string(), cx),
            InputEvent::Changed(_) => {
                if this.watch.error.take().is_some() {
                    cx.notify();
                }
            }
        })
        .detach();
        cx.subscribe(&name_input, |this: &mut Plaivo, _, ev: &InputEvent, _| {
            if let InputEvent::Changed(name) = ev {
                this.settings.watch_name = name.trim().to_owned();
                this.settings.save();
            }
        })
        .detach();
        cx.subscribe(&relay_input, |this: &mut Plaivo, _, ev: &InputEvent, cx| {
            if let InputEvent::Changed(url) = ev {
                this.set_relay(url.trim(), cx);
            }
        })
        .detach();
        Self {
            wt: None,
            generation: 0,
            tx,
            state: PartyState::default(),
            starting: false,
            creating: false,
            error: None,
            epoch: None,
            file: None,
            share: None,
            unavailable: false,
            last_feed: None,
            attached: false,
            custom_relay: !relay.is_empty(),
            relay_error: None,
            code,
            name: name_input,
            relay: relay_input,
        }
    }

    pub fn in_room(&self) -> bool {
        matches!(self.state.connection, Connection::Connected | Connection::Reconnecting)
    }

    pub fn busy(&self) -> bool {
        self.starting || self.in_room() || self.state.connection == Connection::Connecting
    }

    pub fn shutdown(&mut self) {
        self.generation += 1;
        drop(self.wt.take());
    }
}

pub fn relays(setting: &str) -> Relays {
    if setting.is_empty() { Relays::public() } else { Relays::custom(setting).unwrap_or_else(Relays::public) }
}

pub fn clean(s: &str, max: usize) -> String {
    let bidi = |c: char| matches!(c, '\u{61C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c| !c.is_control() && !bidi(*c)).take(max).collect()
}

pub fn pick_name(setting: &str, account: impl FnOnce() -> Option<String>, jellyfin: Option<&str>) -> String {
    let usable = |s: &str| Some(clean(s, NAME_MAX)).filter(|s| !s.is_empty());
    usable(setting).or_else(|| account().and_then(|n| usable(&n))).or_else(|| jellyfin.and_then(usable)).unwrap_or_else(|| "Plaivo".into())
}

fn item_ids(it: &Item) -> BTreeMap<String, String> {
    ids_from_guids(it.guids.iter().map(|g| g.id.as_str()).chain([it.guid.as_str()]))
}

pub fn same_title(wanted: &BTreeMap<String, String>, it: &Item) -> bool {
    let ours = item_ids(it);
    wanted.iter().any(|(ns, id)| ours.get(ns) == Some(id))
}

pub fn describe(server: &str, it: &Item, show: Option<&Item>) -> Option<Media> {
    let kind = match it.kind.as_str() {
        "movie" => MediaKind::Movie,
        "episode" => MediaKind::Episode,
        _ => MediaKind::Video,
    };
    let episode = kind == MediaKind::Episode;
    let title = Some(clean(&it.title, usize::MAX)).filter(|t| !t.is_empty()).unwrap_or_else(|| it.index.map_or("Untitled".into(), |n| format!("Episode {n}")));
    let series = episode.then(|| Series {
        title: clean(show.map_or(&it.grandparent_title, |s| &s.title), usize::MAX),
        year: show.and_then(|s| s.year),
        ids: show.map(item_ids).unwrap_or_default(),
    });
    let name = it.media.first().and_then(|m| m.part.first()).and_then(|p| p.file.rsplit(['/', '\\']).next()).map(|n| clean(n, usize::MAX)).filter(|n| !n.is_empty());
    let mut media = Media {
        kind,
        title,
        year: it.year.filter(|_| !episode),
        ids: if kind == MediaKind::Video { BTreeMap::new() } else { item_ids(it) },
        series,
        season: it.parent_index.filter(|_| episode),
        episode: it.index.filter(|_| episode),
        duration_ms: it.duration.unwrap_or(0) as i64,
        file: name.map(|name| MediaFile { name: Some(name), ..Default::default() }),
        ext: Default::default(),
    };
    media.set_plaivo(&PlaivoExt { server: server.into(), item: it.rating_key.clone() });
    if media.check(true).is_err() {
        media.file = None;
    }
    media.check(true).ok().map(|_| media)
}

pub fn match_for(ours_ms: Option<u64>, theirs_ms: i64) -> Match {
    match ours_ms.filter(|d| *d > 0) {
        Some(d) => {
            let delta = d as i64 - theirs_ms;
            Match { state: if delta.abs() <= SAME_LENGTH_MS { MatchState::Matched } else { MatchState::Mismatched }, delta_ms: Some(delta) }
        }
        None => Match { state: MatchState::Matched, delta_ms: None },
    }
}

fn find_copy(media: &Media, servers: &[Server], c: &Client) -> Option<(String, Item)> {
    let ctx = Ctx::new(c.clone(), Source::Live);
    if let Some(hint) = media.plaivo()
        && let Some(srv) = servers.iter().find(|s| s.id == hint.server)
        && let Ok(it) = srv.metadata(&ctx, &hint.item)
    {
        return Some((srv.id.clone(), it));
    }
    let (title, ids, kind) = match (media.kind, &media.series) {
        (MediaKind::Movie, _) => (&media.title, &media.ids, "movie"),
        (MediaKind::Episode, Some(s)) => (&s.title, &s.ids, "show"),
        _ => return None,
    };
    if ids.is_empty() {
        return None;
    }
    for srv in servers {
        let Ok(hits) = srv.search(&ctx, title) else { continue };
        let mut seen = HashSet::new();
        let candidates = hits.into_iter().map(|(it, _)| it.as_show()).filter(|it| it.kind == kind && seen.insert(it.rating_key.clone()));
        for hit in candidates.take(CANDIDATES) {
            let Ok(found) = srv.metadata(&ctx, &hit.rating_key) else { continue };
            if !same_title(ids, &found) {
                continue;
            }
            if kind == "movie" {
                return Some((srv.id.clone(), found));
            }
            let episodes = srv.all_episodes(&ctx, &found.rating_key).unwrap_or_default();
            if let Some(ep) = episodes.into_iter().find(|e| e.parent_index == media.season && e.index == media.episode) {
                return Some((srv.id.clone(), ep));
            }
        }
    }
    None
}

fn error_text(e: &PartyError) -> Option<&'static str> {
    Some(match e.code.as_str() {
        "cancelled" => return None,
        "invalid_code" => "That isn't a valid room code.",
        "room_not_found" => "No room with that code.",
        "room_full" => "That room is full.",
        "room_locked" | "locked" => "That room is locked.",
        "relay_full" => "The Watch Together servers are busy. Try again soon.",
        "unreachable" => "Couldn't reach the Watch Together server.",
        "forbidden" | "not_host" => "Only the host can do that in this room.",
        _ => "Watch Together ran into a problem.",
    })
}

fn end_text(reason: EndReason) -> Option<&'static str> {
    Some(match reason {
        EndReason::Left => return None,
        EndReason::Ended => "The room has ended.",
        EndReason::Kicked => "You were removed from the room.",
        EndReason::Expired => "The room expired.",
        EndReason::Replaced => "You joined this room from another device.",
        EndReason::Lost => "Lost the connection to the room.",
    })
}

fn action_text(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "pause" => "paused",
        "play" => "resumed",
        "seek" => "skipped",
        "rate" => "changed the speed",
        "media" => "changed what's playing",
        _ => return None,
    })
}

impl Plaivo {
    pub fn watch_create(&mut self, cx: &mut Context<Self>) {
        self.watch_start(None, cx);
    }

    pub fn watch_join(&mut self, code: String, cx: &mut Context<Self>) {
        match normalize_code(&code) {
            Some(code) => self.watch_start(Some(code), cx),
            None => {
                self.watch.error = Some("Enter the 6-character room code.".into());
                cx.notify();
            }
        }
    }

    fn watch_start(&mut self, code: Option<String>, cx: &mut Context<Self>) {
        let w = &mut self.watch;
        if w.busy() {
            return;
        }
        w.generation += 1;
        w.starting = true;
        w.creating = code.is_none();
        w.error = None;
        let generation = w.generation;
        let (old, tx) = (w.wt.take(), w.tx.clone());
        let (setting, relays, client, token) = (self.settings.watch_name.clone(), relays(&self.settings.watch_relay), self.client.clone(), self.account.token.clone());
        let jellyfin = self.account.jellyfin.first().map(|s| s.user.clone());
        Self::bg(
            cx,
            move || {
                drop(old);
                let name = pick_name(&setting, || client.account_name(token.as_deref()?).ok(), jellyfin.as_deref());
                let wt = WatchTogether::new(&name, CLIENT, relays, move |u| {
                    let _ = tx.unbounded_send((generation, u));
                })?;
                match code {
                    Some(code) => wt.join(&code, false),
                    None => wt.create(),
                }
                Ok(wt)
            },
            move |this, wt: Result<WatchTogether, PartyError>, _| {
                if this.watch.generation != generation {
                    std::thread::spawn(move || drop(wt));
                    return;
                }
                this.watch.starting = false;
                match wt {
                    Ok(wt) => this.watch.wt = Some(wt),
                    Err(e) => {
                        log::warn!("watch together: could not start: {e:?}");
                        this.watch.creating = false;
                        this.watch.error = Some("Couldn't start Watch Together. Check your display name in Settings.".into());
                    }
                }
            },
        );
        cx.notify();
    }

    pub fn watch_leave(&mut self, cx: &mut Context<Self>) {
        if self.watch.starting {
            self.watch.generation += 1;
            self.watch.starting = false;
            self.watch.creating = false;
        }
        if let Some(wt) = &self.watch.wt {
            wt.leave();
        }
        cx.notify();
    }

    pub fn watch_call(&self, f: impl FnOnce(&WatchTogether)) {
        if let Some(wt) = &self.watch.wt {
            f(wt);
        }
    }

    fn set_relay(&mut self, url: &str, cx: &mut Context<Self>) {
        let valid = url.is_empty() || Relays::custom(url).is_some();
        self.watch.relay_error = (!valid).then(|| "Enter the relay's address, starting with https:// or http://".into());
        if valid && self.settings.watch_relay != url {
            self.settings.watch_relay = url.to_owned();
            self.settings.save();
        }
        cx.notify();
    }

    pub fn use_public_relay(&mut self, public: bool, cx: &mut Context<Self>) {
        self.watch.custom_relay = !public;
        if public {
            self.watch.relay_error = None;
            self.settings.watch_relay.clear();
            self.settings.save();
        } else {
            let url = self.watch.relay.read(cx).text().trim().to_owned();
            self.set_relay(&url, cx);
        }
        cx.notify();
    }

    pub fn watch_together(&mut self, server: String, key: String, cx: &mut Context<Self>) {
        let w = &self.watch;
        if w.in_room() && !w.state.is_host() {
            return self.toast("Only the host can choose what everyone watches.", cx);
        }
        if !w.busy() {
            self.watch_create(cx);
        }
        let same = self.playing() && self.player.key == key && self.player.server.as_ref().is_some_and(|s| s.id == server);
        if same {
            let at = (self.player.time * 1000.0) as i64;
            self.watch_share(server, key, None, at, cx);
            self.expand_player(cx);
        } else {
            self.play(server, key, cx);
        }
    }

    pub fn watch_share(&mut self, server: String, key: String, item: Option<Item>, position_ms: i64, cx: &mut Context<Self>) {
        let w = &mut self.watch;
        if !(w.creating || (w.wt.is_some() && w.state.is_host())) {
            return;
        }
        let hint = PlaivoExt { server: server.clone(), item: key.clone() };
        let shared = w.state.media.as_ref().and_then(Media::plaivo).as_ref() == Some(&hint) || w.share.as_ref().and_then(|(m, _)| m.plaivo()).as_ref() == Some(&hint);
        w.file = Some((server.clone(), key.clone()));
        if shared {
            return;
        }
        let (srv, downloads, c) = (self.server(&server), self.downloads.clone(), self.client.clone());
        Self::bg(
            cx,
            move || {
                let ctx = Ctx::new(c, Source::Network);
                let fetch = |k: &str| downloads.as_ref().and_then(|d| d.item(&server, k)).filter(|i| !i.guids.is_empty()).or_else(|| srv.as_ref()?.metadata(&ctx, k).ok());
                let it = item.or_else(|| fetch(&key))?;
                let show = (it.kind == "episode" && !it.grandparent_rating_key.is_empty()).then(|| fetch(&it.grandparent_rating_key)).flatten();
                describe(&server, &it, show.as_ref())
            },
            move |this, media, cx| {
                let Some(media) = media else {
                    return this.toast("This video can't be shared with the room.", cx);
                };
                let w = &mut this.watch;
                match &w.wt {
                    Some(wt) if w.state.connection == Connection::Connected && w.state.is_host() => wt.set_media(media, position_ms.max(0)),
                    _ if w.creating => w.share = Some((media, position_ms.max(0))),
                    _ => {}
                }
            },
        );
    }

    fn on_watch(&mut self, generation: u64, update: Update, cx: &mut Context<Self>) {
        if generation != self.watch.generation {
            return;
        }
        match update {
            Update::Player(command) => self.watch_command(command),
            Update::State(state) => self.watch_state(*state, cx),
            Update::Event(event) => self.watch_event(event, cx),
        }
    }

    fn watch_state(&mut self, s: PartyState, cx: &mut Context<Self>) {
        let w = &mut self.watch;
        if s.connection == Connection::Connected && w.state.connection != Connection::Connected {
            w.creating = false;
            if let Some(room) = &s.room {
                self.settings.remember_room(&room.code);
                self.settings.save();
            }
        }
        let epoch = s.room.as_ref().map(|r| r.epoch);
        let mut resolve = None;
        if epoch != w.epoch {
            w.epoch = epoch;
            if let (Some(media), Some(epoch), Some(m)) = (&s.media, epoch, s.match_)
                && m.state == MatchState::Searching
            {
                w.file = None;
                w.unavailable = false;
                resolve = Some((media.clone(), epoch));
            }
        }
        if s.connection == Connection::Connected
            && s.is_host()
            && let (Some(wt), Some((media, at))) = (&w.wt, w.share.take())
        {
            wt.set_media(media, at);
        }
        if matches!(s.connection, Connection::Idle | Connection::Ended) {
            w.creating = false;
            w.share = None;
            w.file = None;
            w.epoch = None;
            w.unavailable = false;
        }
        w.state = s;
        if let Some((media, epoch)) = resolve {
            self.watch_resolve(media, epoch, cx);
        }
        self.watch_feed(true);
        cx.notify();
    }

    fn watch_resolve(&mut self, media: Media, epoch: i64, cx: &mut Context<Self>) {
        let (servers, c) = (self.servers.clone(), self.client.clone());
        let theirs = media.clone();
        Self::bg(cx, move || find_copy(&theirs, &servers, &c), move |this, found, cx| {
            let w = &mut this.watch;
            if w.epoch != Some(epoch) || !w.in_room() {
                return;
            }
            let Some(wt) = &w.wt else { return };
            let Some((server, it)) = found else {
                wt.set_match(epoch, Match { state: MatchState::Unavailable, delta_ms: None });
                w.unavailable = true;
                return this.toast(format!("{} isn't in your libraries.", media.series.as_ref().map_or(&media.title, |s| &s.title)), cx);
            };
            wt.set_match(epoch, match_for(it.duration, media.duration_ms));
            w.file = Some((server.clone(), it.rating_key.clone()));
            if this.watch_holds() {
                this.watch_feed(true);
            } else {
                this.play(server, it.rating_key, cx);
            }
        });
    }

    fn watch_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let name = |s: &PartyState, peer: &Option<String>| s.room.as_ref().zip(peer.as_deref()).and_then(|(r, p)| r.participant(p)).map(|p| p.name.clone());
        let text = match event {
            Event::ParticipantJoined(p) => Some(format!("{} joined", p.name)),
            Event::ParticipantLeft(p) => Some(format!("{} left", p.name)),
            Event::Action { kind, by, by_me: false } => action_text(&kind).map(|a| format!("{} {a}", name(&self.watch.state, &by).unwrap_or_else(|| "Someone".into()))),
            Event::Action { .. } => None,
            Event::Error(e) => error_text(&e).map(str::to_owned),
            Event::Failed(e) => {
                self.watch.error = error_text(&e).map(SharedString::from);
                self.watch.error.as_ref().filter(|_| self.page != Page::Watch).map(|e| e.to_string())
            }
            Event::Ended { reason, .. } => {
                self.watch.creating = false;
                end_text(reason).map(str::to_owned)
            }
        };
        if let Some(text) = text {
            if self.page == Page::Player {
                self.hint(Icon::Group, text, cx);
            } else {
                self.toast(text, cx);
            }
        }
    }

    pub fn watch_holds(&self) -> bool {
        let w = &self.watch;
        w.in_room()
            && w.state.match_.is_some_and(|m| matches!(m.state, MatchState::Matched | MatchState::Mismatched))
            && self.playing()
            && w.file.as_ref().is_some_and(|(server, key)| *key == self.player.key && self.player.server.as_ref().is_some_and(|s| s.id == *server))
    }

    pub fn watch_following(&self) -> bool {
        self.watch_holds() && !self.watch.state.is_host()
    }

    pub fn watch_controls(&self) -> bool {
        self.watch_holds() && self.watch.state.can_control()
    }

    pub fn watch_feed(&mut self, now: bool) {
        if self.watch.wt.is_none() {
            return;
        }
        if !self.watch_holds() {
            if std::mem::take(&mut self.watch.attached) {
                self.watch_call(|wt| wt.detach_player());
                if let Some(m) = &self.player.mpv {
                    m.set("speed", &self.player.speed.to_string());
                }
            }
            return;
        }
        let w = &mut self.watch;
        if !now && w.last_feed.is_some_and(|t| t.elapsed() < FEED_EVERY) {
            return;
        }
        let (Some(wt), Some(epoch)) = (&w.wt, w.epoch) else { return };
        let p = &self.player;
        wt.update_player(
            epoch,
            PlayerFeed {
                position_ms: (p.time * 1000.0) as i64,
                duration_ms: (p.duration > 0.0).then_some((p.duration * 1000.0) as i64),
                paused: p.paused,
                buffering: p.buffering && p.video_ready,
                loading: !p.video_ready,
                can_set_rate: true,
                seek_accuracy_ms: 0,
            },
        );
        w.last_feed = Some(Instant::now());
        w.attached = true;
    }

    fn watch_command(&mut self, command: PlayerCommand) {
        if !self.watch_holds() {
            return;
        }
        match command {
            PlayerCommand::Pause => {
                self.player.paused = true;
                self.mpv(|m| m.set("pause", "yes"));
            }
            PlayerCommand::Play => {
                self.player.paused = false;
                self.mpv(|m| m.set("pause", "no"));
            }
            PlayerCommand::Seek(ms) => {
                self.player.time = ms as f64 / 1000.0;
                self.mpv(|m| m.command(&["seek", &(ms as f64 / 1000.0).to_string(), "absolute"]));
            }
            PlayerCommand::SetRate(rate) => self.mpv(|m| m.set("speed", &rate.to_string())),
        }
        self.watch_feed(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::{Guid, Media as PlexMedia, Part};

    fn item(kind: &str, guids: &[&str]) -> Item {
        Item {
            rating_key: "42".into(),
            kind: kind.into(),
            title: "Arrival".into(),
            guid: "plex://movie/5d776".into(),
            year: Some(2016),
            duration: Some(6_960_000),
            guids: guids.iter().map(|g| Guid { id: (*g).into() }).collect(),
            media: vec![PlexMedia { part: vec![Part { file: "/data/movies/Arrival (2016).mkv".into(), ..Default::default() }], ..Default::default() }],
            ..Default::default()
        }
    }

    #[test]
    fn movies_describe_with_ids_file_name_and_plaivo_hint() {
        let m = describe("srv", &item("movie", &["imdb://tt2543164", "tmdb://329865"]), None).unwrap();
        assert_eq!((m.kind, m.title.as_str(), m.year, m.duration_ms), (MediaKind::Movie, "Arrival", Some(2016), 6_960_000));
        assert_eq!(m.ids["imdb"], "tt2543164");
        assert_eq!(m.ids["plex"], "plex://movie/5d776");
        assert_eq!(m.file.as_ref().and_then(|f| f.name.as_deref()), Some("Arrival (2016).mkv"));
        assert_eq!(m.plaivo(), Some(PlaivoExt { server: "srv".into(), item: "42".into() }));
        assert!(describe("srv", &Item { duration: None, ..item("movie", &[]) }, None).is_none());
    }

    #[test]
    fn episodes_carry_their_show_and_numbers() {
        let mut ep = item("episode", &["tvdb://349232"]);
        ep.title = String::new();
        ep.grandparent_title = "Severance".into();
        ep.parent_index = Some(1);
        ep.index = Some(3);
        let show = Item { title: "Severance".into(), year: Some(2022), guids: vec![Guid { id: "tvdb://371980".into() }], ..Default::default() };
        let m = describe("srv", &ep, Some(&show)).unwrap();
        assert_eq!((m.title.as_str(), m.year, m.season, m.episode), ("Episode 3", None, Some(1), Some(3)));
        let series = m.series.unwrap();
        assert_eq!((series.title.as_str(), series.year, series.ids["tvdb"].as_str()), ("Severance", Some(2022), "371980"));
        assert!(describe("srv", &ep, None).unwrap().series.unwrap().ids.is_empty());
    }

    #[test]
    fn copies_match_on_any_shared_catalogue_id() {
        let ours = item("movie", &["imdb://tt2543164", "tmdb://329865"]);
        let wanted = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>();
        assert!(same_title(&wanted(&[("tmdb", "329865")]), &ours));
        assert!(same_title(&wanted(&[("plex", "plex://movie/5d776")]), &ours));
        assert!(!same_title(&wanted(&[("imdb", "tt0000001")]), &ours));
        assert!(!same_title(&wanted(&[]), &ours));
    }

    #[test]
    fn lengths_within_two_seconds_count_as_the_same_file() {
        assert_eq!(match_for(Some(6_961_000), 6_960_000), Match { state: MatchState::Matched, delta_ms: Some(1000) });
        assert_eq!(match_for(Some(6_900_000), 6_960_000).state, MatchState::Mismatched);
        assert_eq!(match_for(None, 6_960_000), Match { state: MatchState::Matched, delta_ms: None });
    }

    #[test]
    fn names_fall_back_and_stay_protocol_safe() {
        assert_eq!(pick_name("  Alex  ", || panic!("not asked"), None), "Alex");
        assert_eq!(pick_name("", || Some("plexuser".into()), Some("jf")), "plexuser");
        assert_eq!(pick_name("\u{200E}", || None, Some("jf")), "jf");
        assert_eq!(pick_name("", || None, None), "Plaivo");
        assert_eq!(pick_name(&"x".repeat(40), || None, None).len(), NAME_MAX);
        assert_eq!(clean("a\tb\n\u{202E}c", 99), "a b c");
    }

    #[test]
    fn relay_setting_falls_back_to_public() {
        assert!(!relays("").custom);
        assert!(relays("https://relay.example.com").custom);
        assert!(!relays("ftp://nope").custom);
    }
}
