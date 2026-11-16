use crate::{Plaivo, R};
use plaivo_core::plex::{Ctx, Source, resolve_playable};
use plaivo_core::settings::Settings;
use plaivo_core::watch_together::{
    Connection, Control, Event, Match, MatchState, Media, Participant, PartyState, PlayerCommand,
    PlayerFeed, Relays, Update, WatchTogether, describe, display_name, find_copy, format_code,
    match_for, mismatch_text, normalize_code, sync_label,
};
use std::sync::{Arc, Mutex};

#[uniffi::export(callback_interface)]
pub trait WatchListener: Send + Sync {
    fn state(&self, state: WatchState);
    fn notice(&self, text: String);
    fn room_gone(&self);
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
    pub offset_ms: i64,
    pub adjustable: bool,
    pub mismatch_ms: Option<i64>,
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
    pub mismatch_ms: Option<i64>,
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

    pub fn watch_create(&self) -> R<()> {
        self.start_watch(WatchTogether::create)
    }

    pub fn watch_join(&self, code: String, create: bool) -> R<()> {
        let code = normalize_code(&code).ok_or("Room codes are six letters and numbers")?;
        self.start_watch(|w| w.join(&code, create))
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
        let Some(theirs) = self.watch_media(epoch).map(|m| m.duration_ms) else {
            return;
        };
        self.with_watch(|w| w.set_match(epoch, match_for(Some(duration_ms), theirs)));
    }

    pub fn watch_answer_mismatch(&self, epoch: i64, delta_ms: i64, accept: bool) {
        let state = if accept {
            MatchState::Mismatched
        } else {
            MatchState::Unavailable
        };
        self.with_watch(|w| {
            w.set_match(
                epoch,
                Match {
                    state,
                    delta_ms: Some(delta_ms),
                },
            )
        });
    }

    pub fn watch_set_offset(&self, offset_ms: i64) {
        self.with_watch(|w| w.set_offset(offset_ms));
    }

    pub fn watch_unavailable(&self, epoch: i64) -> Option<String> {
        let text = self.watch_media(epoch).map(|m| m.not_found_text());
        self.with_watch(|w| w.set_match(epoch, Match::UNAVAILABLE));
        text
    }

    pub fn watch_resolve(&self, epoch: i64) -> Option<WatchItem> {
        let media = self.watch_media(epoch)?;
        let servers = self.active_servers();
        find_copy(&media, &servers, &self.client).map(|(server, it)| {
            let m = match_for(it.duration.map(|d| d as i64), media.duration_ms);
            WatchItem {
                server,
                key: it.rating_key,
                mismatch_ms: m.delta_ms.filter(|_| m.state == MatchState::Mismatched),
            }
        })
    }

    pub fn watch_share(&self, server_id: String, key: String) -> R<()> {
        let srv = self.server(&server_id)?;
        let cx = Ctx::new(self.client.clone(), Source::Live);
        let it = resolve_playable(&srv, &cx, &key)?;
        let show = (it.kind == "episode" && !it.grandparent_rating_key.is_empty())
            .then(|| srv.metadata(&cx, &it.grandparent_rating_key).ok())
            .flatten();
        let media = describe(&srv.id, &it, show.as_ref())
            .map_err(|e| format!("This can't be shared ({e})"))?;
        if !self.with_watch(|w| w.set_media(media, 0)) {
            return Err("Join a room first".into());
        }
        Ok(())
    }

