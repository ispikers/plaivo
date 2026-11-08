use crate::{Plaivo, R};
use plaivo_core::plex::{Ctx, Item, Server, Source, resolve_playable};
use plaivo_core::settings::Settings;
use plaivo_core::watch_together::{
    Connection, Control, EndReason, Event, Match, MatchState, Media, MediaFile, MediaKind, Participant, PartyState, PlaivoExt, PlayerCommand, PlayerFeed, PlayerState,
    Relays, Series, Update, WatchTogether, format_code, ids_from_guids, normalize_code,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

const CLIENT: &str = concat!("Plaivo/", env!("CARGO_PKG_VERSION"));
const NAME_MAX: usize = 32;
const SAME_FILE_MS: i64 = 2000;
const CANDIDATES: usize = 5;

#[uniffi::export(callback_interface)]
pub trait WatchListener: Send + Sync {
    fn state(&self, state: WatchState);
    fn notice(&self, text: String);
    fn command(&self, command: WatchCommand);
}

#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq)]
pub enum WatchConnection {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Ended,
}

#[derive(uniffi::Record, Debug, PartialEq)]
pub struct WatchPeer {
    pub peer_id: String,
    pub name: String,
    pub host: bool,
    pub me: bool,
    pub status: String,
}

#[derive(uniffi::Record, Debug, PartialEq)]
pub struct WatchState {
    pub connection: WatchConnection,
    pub code: String,
    pub display_code: String,
    pub epoch: i64,
    pub host: bool,
    pub everyone_controls: bool,
    pub locked: bool,
    pub media: Option<String>,
    pub status: String,
    pub peers: Vec<WatchPeer>,
}

#[derive(uniffi::Enum, Debug, PartialEq)]
pub enum WatchCommand {
    Pause,
    Play,
    Seek { position_ms: i64 },
    SetRate { rate: f64 },
}

#[derive(uniffi::Record)]
pub struct WatchFeed {
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    pub paused: bool,
    pub buffering: bool,
    pub loading: bool,
}

#[derive(uniffi::Record, Debug, PartialEq)]
pub struct WatchItem {
    pub server: String,
    pub key: String,
}

pub(crate) struct Session {
    wt: WatchTogether,
    name: String,
    relay: String,
    state: Arc<Mutex<PartyState>>,
}

#[uniffi::export]
impl Plaivo {
    pub fn watch_listen(&self, listener: Box<dyn WatchListener>) {
        let _ = self.watch_listener.set(Arc::from(listener));
    }

    pub fn watch_create(&self, device_name: String) -> R<()> {
        self.start_watch(&device_name, WatchTogether::create)
    }

    pub fn watch_join(&self, code: String, device_name: String) -> R<()> {
        let code = normalize_code(&code).ok_or("Room codes are six letters and numbers")?;
        self.start_watch(&device_name, |w| w.join(&code, false))
    }

    pub fn watch_leave(&self) {
        self.with_watch(WatchTogether::leave);
    }

    pub fn watch_play(&self) {
        self.with_watch(WatchTogether::play);
    }

    pub fn watch_pause(&self) {
        self.with_watch(WatchTogether::pause);
    }

    pub fn watch_seek(&self, position_ms: i64) {
        self.with_watch(|w| w.seek(position_ms.max(0)));
    }

    pub fn watch_set_rate(&self, rate: f64) {
        self.with_watch(|w| w.set_rate(rate));
    }

    pub fn watch_feed(&self, epoch: i64, feed: WatchFeed) {
        let feed = PlayerFeed {
            position_ms: feed.position_ms.max(0),
            duration_ms: feed.duration_ms.filter(|d| *d > 0),
            paused: feed.paused,
            buffering: feed.buffering,
            loading: feed.loading,
            can_set_rate: true,
            seek_accuracy_ms: 0,
        };
        self.with_watch(|w| w.update_player(epoch, feed));
    }

    pub fn watch_detach(&self) {
        self.with_watch(WatchTogether::detach_player);
    }

    pub fn watch_matched(&self, epoch: i64, duration_ms: i64) {
        let Some(theirs) = self.watch_media(epoch).map(|m| m.duration_ms) else { return };
        self.with_watch(|w| w.set_match(epoch, local_match(theirs, duration_ms)));
    }

    pub fn watch_unavailable(&self, epoch: i64) {
        self.with_watch(|w| w.set_match(epoch, Match { state: MatchState::Unavailable, delta_ms: None }));
    }

    pub fn watch_resolve(&self, epoch: i64) -> Option<WatchItem> {
        let media = self.watch_media(epoch)?;
        let servers = self.account.lock().unwrap().all_servers();
        if let Some(hint) = media.plaivo().filter(|h| servers.iter().any(|s| s.id == h.server)) {
            return Some(WatchItem { server: hint.server, key: hint.item });
        }
        let target = Target::of(&media)?;
        let cx = Ctx::new(self.client.clone(), Source::Live);
        std::thread::scope(|s| {
            let jobs: Vec<_> = servers.iter().map(|srv| s.spawn(|| find_on(srv, &cx, &target, &media))).collect();
            jobs.into_iter().find_map(|job| job.join().ok().flatten())
        })
    }

    pub fn watch_share(&self, server_id: String, key: String) -> R<()> {
        let srv = self.server(&server_id)?;
        let cx = Ctx::new(self.client.clone(), Source::Live);
        let it = resolve_playable(&srv, &cx, &key)?;
        let show = (it.kind == "episode" && !it.grandparent_rating_key.is_empty()).then(|| srv.metadata(&cx, &it.grandparent_rating_key).ok()).flatten();
        let media = shared_media(&srv.id, &it, show.as_ref()).map_err(|e| format!("This can't be shared ({e})"))?;
        if !self.with_watch(|w| w.set_media(media, 0)) {
            return Err("Join a room first".into());
        }
        Ok(())
    }

    pub fn watch_clear_media(&self) {
        self.with_watch(WatchTogether::clear_media);
    }

    pub fn watch_set_control(&self, everyone: bool) {
        self.with_watch(|w| w.set_control(if everyone { Control::Everyone } else { Control::Host }));
    }

    pub fn watch_transfer_host(&self, peer_id: String) {
        self.with_watch(|w| w.transfer_host(&peer_id));
    }

    pub fn watch_kick(&self, peer_id: String) {
        self.with_watch(|w| w.kick(&peer_id));
    }

    pub fn watch_lock(&self, locked: bool) {
        self.with_watch(|w| w.lock(locked));
    }

    pub fn watch_end(&self) {
        self.with_watch(WatchTogether::end);
    }
}

#[uniffi::export]
pub fn remember_room(mut settings: Settings, code: String) -> Settings {
    settings.remember_room(&code);
    settings
}

#[uniffi::export]
pub fn format_room_code(code: String) -> String {
    format_code(&code).unwrap_or(code)
}

#[uniffi::export]
pub fn valid_relay(url: String) -> bool {
    Relays::custom(&url).is_some()
}

impl Plaivo {
    fn start_watch(&self, device_name: &str, f: impl FnOnce(&WatchTogether)) -> R<()> {
        let s = self.settings.lock().unwrap();
        let name = display_name(&s.watch_name, device_name).ok_or("Set a display name under Settings, Watch Together")?;
        let relay = s.watch_relay.trim().to_owned();
        drop(s);
        let relays = if relay.is_empty() { Relays::public() } else { Relays::custom(&relay).ok_or("The custom relay address isn't valid")? };
        let listener = self.watch_listener.get().cloned().ok_or("Watch Together isn't ready")?;
        let mut slot = self.watch.lock().unwrap();
        if slot.as_ref().is_none_or(|s| s.name != name || s.relay != relay) {
            let old = slot.take();
            drop(slot);
            drop(old);
            let state = Arc::new(Mutex::new(PartyState::default()));
            let shared = state.clone();
            let wt = WatchTogether::new(&name, CLIENT, relays, move |u| deliver(&*listener, &shared, u)).map_err(|e| format!("Watch Together: {}", e.code))?;
            slot = self.watch.lock().unwrap();
            *slot = Some(Session { wt, name, relay, state });
        }
        if let Some(s) = slot.as_ref() {
            f(&s.wt);
        }
        Ok(())
    }

    fn with_watch(&self, f: impl FnOnce(&WatchTogether)) -> bool {
        let slot = self.watch.lock().unwrap();
        slot.as_ref().map(|s| f(&s.wt)).is_some()
    }

    fn watch_media(&self, epoch: i64) -> Option<Media> {
        let state = self.watch.lock().unwrap().as_ref()?.state.clone();
        let s = state.lock().unwrap();
        s.room.as_ref().filter(|r| r.epoch == epoch)?;
        s.media.clone()
    }
}

fn deliver(listener: &dyn WatchListener, shared: &Mutex<PartyState>, update: Update) {
    match update {
        Update::State(s) => {
            let view = view(&s);
            *shared.lock().unwrap() = *s;
            listener.state(view);
        }
        Update::Event(e) => {
            let text = notice(&e, &shared.lock().unwrap());
            if let Some(text) = text {
                listener.notice(text);
            }
        }
        Update::Player(c) => listener.command(match c {
            PlayerCommand::Pause => WatchCommand::Pause,
            PlayerCommand::Play => WatchCommand::Play,
            PlayerCommand::Seek(position_ms) => WatchCommand::Seek { position_ms },
            PlayerCommand::SetRate(rate) => WatchCommand::SetRate { rate },
        }),
    }
}

fn display_name(saved: &str, device: &str) -> Option<String> {
    let name = [saved, device, "Plaivo"].into_iter().map(str::trim).find(|n| !n.is_empty())?;
    Some(name.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect())
}

fn local_match(theirs_ms: i64, ours_ms: i64) -> Match {
    if ours_ms <= 0 {
        return Match { state: MatchState::Matched, delta_ms: None };
    }
    let delta = ours_ms - theirs_ms;
    Match { state: if delta.abs() <= SAME_FILE_MS { MatchState::Matched } else { MatchState::Mismatched }, delta_ms: Some(delta) }
}

fn view(s: &PartyState) -> WatchState {
    let room = s.room.as_ref();
    let me = s.peer_id.as_deref();
    let name = |id: &str| room.and_then(|r| r.participant(id)).map_or("someone", |p| p.name.as_str());
    let waiting: Vec<&str> = room.map(|r| r.waiting_on.iter().filter(|id| Some(id.as_str()) != me).map(|id| name(id)).collect()).unwrap_or_default();
    let status = match s.match_.map(|m| m.state) {
        Some(MatchState::Searching) => "Finding the video".into(),
        Some(MatchState::Unavailable) => "Not on your servers".into(),
        _ if !waiting.is_empty() => format!("Waiting for {}", waiting.join(", ")),
        _ => String::new(),
    };
    WatchState {
        connection: match s.connection {
            Connection::Idle => WatchConnection::Idle,
            Connection::Connecting => WatchConnection::Connecting,
            Connection::Connected => WatchConnection::Connected,
            Connection::Reconnecting => WatchConnection::Reconnecting,
            Connection::Ended => WatchConnection::Ended,
        },
        code: room.map(|r| r.code.clone()).unwrap_or_default(),
        display_code: room.and_then(|r| format_code(&r.code)).unwrap_or_default(),
        epoch: room.map_or(0, |r| r.epoch),
        host: s.is_host(),
        everyone_controls: room.is_some_and(|r| r.control == Control::Everyone),
        locked: room.is_some_and(|r| r.locked),
        media: s.media.as_ref().map(media_title),
        status,
        peers: room.map(|r| r.participants.iter().map(|p| peer(p, me)).collect()).unwrap_or_default(),
    }
}

fn peer(p: &Participant, me: Option<&str>) -> WatchPeer {
    let status = match (p.connected, p.match_.map(|m| m.state), p.player) {
        (false, _, _) => "Reconnecting",
        (_, Some(MatchState::Searching), _) => "Finding the video",
        (_, Some(MatchState::Unavailable), _) => "Doesn't have this video",
        (_, _, PlayerState::Idle) => "",
        (_, _, PlayerState::Loading) => "Loading",
        (_, _, PlayerState::Buffering) => "Buffering",
        (_, _, PlayerState::Ready) => "Ready",
    };
    WatchPeer { peer_id: p.peer_id.clone(), name: p.name.clone(), host: p.host, me: Some(p.peer_id.as_str()) == me, status: status.into() }
}

fn media_title(m: &Media) -> String {
    match (&m.series, m.season, m.episode) {
        (Some(s), Some(season), Some(episode)) => format!("{} · S{season} E{episode} · {}", s.title, m.title),
        (Some(s), _, _) => format!("{} · {}", s.title, m.title),
        _ => m.year.map_or_else(|| m.title.clone(), |y| format!("{} ({y})", m.title)),
    }
}

fn notice(e: &Event, s: &PartyState) -> Option<String> {
    let name = |id: Option<&str>| id.and_then(|id| s.room.as_ref()?.participant(id)).map_or("Someone", |p| p.name.as_str()).to_owned();
    Some(match e {
        Event::Action { by_me: true, .. } => return None,
        Event::Action { kind, by, .. } => {
            let what = match kind.as_str() {
                "play" => "pressed play",
                "pause" => "paused",
                "seek" => "skipped",
                "rate" => "changed the speed",
                "media" => "picked something to watch",
                _ => return None,
            };
            format!("{} {what}", name(by.as_deref()))
        }
        Event::ParticipantJoined(p) => format!("{} joined", p.name),
        Event::ParticipantLeft(p) => format!("{} left", p.name),
        Event::Error(e) | Event::Failed(e) => match e.code.as_str() {
            "cancelled" => return None,
            "forbidden" => "Only the host can do that".into(),
            "room_not_found" => "There's no room with that code".into(),
            "unreachable" => "Couldn't reach the Watch Together relay".into(),
            "invalid_code" => "Room codes are six letters and numbers".into(),
            "room_locked" | "locked" => "That room is locked".into(),
            code if e.message.is_empty() => format!("Watch Together: {code}"),
            _ => format!("Watch Together: {}", e.message),
        },
        Event::Ended { reason, .. } => match reason {
            EndReason::Left => return None,
            EndReason::Ended => "The host ended the room".into(),
            EndReason::Kicked => "You were removed from the room".into(),
            EndReason::Expired => "The room expired".into(),
            EndReason::Replaced => "You joined this room from somewhere else".into(),
            EndReason::Lost => "Lost the connection to the room".into(),
        },
    })
}

fn shared_media(server: &str, it: &Item, show: Option<&Item>) -> Result<Media, &'static str> {
    let ids = |it: &Item| ids_from_guids(it.guids.iter().map(|g| g.id.as_str()));
    let episode = it.kind == "episode";
    let index = |i: Option<i64>| i.filter(|i| *i >= 0 && episode);
    let file = it.media.first().and_then(|m| m.part.first()).and_then(|p| p.file.rsplit(['/', '\\']).next()).filter(|f| !f.is_empty());
    let mut m = Media {
        kind: match it.kind.as_str() {
            "movie" => MediaKind::Movie,
            "episode" => MediaKind::Episode,
            _ => MediaKind::Video,
        },
        title: it.title.clone(),
        year: it.year,
        ids: ids(it),
        series: episode.then(|| Series { title: it.grandparent_title.clone(), year: show.and_then(|s| s.year), ids: show.map(ids).unwrap_or_default() }),
        season: index(it.parent_index),
        episode: index(it.index),
        duration_ms: it.duration.unwrap_or(0) as i64,
        file: file.map(|name| MediaFile { name: Some(name.into()), ..Default::default() }),
        ext: Default::default(),
    };
    m.set_plaivo(&PlaivoExt { server: server.into(), item: it.rating_key.clone() });
    if m.check(true).is_err() {
        m.file = None;
    }
    m.check(true).map(|_| m)
}

struct Target<'a> {
    kind: &'static str,
    title: &'a str,
    year: Option<i64>,
    ids: &'a BTreeMap<String, String>,
}

impl<'a> Target<'a> {
    fn of(m: &'a Media) -> Option<Self> {
        match (m.kind, &m.series) {
            (MediaKind::Movie, _) => Some(Self { kind: "movie", title: &m.title, year: m.year, ids: &m.ids }),
            (MediaKind::Episode, Some(s)) => Some(Self { kind: "show", title: &s.title, year: s.year, ids: &s.ids }),
            _ => None,
        }
    }

    fn matches(&self, it: &Item) -> bool {
        it.kind == self.kind && (shares_id(self.ids, it).unwrap_or_else(|| self.year.is_some() && self.year == it.year && same_title(self.title, &it.title)))
    }
}

fn shares_id(ids: &BTreeMap<String, String>, it: &Item) -> Option<bool> {
    let theirs = ids_from_guids(it.guids.iter().map(|g| g.id.as_str()));
    let common: Vec<bool> = ids.iter().filter_map(|(ns, v)| theirs.get(ns).map(|w| v.eq_ignore_ascii_case(w))).collect();
    (!common.is_empty()).then(|| common.contains(&true))
}

fn same_title(a: &str, b: &str) -> bool {
    let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect::<String>();
    key(a) == key(b)
}