    pub fn watch_set_control(&self, everyone: bool) {
        self.with_watch(|w| {
            w.set_control(if everyone {
                Control::Everyone
            } else {
                Control::Host
            })
        });
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
pub fn forget_room(mut settings: Settings, code: String) -> Settings {
    settings.forget_room(&normalize_code(&code).unwrap_or(code));
    settings
}

#[uniffi::export]
pub fn format_room_code(code: String) -> String {
    format_code(&code).unwrap_or(code)
}

#[uniffi::export]
pub fn watch_mismatch_text(delta_ms: i64) -> String {
    mismatch_text(delta_ms)
}

#[uniffi::export]
pub fn valid_relay(url: String) -> bool {
    Relays::custom(&url).is_some()
}

impl Plaivo {
    fn start_watch(&self, f: impl FnOnce(&WatchTogether)) -> R<()> {
        let plex = self.plex_user().map(|u| u.name);
        let jellyfin = self
            .account
            .lock()
            .unwrap()
            .jellyfin
            .first()
            .map(|s| s.user.clone());
        let name = display_name(plex.iter().chain(&jellyfin).map(String::as_str));
        let relay = self.settings.lock().unwrap().watch_relay.trim().to_owned();
        let relays = Relays::from_setting(&relay).ok_or("The custom relay address isn't valid")?;
        let listener = self
            .watch_listener
            .get()
            .cloned()
            .ok_or("Watch Together isn't ready")?;
        let mut slot = self.watch.lock().unwrap();
        let mut replaced = None;
        if slot
            .as_ref()
            .is_none_or(|s| s.name != name || s.relay != relay)
        {
            let old = slot.take();
            drop(slot);
            drop(old);
            let state = Arc::new(Mutex::new(PartyState::default()));
            let shared = state.clone();
            let wt = WatchTogether::new(&name, &client(), relays, move |u| {
                deliver(&*listener, &shared, u)
            })
            .map_err(|e| format!("Watch Together: {}", e.code))?;
            slot = self.watch.lock().unwrap();
            replaced = slot.replace(Session {
                wt,
                name,
                relay,
                state,
            });
        }
        if let Some(s) = slot.as_ref() {
            f(&s.wt);
        }
        drop(slot);
        drop(replaced);
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
            if matches!(&e, Event::Failed(err) if err.room_gone()) {
                listener.room_gone();
            }
            let text = e.text(&shared.lock().unwrap());
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

fn client() -> String {
    let version = option_env!("PLAIVO_APP_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"));
    format!("Plaivo/{version}")
}

fn view(s: &PartyState) -> WatchState {
    let room = s.room.as_ref();
    let me = s.peer_id.as_deref();
    let status = match s.match_.map(|m| m.state) {
        Some(MatchState::Searching | MatchState::Unavailable) => s.sync_text().into(),
        _ => s.waiting_text().unwrap_or_default(),
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
        media: s.media.as_ref().map(Media::display_title),
        status,
        peers: room
            .map(|r| r.participants.iter().map(|p| peer(p, me)).collect())
            .unwrap_or_default(),
        offset_ms: s.offset_ms,
        adjustable: s.offset_adjustable(),
        mismatch_ms: s
            .match_
            .filter(|m| m.state == MatchState::Mismatched)
            .and_then(|m| m.delta_ms),
    }
}

fn peer(p: &Participant, me: Option<&str>) -> WatchPeer {
    WatchPeer {
        peer_id: p.peer_id.clone(),
        name: p.name.clone(),
        host: p.host,
        me: Some(p.peer_id.as_str()) == me,
        status: sync_label(p.player, p.match_, p.connected).into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use plaivo_core::watch_together::{EndReason, Event, Phase, PlayerState, Room};

    fn participant(id: &str, name: &str, host: bool) -> Participant {
        Participant {
            peer_id: id.into(),
            name: name.into(),
            client: "Plezy/1".into(),
            host,
            connected: true,
            player: PlayerState::Ready,
            match_: None,
            offset_ms: 0,
        }
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
            participants: vec![
                participant("p1", "Ann", true),
                participant("p2", "Bob", false),
            ],
        };
        let s = PartyState {
            connection: Connection::Connected,
            peer_id: Some("p1".into()),
            room: Some(room),
            ..Default::default()
        };
        let v = view(&s);
        assert_eq!(
            (
                v.display_code.as_str(),
                v.host,
                v.everyone_controls,
                v.locked,
                v.status.as_str()
            ),
            ("ABC-234", true, false, true, "Waiting for Bob")
        );
        assert_eq!(
            v.peers
                .iter()
                .map(|p| (p.name.as_str(), p.me, p.host))
                .collect::<Vec<_>>(),
            [("Ann", true, true), ("Bob", false, false)]
        );
        assert_eq!(
            Event::Action {
                kind: "pause".into(),
                by: Some("p2".into()),
                by_me: false
            }
            .text(&s)
            .as_deref(),
            Some("Bob paused")
        );
        assert_eq!(
            Event::Action {
                kind: "pause".into(),
                by: Some("p1".into()),
                by_me: true
            }
            .text(&s),
            None
        );
        assert_eq!(
            Event::Ended {
                reason: EndReason::Kicked,
                error: None
            }
            .text(&s)
            .as_deref(),
            Some("You were removed from the room.")
        );
    }
}