fn pick_episode<'a>(episodes: &'a [Item], m: &Media) -> Option<&'a Item> {
    episodes.iter().find(|e| shares_id(&m.ids, e) == Some(true)).or_else(|| {
        let (season, episode) = (m.season?, m.episode?);
        episodes.iter().find(|e| e.parent_index == Some(season) && e.index == Some(episode))
    })
}

fn find_on(srv: &Server, cx: &Ctx, target: &Target, media: &Media) -> Option<WatchItem> {
    let hits = srv.search(cx, target.title).inspect_err(|e| log::warn!("watch together: search on {}: {e}", srv.name)).ok()?;
    let found = hits.into_iter().filter(|(it, _)| it.kind == target.kind).take(CANDIDATES).find_map(|(it, _)| srv.metadata(cx, &it.rating_key).ok().filter(|full| target.matches(full)))?;
    let key = match media.kind {
        MediaKind::Episode => pick_episode(&srv.all_episodes(cx, &found.rating_key).ok()?, media)?.rating_key.clone(),
        _ => found.rating_key,
    };
    Some(WatchItem { server: srv.id.clone(), key })
}

#[cfg(test)]
mod tests {
    use super::*;
    use plaivo_core::plex::{Guid, Media as PlexMedia, Part};
    use plaivo_core::watch_together::{Phase, Room};

    fn guids(ids: &[&str]) -> Vec<Guid> {
        ids.iter().map(|id| Guid { id: (*id).into() }).collect()
    }

    fn movie_item() -> Item {
        Item {
            rating_key: "42".into(),
            kind: "movie".into(),
            title: "Arrival".into(),
            year: Some(2016),
            duration: Some(6_960_000),
            guids: guids(&["imdb://tt2543164", "tmdb://329865"]),
            media: vec![PlexMedia { part: vec![Part { file: "/movies/Arrival (2016)/Arrival.mkv".into(), ..Default::default() }], ..Default::default() }],
            ..Default::default()
        }
    }

    fn episode_item(season: i64, episode: i64, key: &str) -> Item {
        Item {
            rating_key: key.into(),
            kind: "episode".into(),
            title: format!("Episode {episode}"),
            grandparent_title: "Severance".into(),
            parent_index: Some(season),
            index: Some(episode),
            duration: Some(3_000_000),
            ..Default::default()
        }
    }

    fn participant(id: &str, name: &str, host: bool) -> Participant {
        Participant { peer_id: id.into(), name: name.into(), client: "Plezy/1".into(), host, connected: true, player: PlayerState::Ready, match_: None, offset_ms: 0 }
    }

    #[test]
    fn shared_media_carries_ids_hint_and_file_name() {
        let m = shared_media("srv", &movie_item(), None).unwrap();
        assert_eq!((m.kind, m.title.as_str(), m.year, m.duration_ms), (MediaKind::Movie, "Arrival", Some(2016), 6_960_000));
        assert_eq!(m.ids["imdb"], "tt2543164");
        assert_eq!(m.plaivo(), Some(PlaivoExt { server: "srv".into(), item: "42".into() }));
        assert_eq!(m.file.unwrap().name.as_deref(), Some("Arrival.mkv"));
        let show = Item { kind: "show".into(), year: Some(2022), guids: guids(&["tvdb://371980"]), ..Default::default() };
        let e = shared_media("srv", &episode_item(1, 3, "7"), Some(&show)).unwrap();
        let series = e.series.unwrap();
        assert_eq!((e.season, e.episode, series.title.as_str(), series.year), (Some(1), Some(3), "Severance", Some(2022)));
        assert_eq!(series.ids["tvdb"], "371980");
        assert_eq!(shared_media("srv", &Item { duration: None, ..movie_item() }, None).unwrap_err(), "durationMs");
    }

    #[test]
    fn resolves_by_catalogue_ids_then_by_title_and_year() {
        let ours = shared_media("elsewhere", &movie_item(), None).unwrap();
        let t = Target::of(&ours).unwrap();
        assert!(t.matches(&movie_item()));
        assert!(!t.matches(&Item { guids: guids(&["imdb://tt0000001"]), ..movie_item() }));
        assert!(t.matches(&Item { guids: guids(&["imdb://TT2543164"]), ..movie_item() }));
        assert!(!t.matches(&Item { kind: "show".into(), ..movie_item() }));
        let bare = Media { ids: BTreeMap::new(), ..ours.clone() };
        let t = Target::of(&bare).unwrap();
        assert!(t.matches(&Item { title: "arrival".into(), guids: vec![], ..movie_item() }));
        assert!(!t.matches(&Item { year: Some(2017), guids: vec![], ..movie_item() }));
        assert!(Target::of(&Media { kind: MediaKind::Video, ..ours }).is_none());
    }

    #[test]
    fn picks_the_episode_by_ids_or_numbers() {
        let mut eps = vec![episode_item(1, 1, "a"), episode_item(1, 2, "b"), episode_item(2, 1, "c")];
        let m = shared_media("srv", &episode_item(2, 1, "x"), None).unwrap();
        assert_eq!(pick_episode(&eps, &m).unwrap().rating_key, "c");
        eps[0].guids = guids(&["tvdb://99"]);
        let by_id = Media { ids: ids_from_guids(["tvdb://99"]), ..m.clone() };
        assert_eq!(pick_episode(&eps, &by_id).unwrap().rating_key, "a");
        assert!(pick_episode(&eps, &Media { season: Some(5), ..m }).is_none());
    }

    #[test]
    fn local_match_compares_durations() {
        assert_eq!(local_match(100_000, 101_500), Match { state: MatchState::Matched, delta_ms: Some(1500) });
        assert_eq!(local_match(100_000, 95_000).state, MatchState::Mismatched);
        assert_eq!(local_match(100_000, 0), Match { state: MatchState::Matched, delta_ms: None });
    }

    #[test]
    fn view_and_notices_read_the_room() {
        let room = Room {
            rev: 3,
            code: "ABC234".into(),
            epoch: 2,
            phase: Phase::Waiting,
            position_ms: 0,
            at_ms: 0,
            rate: 1.0,
            control: Control::Host,
            locked: true,
            waiting_on: vec!["p2".into()],
            participants: vec![participant("p1", "Ann", true), participant("p2", "Bob", false)],
        };
        let s = PartyState { connection: Connection::Connected, peer_id: Some("p1".into()), room: Some(room), ..Default::default() };
        let v = view(&s);
        assert_eq!((v.display_code.as_str(), v.host, v.everyone_controls, v.locked, v.status.as_str()), ("ABC-234", true, false, true, "Waiting for Bob"));
        assert_eq!(v.peers.iter().map(|p| (p.name.as_str(), p.me, p.host)).collect::<Vec<_>>(), [("Ann", true, true), ("Bob", false, false)]);
        assert_eq!(notice(&Event::Action { kind: "pause".into(), by: Some("p2".into()), by_me: false }, &s).as_deref(), Some("Bob paused"));
        assert_eq!(notice(&Event::Action { kind: "pause".into(), by: Some("p1".into()), by_me: true }, &s), None);
        assert_eq!(notice(&Event::Ended { reason: EndReason::Kicked, error: None }, &s).as_deref(), Some("You were removed from the room"));
        assert_eq!(display_name("  ", "Living Room TV").as_deref(), Some("Living Room TV"));
        assert_eq!(display_name("", "").as_deref(), Some("Plaivo"));
        assert_eq!(display_name(&"x".repeat(40), "").unwrap().len(), NAME_MAX);
    }
}
