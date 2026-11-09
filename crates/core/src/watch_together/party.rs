use super::clock::Clock;
use super::follow::{self, PlayerCommand, PlayerFeed, StepIn};
use super::media::Media;
use super::protocol::*;
use super::relays::normalize_code;
use serde_json::Value;

pub const CONNECT_TIMEOUT_MS: i64 = 10_000;
const SILENCE_MS: i64 = 15_000;
const RESUME_WINDOW_MS: i64 = 300_000;
const HEARTBEAT_MS: i64 = 15_000;
const SPACING_MS: i64 = 250;
const HOLD_MS: i64 = 2000;
const LANE_MS: i64 = 2000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Connection {
    #[default]
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Ended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Left,
    Ended,
    Kicked,
    Expired,
    Replaced,
    Lost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyError {
    pub code: String,
    pub message: String,
    pub re: Option<String>,
}

impl PartyError {
    pub(crate) fn local(code: &str) -> Self {
        Self {
            code: code.into(),
            message: String::new(),
            re: None,
        }
    }
}

impl From<WireError> for PartyError {
    fn from(e: WireError) -> Self {
        Self {
            code: e.code,
            message: e.message,
            re: e.re,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyState {
    pub connection: Connection,
    pub peer_id: Option<String>,
    pub room: Option<Room>,
    pub media: Option<Media>,
    pub match_: Option<Match>,
    pub offset_ms: i64,
    pub player: PlayerState,
    pub end_reason: Option<EndReason>,
}

impl PartyState {
    pub fn me(&self) -> Option<&Participant> {
        self.room.as_ref()?.participant(self.peer_id.as_deref()?)
    }

    pub fn is_host(&self) -> bool {
        self.me().is_some_and(|p| p.host)
    }

    pub fn can_control(&self) -> bool {
        self.is_host()
            || (self.me().is_some()
                && self
                    .room
                    .as_ref()
                    .is_some_and(|r| r.control == Control::Everyone))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Action {
        kind: String,
        by: Option<String>,
        by_me: bool,
    },
    ParticipantJoined(Participant),
    ParticipantLeft(Participant),
    Error(PartyError),
    Ended {
        reason: EndReason,
        error: Option<String>,
    },
    Failed(PartyError),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Output {
    Send(String),
    Connect(String),
    Close(u16),
    Player(PlayerCommand),
    Refused(PartyError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Create,
    Join { code: String, create: bool },
}

struct Link {
    url: String,
    code: String,
    peer_id: String,
    token: String,
}

#[derive(Clone, Copy, PartialEq)]
struct Status {
    epoch: i64,
    player: PlayerState,
    match_: Option<Match>,
    offset_ms: i64,
}

pub struct Party {
    name: String,
    client: String,
    state: PartyState,
    request: Option<Request>,
    link: Option<Link>,
    url: Option<String>,
    open: bool,
    answer_by: Option<i64>,
    cancel_by: Option<i64>,
    tried: Vec<String>,
    clock: Option<Clock>,
    old_clock: Option<Clock>,
    rev: i64,
    epoch: Option<i64>,
    own_media: Option<Media>,
    pending_media: Option<(Media, i64)>,
    feed: Option<(PlayerFeed, i64)>,
    away: bool,
    bound: bool,
    applied_rate: f64,
    nudging: bool,
    last_seek_at: Option<i64>,
    lane_until: Option<i64>,
    seek_in_lane: bool,
    hold: Option<(i64, &'static str)>,
    next_step_at: i64,
    step_now: bool,
    play_at: Option<i64>,
    last_status: Option<(Status, i64)>,
    dropped_at: i64,
    attempt: u32,
    attempt_at: Option<i64>,
    last_recv: i64,
    rng: u64,
    out: Vec<Output>,
    events: Vec<Event>,
}

impl Party {
    pub fn new(name: &str, client: &str, seed: u64) -> Result<Self, PartyError> {
        if !valid_text(name, 32) || !valid_client(client) {
            return Err(PartyError::local("invalid_message"));
        }
        Ok(Self {
            name: name.into(),
            client: client.into(),
            state: PartyState::default(),
            request: None,
            link: None,
            url: None,
            open: false,
            answer_by: None,
            cancel_by: None,
            tried: Vec::new(),
            clock: None,
            old_clock: None,
            rev: 0,
            epoch: None,
            own_media: None,
            pending_media: None,
            feed: None,
            away: false,
            bound: false,
            applied_rate: 1.0,
            nudging: false,
            last_seek_at: None,
            lane_until: None,
            seek_in_lane: false,
            hold: None,
            next_step_at: 0,
            step_now: false,
            play_at: None,
            last_status: None,
            dropped_at: 0,
            attempt: 0,
            attempt_at: None,
            last_recv: 0,
            rng: seed | 1,
            out: Vec::new(),
            events: Vec::new(),
        })
    }

    pub fn state(&self) -> &PartyState {
        &self.state
    }

    pub fn request(&self) -> Option<&Request> {
        self.request.as_ref()
    }

    pub fn tried(&self) -> &[String] {
        &self.tried
    }

    pub fn next_deadline(&self) -> Option<i64> {
        let reconnecting = self.state.connection == Connection::Reconnecting;
        let window = reconnecting.then_some(self.dropped_at + RESUME_WINDOW_MS);
        let blocked = [self.hold.map(|(t, _)| t), self.lane_until]
            .into_iter()
            .flatten()
            .max();
        let step = (reconnecting && self.bound)
            .then_some(blocked.map_or(self.next_step_at, |t| t.max(self.next_step_at)));
        [
            self.answer_by,
            self.attempt_at,
            self.cancel_by,
            window,
            step,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn take_outputs(&mut self) -> Vec<Output> {
        std::mem::take(&mut self.out)
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn relay_now(&self, now: i64) -> Option<i64> {
        self.clock
            .as_ref()
            .and_then(|c| c.relay_now(now))
            .or_else(|| self.old_clock.as_ref()?.relay_now(now))
    }

    pub fn start(&mut self, request: Request, now: i64) -> Result<(), PartyError> {
        if matches!(
            self.state.connection,
            Connection::Connecting | Connection::Connected | Connection::Reconnecting
        ) {
            return Err(PartyError::local("already_in_room"));
        }
        let request = match request {
            Request::Join { code, create } => Request::Join {
                code: normalize_code(&code).ok_or_else(|| PartyError::local("invalid_code"))?,
                create,
            },
            r => r,
        };
        self.reset();
        self.away = false;
        self.state.end_reason = None;
        self.state.connection = Connection::Connecting;
        self.request = Some(request);
        self.tried.clear();
        self.answer_by = Some(now + CONNECT_TIMEOUT_MS);
        Ok(())
    }

    pub fn on_open(&mut self, url: &str, now: i64, clock: Option<Clock>) {
        let msg = match (self.state.connection, &self.request, &self.link) {
            (Connection::Connecting, Some(Request::Create), _) => ClientMsg::Create {
                name: self.name.clone(),
                client: self.client.clone(),
            },
            (Connection::Connecting, Some(Request::Join { code, create }), _) => ClientMsg::Join {
                code: code.clone(),
                name: self.name.clone(),
                client: self.client.clone(),
                create: *create,
            },
            (Connection::Reconnecting, _, Some(l)) if !self.open => ClientMsg::Resume {
                code: l.code.clone(),
                peer_id: l.peer_id.clone(),
                token: l.token.clone(),
            },
            _ => {
                self.out.push(Output::Close(1000));
                return;
            }
        };
        if self.state.connection == Connection::Connecting {
            self.tried.push(url.into());
        }
        self.url = Some(url.into());
        self.open = true;
        self.attempt_at = None;
        self.clock = Some(clock.unwrap_or_else(|| Clock::new(now)));
        self.rev = 0;
        self.last_recv = now;
        self.answer_by = Some(now + CONNECT_TIMEOUT_MS);
        self.send(msg);
        self.ping(now);
    }

    pub fn on_connect_failed(&mut self, now: i64) {
        match self.state.connection {
            Connection::Connecting => self.fail(PartyError::local("unreachable")),
            Connection::Reconnecting if self.answer_by.is_some() && !self.open => {
                self.attempt_failed(now)
            }
            _ => {}
        }
    }

    pub fn on_drop(&mut self, now: i64, code: Option<u16>) {
        if !self.open {
            return;
        }
        self.open = false;
        match self.state.connection {
            Connection::Connecting => self.fail(PartyError::local("unreachable")),
            Connection::Connected if code == Some(4000) => self.end(EndReason::Replaced, None),
            Connection::Connected => {
                self.state.connection = Connection::Reconnecting;
                self.dropped_at = now;
                self.attempt = 0;
                self.old_clock = self.clock.take();
                self.last_status = None;
                self.schedule_attempt(now);
            }
            Connection::Reconnecting => self.attempt_failed(now),
            _ => {}
        }
    }

    pub fn on_text(&mut self, now: i64, text: &str) {
        self.last_recv = now;
        if self.cancel_by.is_some() {
            return self.on_cancelled_answer(text);
        }
        match parse(text) {
            ServerMsg::Pong { id, server_time } => {
                if let (Some(clock), Some(id)) = (self.clock.as_mut(), id.as_u64()) {
                    clock.on_pong(id, server_time, now);
                }
            }
            ServerMsg::Joined(j) => self.on_joined(*j, now),
            ServerMsg::Malformed(kind) if kind == "joined" => match self.state.connection {
                Connection::Connecting => {
                    self.send(ClientMsg::Leave);
                    self.fail(PartyError::local("invalid_message"));
                }
                Connection::Reconnecting => self.attempt_failed(now),
                _ => {}
            },
            ServerMsg::Room {
                room,
                action,
                media,
            } if self.state.connection == Connection::Connected => {
                self.on_room(*room, action, media, now)
            }
            ServerMsg::Error(e) => self.on_error(e.into(), now),
            ServerMsg::Closed(reason) if self.state.connection == Connection::Connected => {
                let reason = match reason.as_str() {
                    "kicked" => EndReason::Kicked,
                    "expired" => EndReason::Expired,
                    _ => EndReason::Ended,
                };
                self.end(reason, None);
            }
            _ => {}
        }
    }

    fn on_cancelled_answer(&mut self, text: &str) {
        match parse(text) {
            ServerMsg::Joined(_) => self.send(ClientMsg::Leave),
            ServerMsg::Error(e) if matches!(e.re.as_deref(), Some("create" | "join")) => {}
            _ => return,
        }
        self.close_cancelled();
    }

    fn close_cancelled(&mut self) {
        self.cancel_by = None;
        if self.open {
            self.open = false;
            self.out.push(Output::Close(1000));
        }
    }

    fn on_joined(&mut self, j: Joined, now: i64) {
        let resumed = match self.state.connection {
            Connection::Connecting => false,
            Connection::Reconnecting if self.open => true,
            _ => return,
        };
        self.answer_by = None;
        self.request = None;
        self.link = Some(Link {
            url: self.url.clone().unwrap_or_default(),
            code: j.code,
            peer_id: j.you.peer_id.clone(),
            token: j.you.token,
        });
        self.state.connection = Connection::Connected;
        self.state.peer_id = Some(j.you.peer_id);
        self.rev = j.room.rev;
        let epoch = j.room.epoch;
        let before = self.state.room.replace(j.room);
        if resumed {
            self.diff_participants(before.as_ref());
        }
        if !resumed || self.epoch != Some(epoch) {
            self.start_epoch(j.media);
        }
        self.attempt = 0;
        self.attempt_at = None;
        self.last_status = None;
        self.step_now = true;
        self.next_step_at = now;
        self.state.player = self.derive_player(now);
        self.pace_status(now);
        if let Some((media, position_ms)) = self.pending_media.clone() {
            if self.state.is_host() {
                self.send(ClientMsg::SetMedia {
                    media: Box::new(media),
                    position_ms,
                });
            } else {
                self.pending_media = None;
                self.rebind();
            }
        }
    }

    fn on_room(&mut self, room: Room, action: Option<Action>, media: Option<Value>, now: i64) {
        if room.rev <= self.rev {
            return;
        }
        let new_epoch = self.epoch != Some(room.epoch);
        let media = match (new_epoch, media) {
            (false, _) => None,
            (true, Some(Value::Null)) => Some(None),
            (true, Some(v)) => Media::parse(v).map(Some),
            (true, None) => None,
        };
        if new_epoch && media.is_none() {
            self.send(ClientMsg::Leave);
            self.end(EndReason::Lost, Some("invalid_message".into()));
            return;
        }
        self.rev = room.rev;
        let before = self.state.room.replace(room);
        if let Some(media) = media {
            self.start_epoch(media);
        }
        self.diff_participants(before.as_ref());
        if let Some(a) = action {
            self.hold = None;
            let by_me = a.by.is_some() && a.by == self.state.peer_id;
            self.events.push(Event::Action {
                kind: a.kind,
                by: a.by,
                by_me,
            });
        }
        self.step_now = true;
        self.tick(now);
    }

    fn on_error(&mut self, err: PartyError, now: i64) {
        let re = err.re.as_deref();
        match self.state.connection {
            Connection::Connecting if matches!(re, Some("create" | "join")) => {
                self.answer_by = None;
                self.out.push(Output::Refused(err));
            }
            Connection::Reconnecting if self.open && re == Some("resume") => {
                if matches!(err.code.as_str(), "resume_failed" | "client_blocked") {
                    self.end(EndReason::Lost, Some(err.code));
                } else {
                    self.attempt_failed(now);
                }
            }
            Connection::Connected => {
                if re == Some("setMedia") {
                    self.pending_media = None;
                    self.rebind();
                }
                if self.hold.is_some_and(|(_, kind)| Some(kind) == re) {
                    self.hold = None;
                    self.step_now = true;
                }
                self.events.push(Event::Error(err));
            }
            _ => {}
        }
    }

    pub fn tick(&mut self, now: i64) {
        if self.cancel_by.is_some_and(|t| now >= t) {
            self.close_cancelled();
        }
        match self.state.connection {
            Connection::Connecting if self.answer_by.is_some_and(|t| now >= t) => {
                return self.fail(PartyError::local("unreachable"));
            }
            Connection::Reconnecting => {
                if now - self.dropped_at >= RESUME_WINDOW_MS {
                    return self.end(EndReason::Lost, Some("unreachable".into()));
                }
                if self.answer_by.is_some_and(|t| now >= t) {
                    self.attempt_failed(now);
                }
                if let Some(link) = &self.link
                    && !self.open
                    && self.attempt_at.is_some_and(|t| now >= t)
                {
                    self.attempt_at = None;
                    self.answer_by = Some(now + CONNECT_TIMEOUT_MS);
                    self.out.push(Output::Connect(link.url.clone()));
                }
            }
            Connection::Connected if now - self.last_recv >= SILENCE_MS => {
                self.out.push(Output::Close(1000));
                self.on_drop(now, None);
            }
            Connection::Idle | Connection::Ended => return,
            _ => {}
        }
        self.ping(now);
        if !matches!(
            self.state.connection,
            Connection::Connected | Connection::Reconnecting
        ) {
            return;
        }
        if self.state.connection == Connection::Connected
            && self.clock.as_ref().is_some_and(|c| c.best().is_some())
        {
            self.old_clock = None;
        }
        if self.hold.is_some_and(|(t, _)| now >= t) {
            self.hold = None;
            self.step_now = true;
        }
        if self.lane_until.is_some_and(|t| now >= t) {
            self.lane_until = None;
            self.seek_in_lane = false;
        }
        if self.play_at.is_some_and(|t| now >= t) {
            self.play_at = None;
            self.step_now = true;
        }
        if self.step_now || now >= self.next_step_at {
            self.run_step(now);
        }
        self.state.player = self.derive_player(now);
        if self.state.connection == Connection::Connected {
            self.pace_status(now);
        }
    }

    pub fn set_player(&mut self, epoch: i64, feed: PlayerFeed, now: i64) {
        if self.epoch != Some(epoch) {
            return;
        }
        let changed = self.feed.is_none_or(|(f, _)| {
            (f.paused, f.buffering, f.loading) != (feed.paused, feed.buffering, feed.loading)
        });
        self.feed = Some((feed, now));
        self.away = false;
        self.lane_until = None;
        self.seek_in_lane = false;
        self.step_now |= changed;
        self.rebind();
    }

    pub fn detach_player(&mut self) {
        self.release();
        self.away = true;
    }

    pub fn set_match(&mut self, epoch: i64, m: Match) {
        if self.state.media.is_some() && self.epoch == Some(epoch) {
            self.state.match_ = Some(m);
            self.rebind();
        }
    }

    pub fn set_offset(&mut self, offset_ms: i64) -> Result<(), PartyError> {
        if !(-MAX_DURATION_MS..=MAX_DURATION_MS).contains(&offset_ms) {
            return Err(PartyError::local("invalid_message"));
        }
        self.state.offset_ms = offset_ms;
        self.step_now = true;
        Ok(())
    }

    pub fn play(&mut self) -> Result<(), PartyError> {
        self.controller()?;
        self.send(ClientMsg::Play);
        Ok(())
    }

    pub fn pause(&mut self, now: i64) -> Result<(), PartyError> {
        self.controller()?;
        let position_ms = self
            .bound
            .then(|| (self.player_position(now) - self.state.offset_ms).max(0));
        if self.bound {
            self.command(PlayerCommand::Pause, now);
        }
        self.send(ClientMsg::Pause { position_ms });
        self.hold = Some((now + HOLD_MS, "pause"));
        Ok(())
    }

    pub fn seek(&mut self, position_ms: i64, now: i64) -> Result<(), PartyError> {
        self.controller()?;
        if !(0..=MAX_INT).contains(&position_ms) {
            return Err(PartyError::local("invalid_message"));
        }
        if self.bound {
            self.command(PlayerCommand::Seek(position_ms), now);
        }
        self.send(ClientMsg::Seek {
            position_ms: (position_ms - self.state.offset_ms).max(0),
        });
        self.hold = Some((now + HOLD_MS, "seek"));
        Ok(())
    }

    pub fn set_rate(&mut self, rate: f64, now: i64) -> Result<(), PartyError> {
        self.controller()?;
        if !(0.5..=2.0).contains(&rate) {
            return Err(PartyError::local("invalid_message"));
        }
        if self.bound && self.feed.is_some_and(|(f, _)| f.can_set_rate) {
            self.command(PlayerCommand::SetRate(follow::round_rate(rate)), now);
        }
        self.send(ClientMsg::Rate { rate });
        self.hold = Some((now + HOLD_MS, "rate"));
        Ok(())
    }

    pub fn set_media(&mut self, media: Media, position_ms: i64) -> Result<(), PartyError> {
        self.host()?;
        if media.check(true).is_err() || !(0..=MAX_INT).contains(&position_ms) {
            return Err(PartyError::local("invalid_message"));
        }
        self.own_media = Some(media.clone());
        self.pending_media = Some((media.clone(), position_ms));
        self.rebind();
        self.send(ClientMsg::SetMedia {
            media: Box::new(media),
            position_ms,
        });
        Ok(())
    }

    pub fn clear_media(&mut self) -> Result<(), PartyError> {
        self.host_send(ClientMsg::ClearMedia)
    }

    pub fn set_control(&mut self, mode: Control) -> Result<(), PartyError> {
        self.host_send(ClientMsg::SetControl { mode })
    }

    pub fn lock(&mut self, locked: bool) -> Result<(), PartyError> {
        self.host_send(ClientMsg::Lock { locked })
    }

    pub fn end_room(&mut self) -> Result<(), PartyError> {
        self.host_send(ClientMsg::End)
    }

    pub fn transfer_host(&mut self, peer_id: &str) -> Result<(), PartyError> {
        self.known_peer(peer_id)?;
        self.host_send(ClientMsg::TransferHost {
            peer_id: peer_id.into(),
        })
    }

    pub fn kick(&mut self, peer_id: &str) -> Result<(), PartyError> {
        self.known_peer(peer_id)?;
        self.host_send(ClientMsg::Kick {
            peer_id: peer_id.into(),
        })
    }

    pub fn leave(&mut self) {
        match self.state.connection {
            Connection::Connected => {
                self.send(ClientMsg::Leave);
                self.end(EndReason::Left, None);
            }
            Connection::Reconnecting => self.end(EndReason::Left, None),
            Connection::Connecting if self.open => {
                let cancel_by = self.answer_by.take();
                self.open = false;
                self.fail(PartyError::local("cancelled"));
                self.open = true;
                self.cancel_by = cancel_by;
            }
            Connection::Connecting => self.fail(PartyError::local("cancelled")),
            _ => {}
        }
    }

    pub fn retry(&mut self, now: i64) {
        if self.open {
            self.out.push(Output::Close(1000));
            self.open = false;
        }
        self.clock = None;
        self.answer_by = Some(now + CONNECT_TIMEOUT_MS);
    }

    pub fn fail(&mut self, err: PartyError) {
        self.reset();
        self.request = None;
        self.state.connection = Connection::Idle;
        self.events.push(Event::Failed(err));
    }

    fn end(&mut self, reason: EndReason, error: Option<String>) {
        self.release();
        self.reset();
        self.own_media = None;
        self.state.connection = Connection::Ended;
        self.state.end_reason = Some(reason);
        self.events.push(Event::Ended { reason, error });
    }

    fn reset(&mut self) {
        self.cancel_by = None;
        if self.open {
            self.out.push(Output::Close(1000));
            self.open = false;
        }
        let (connection, end_reason) = (self.state.connection, self.state.end_reason);
        self.state = PartyState {
            connection,
            end_reason,
            ..PartyState::default()
        };
        self.link = None;
        self.url = None;
        self.answer_by = None;
        self.clock = None;
        self.old_clock = None;
        self.rev = 0;
        self.epoch = None;
        self.pending_media = None;
        self.bound = false;
        self.nudging = false;
        self.hold = None;
        self.play_at = None;
        self.lane_until = None;
        self.seek_in_lane = false;
        self.last_status = None;
        self.attempt = 0;
        self.attempt_at = None;
    }

    fn release(&mut self) {
        if self.nudging
            && let Some(room) = &self.state.room
        {
            self.out
                .push(Output::Player(PlayerCommand::SetRate(follow::round_rate(
                    room.rate,
                ))));
        }
        self.feed = None;
        self.bound = false;
        self.nudging = false;
        self.play_at = None;
        self.lane_until = None;
        self.seek_in_lane = false;
    }

    fn start_epoch(&mut self, media: Option<Media>) {
        self.epoch = self.state.room.as_ref().map(|r| r.epoch);
        self.state.offset_ms = 0;
        self.pending_media = None;
        let own = media.is_some() && media == self.own_media;
        self.state.match_ = match &media {
            None => None,
            Some(_) if own => Some(Match {
                state: MatchState::Matched,
                delta_ms: Some(0),
            }),
            Some(_) => Some(Match {
                state: MatchState::Searching,
                delta_ms: None,
            }),
        };
        self.state.media = media;
        self.bound = false;
        if !own && self.feed.is_some() {
            self.out.push(Output::Player(PlayerCommand::Pause));
            self.release();
        }
        self.rebind();
    }

    fn rebind(&mut self) {
        let matched = self
            .state
            .match_
            .is_some_and(|m| matches!(m.state, MatchState::Matched | MatchState::Mismatched));
        let bound = matched
            && self.feed.is_some()
            && self.state.media.is_some()
            && self.pending_media.is_none();
        if bound && !self.bound {
            self.nudging = false;
            self.applied_rate = self.state.room.as_ref().map_or(1.0, |r| r.rate);
            if self.feed.is_some_and(|(f, _)| f.can_set_rate) {
                self.out
                    .push(Output::Player(PlayerCommand::SetRate(follow::round_rate(
                        self.applied_rate,
                    ))));
            }
            self.step_now = true;
        }
        if !bound {
            self.play_at = None;
        }
        self.bound = bound;
    }

    fn player_position(&self, now: i64) -> i64 {
        self.feed.map_or(0, |(f, at)| {
            let moving = !f.paused && !f.buffering && !f.loading;
            if moving {
                f.position_ms + ((now - at) as f64 * self.applied_rate) as i64
            } else {
                f.position_ms
            }
        })
    }

    fn run_step(&mut self, now: i64) {
        if self.hold.is_some() || self.lane_until.is_some() {
            return;
        }
        self.step_now = false;
        self.next_step_at = now + follow::TICK_MS;
        let (Some((feed, _)), Some(room), Some(media), Some(relay_now)) = (
            self.feed,
            &self.state.room,
            &self.state.media,
            self.relay_now(now),
        ) else {
            return;
        };
        if !self.bound {
            return;
        }
        let player = PlayerFeed {
            position_ms: self.player_position(now),
            ..feed
        };
        let step = follow::step(&StepIn {
            room,
            duration_ms: media.duration_ms,
            relay_now,
            offset_ms: self.state.offset_ms,
            player: &player,
            applied_rate: self.applied_rate,
            nudging: self.nudging,
            since_seek_ms: self.last_seek_at.map(|t| now - t),
        });
        self.nudging = step.nudging;
        self.play_at = step.play_in_ms.map(|d| now + d);
        for action in step.actions {
            self.command(action, now);
        }
    }

    fn command(&mut self, action: PlayerCommand, now: i64) {
        match action {
            PlayerCommand::SetRate(r) => self.applied_rate = r,
            PlayerCommand::Seek(_) => {
                self.last_seek_at = Some(now);
                self.seek_in_lane = true;
            }
            _ => {}
        }
        self.lane_until = Some(now + LANE_MS);
        self.out.push(Output::Player(action));
    }

    fn derive_player(&self, now: i64) -> PlayerState {
        let (Some(room), Some(_)) = (&self.state.room, &self.state.media) else {
            return PlayerState::Idle;
        };
        if self
            .state
            .match_
            .is_some_and(|m| m.state == MatchState::Unavailable)
            || self.away
        {
            return PlayerState::Idle;
        }
        let Some((feed, _)) = self.feed.filter(|_| self.bound) else {
            return PlayerState::Loading;
        };
        let relay_now = self.relay_now(now);
        if feed.loading
            || self.seek_in_lane
            || (room.phase == Phase::Playing && relay_now.is_none())
        {
            return PlayerState::Loading;
        }
        let relay_now = relay_now.unwrap_or(0);
        let held = follow::hold_at(room, relay_now, self.state.offset_ms, feed.duration_ms);
        if held.is_some_and(|(at, _)| {
            (self.player_position(now) - at).abs() > feed.seek_accuracy_ms.max(500)
        }) {
            return PlayerState::Loading;
        }
        match (feed.buffering, held.is_none() && room.running(relay_now)) {
            (true, true) => PlayerState::Buffering,
            (true, false) => PlayerState::Loading,
            _ => PlayerState::Ready,
        }
    }

    fn pace_status(&mut self, now: i64) {
        let Some(epoch) = self.epoch else { return };
        let status = Status {
            epoch,
            player: self.state.player,
            match_: self.state.match_,
            offset_ms: self.state.offset_ms,
        };
        let due = match self.last_status {
            None => true,
            Some((last, _)) if last.epoch != epoch => true,
            Some((last, at)) if last != status => now - at >= SPACING_MS,
            Some((_, at)) => now - at >= HEARTBEAT_MS,
        };
        if due {
            let rtt_ms = self.clock.as_ref().and_then(Clock::best).map(|b| b.rtt_ms);
            self.send(ClientMsg::Status {
                epoch,
                player: status.player,
                match_: status.match_,
                offset_ms: status.offset_ms,
                rtt_ms,
            });
            self.last_status = Some((status, now));
        }
    }

    fn diff_participants(&mut self, before: Option<&Room>) {
        let (Some(before), Some(after)) = (before, &self.state.room) else {
            return;
        };
        let joined = after
            .participants
            .iter()
            .filter(|p| before.participant(&p.peer_id).is_none());
        let left = before
            .participants
            .iter()
            .filter(|p| after.participant(&p.peer_id).is_none());
        let joined: Vec<_> = joined.cloned().map(Event::ParticipantJoined).collect();
        self.events.extend(joined);
        self.events
            .extend(left.cloned().map(Event::ParticipantLeft));
    }

    fn ping(&mut self, now: i64) {
        if let Some(id) = self
            .clock
            .as_mut()
            .filter(|_| self.open)
            .and_then(|c| c.due_ping(now))
        {
            self.send(ClientMsg::Ping { id: id.into() });
        }
    }

    fn schedule_attempt(&mut self, now: i64) {
        self.attempt += 1;
        let u = self.random();
        self.attempt_at = Some(now + follow::backoff_ms(self.attempt, u));
    }

    fn attempt_failed(&mut self, now: i64) {
        if self.open {
            self.out.push(Output::Close(1000));
            self.open = false;
        }
        self.answer_by = None;
        self.clock = None;
        self.schedule_attempt(now);
    }

    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    fn send(&mut self, msg: ClientMsg) {
        self.out.push(Output::Send(msg.to_json()));
    }

    fn member(&self) -> Result<(), PartyError> {
        if self.state.connection == Connection::Connected {
            Ok(())
        } else {
            Err(PartyError::local("not_in_room"))
        }
    }

    fn controller(&self) -> Result<(), PartyError> {
        self.member()?;
        if self.state.media.is_none() {
            return Err(PartyError::local("invalid_message"));
        }
        if self.state.can_control() {
            Ok(())
        } else {
            Err(PartyError::local("forbidden"))
        }
    }

    fn host(&self) -> Result<(), PartyError> {
        self.member()?;
        if self.state.is_host() {
            Ok(())
        } else {
            Err(PartyError::local("forbidden"))
        }
    }

    fn host_send(&mut self, msg: ClientMsg) -> Result<(), PartyError> {
        self.host()?;
        self.send(msg);
        Ok(())
    }

    fn known_peer(&self, peer_id: &str) -> Result<(), PartyError> {
        self.host()?;
        match self
            .state
            .room
            .as_ref()
            .and_then(|r| r.participant(peer_id))
        {
            Some(_) => Ok(()),
            None => Err(PartyError::local("invalid_message")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OFFSET: i64 = 1_000_000;
    const URL: &str = "wss://relay.test/v1/ws";
    const DURATION: i64 = 6_960_000;

    #[derive(Default)]
    struct Drained {
        sent: Vec<Value>,
        commands: Vec<PlayerCommand>,
        other: Vec<Output>,
    }

    fn drain(p: &mut Party, now: i64) -> Drained {
        let mut d = Drained::default();
        loop {
            let outputs = p.take_outputs();
            if outputs.is_empty() {
                return d;
            }
            for o in outputs {
                match o {
                    Output::Send(text) => {
                        let v: Value = serde_json::from_str(&text).unwrap();
                        if v["type"] == "ping" {
                            p.on_text(
                                now,
                                &json!({"type": "pong", "id": v["id"], "serverTime": now + OFFSET})
                                    .to_string(),
                            );
                        } else {
                            d.sent.push(v);
                        }
                    }
                    Output::Player(c) => d.commands.push(c),
                    o => d.other.push(o),
                }
            }
        }
    }

    fn snapshot(rev: i64, epoch: i64, phase: &str, position: i64, at: i64, host: bool) -> Value {
        let participant = |id: &str, host: bool| json!({"peerId": id, "name": id, "client": "Test/1", "host": host, "connected": true, "player": "idle", "match": null, "offsetMs": 0});
        json!({
            "rev": rev, "code": "K7MQ2X", "epoch": epoch, "phase": phase, "positionMs": position, "atMs": at, "rate": 1.0,
            "control": "host", "locked": false, "waitingOn": [], "participants": [participant("p1", host), participant("p2", !host)]
        })
    }

    fn movie() -> Value {
        json!({"kind": "movie", "title": "Arrival", "durationMs": DURATION})
    }

    fn room_msg(mut room: Value, media: Option<Value>, action: Option<Value>) -> String {
        room["type"] = "room".into();
        if let Some(m) = media {
            room["media"] = m;
        }
        if let Some(a) = action {
            room["action"] = a;
        }
        room.to_string()
    }

    fn joined(now: i64, room: Value, media: Value) -> Party {
        let mut p = Party::new("Tester", "Plaivo/0.1.0", 7).unwrap();
        p.start(
            Request::Join {
                code: "k7m-q2x".into(),
                create: false,
            },
            now,
        )
        .unwrap();
        p.on_open(URL, now, None);
        let d = drain(&mut p, now);
        assert_eq!(
            d.sent[0],
            json!({"type": "join", "code": "K7MQ2X", "name": "Tester", "client": "Plaivo/0.1.0"})
        );
        let token = "a".repeat(43);
        p.on_text(now, &json!({"type": "joined", "code": "K7MQ2X", "you": {"peerId": "p1", "token": token}, "room": room, "media": media, "serverTime": 0}).to_string());
        assert_eq!(p.state().connection, Connection::Connected);
        p
    }

    struct Sim {
        pos: f64,
        paused: bool,
        rate: f64,
        ignore_play_until: i64,
    }

    impl Sim {
        fn apply(&mut self, c: PlayerCommand, now: i64) {
            match c {
                PlayerCommand::Pause => self.paused = true,
                PlayerCommand::Play if now >= self.ignore_play_until => self.paused = false,
                PlayerCommand::Play => {}
                PlayerCommand::Seek(p) => self.pos = p as f64,
                PlayerCommand::SetRate(r) => self.rate = r,
            }
        }

        fn feed(&self) -> PlayerFeed {
            PlayerFeed {
                position_ms: self.pos as i64,
                duration_ms: Some(DURATION),
                paused: self.paused,
                buffering: false,
                loading: false,
                can_set_rate: true,
                seek_accuracy_ms: 0,
            }
        }
    }

    fn run(
        p: &mut Party,
        sim: &mut Sim,
        now: &mut i64,
        ms: i64,
        log: &mut Vec<(i64, PlayerCommand)>,
    ) {
        for _ in 0..ms / 100 {
            if !sim.paused {
                sim.pos += 100.0 * sim.rate;
            }
            *now += 100;
            p.set_player(1, sim.feed(), *now);
            p.tick(*now);
            for c in drain(p, *now).commands {
                log.push((*now, c));
                sim.apply(c, *now);
            }
        }
    }

    #[test]
    fn follows_a_playing_room_with_a_simulated_player() {
        let mut now = 10_000;
        let start = now + OFFSET + 1000;
        let mut p = joined(
            now,
            snapshot(5, 1, "playing", 60_000, start, false),
            movie(),
        );
        assert_eq!(p.state().match_.unwrap().state, MatchState::Searching);
        p.set_match(
            1,
            Match {
                state: MatchState::Matched,
                delta_ms: Some(0),
            },
        );
        let mut sim = Sim {
            pos: 0.0,
            paused: true,
            rate: 1.0,
            ignore_play_until: now + 3600,
        };
        let mut log = Vec::new();
        run(&mut p, &mut sim, &mut now, 900, &mut log);
        assert!(
            log.iter().any(|(_, c)| *c == PlayerCommand::Seek(60_000)),
            "{log:?}"
        );
        assert!(sim.paused);
        assert_eq!(p.state().player, PlayerState::Ready);

        run(&mut p, &mut sim, &mut now, 8000, &mut log);
        let target = |now: i64| 60_000 + (now + OFFSET - start);
        assert!(
            log.iter()
                .any(|(_, c)| matches!(c, PlayerCommand::Seek(s) if *s > 62_000)),
            "late start is caught up by a seek: {log:?}"
        );
        assert!(
            (sim.pos as i64 - target(now)).abs() <= 350,
            "drift {}",
            sim.pos as i64 - target(now)
        );

        log.clear();
        sim.pos -= 600.0;
        run(&mut p, &mut sim, &mut now, 20_000, &mut log);
        let rates: Vec<f64> = log
            .iter()
            .filter_map(|(_, c)| {
                if let PlayerCommand::SetRate(r) = c {
                    Some(*r)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(rates, [1.04, 1.0], "{log:?}");
        assert!((sim.pos as i64 - target(now)).abs() < 150);
        assert!(!log.iter().any(|(_, c)| matches!(c, PlayerCommand::Seek(_))));

        let pause_at = now + OFFSET;
        let position = target(now);
        p.on_text(
            now,
            &room_msg(
                snapshot(6, 1, "paused", position, pause_at, false),
                None,
                Some(json!({"kind": "pause", "by": "p2"})),
            ),
        );
        let events = p.take_events();
        assert_eq!(
            events,
            [Event::Action {
                kind: "pause".into(),
                by: Some("p2".into()),
                by_me: false
            }]
        );
        log.clear();
        run(&mut p, &mut sim, &mut now, 1000, &mut log);
        assert!(sim.paused);
        assert!((sim.pos as i64 - position).abs() <= 500);

        p.on_text(
            now,
            &room_msg(
                snapshot(7, 1, "paused", 1_200_000, now + OFFSET, false),
                None,
                Some(json!({"kind": "seek", "by": "p2"})),
            ),
        );
        run(&mut p, &mut sim, &mut now, 3000, &mut log);
        assert_eq!(sim.pos as i64, 1_200_000);
        assert!(sim.paused);
    }

    #[test]
    fn status_is_spaced_250_ms_with_a_15_s_heartbeat() {
        let mut now = 0;
        let mut p = joined(now, snapshot(1, 1, "paused", 0, 0, false), movie());
        let statuses = |p: &mut Party, now: i64| {
            drain(p, now)
                .sent
                .into_iter()
                .filter(|m| m["type"] == "status")
                .collect::<Vec<_>>()
        };
        let first = statuses(&mut p, now);
        assert_eq!(
            first,
            [
                json!({"type": "status", "epoch": 1, "player": "loading", "match": {"state": "searching"}, "offsetMs": 0, "rttMs": 0})
            ]
        );
        let mut sent_at = Vec::new();
        for step in 1..=1600 {
            now = step * 10;
            match now {
                50 => p.set_match(
                    1,
                    Match {
                        state: MatchState::Mismatched,
                        delta_ms: Some(4200),
                    },
                ),
                100 => p.set_offset(4200).unwrap(),
                260 => p.set_offset(4100).unwrap(),
                _ => {}
            }
            p.tick(now);
            if !statuses(&mut p, now).is_empty() {
                sent_at.push(now);
            }
        }
        assert_eq!(sent_at, [250, 500, 15_500]);
    }

    #[test]
    fn own_media_matches_itself_and_others_media_pauses_the_player() {
        let mut p = joined(0, snapshot(1, 0, "idle", 0, 0, true), Value::Null);
        assert!(p.state().is_host());
        let media = Media::parse(movie()).unwrap();
        p.set_media(media.clone(), 0).unwrap();
        assert_eq!(drain(&mut p, 0).sent.last().unwrap()["type"], "setMedia");
        p.on_text(
            10,
            &room_msg(
                snapshot(2, 1, "waiting", 0, 0, true),
                Some(movie()),
                Some(json!({"kind": "media", "by": "p1"})),
            ),
        );
        assert_eq!(
            p.state().match_,
            Some(Match {
                state: MatchState::Matched,
                delta_ms: Some(0)
            })
        );
        assert_eq!(p.state().media.as_ref(), Some(&media));
        assert!(matches!(
            p.take_events()[..],
            [Event::Action { by_me: true, .. }]
        ));

        p.set_player(
            1,
            PlayerFeed {
                paused: true,
                duration_ms: Some(DURATION),
                ..Default::default()
            },
            20,
        );
        p.tick(20);
        drain(&mut p, 20);
        let other = json!({"kind": "movie", "title": "Dune", "durationMs": 9_000_000});
        p.on_text(
            30,
            &room_msg(snapshot(3, 2, "waiting", 0, 0, true), Some(other), None),
        );
        assert_eq!(p.state().match_.unwrap().state, MatchState::Searching);
        assert_eq!(drain(&mut p, 30).commands, [PlayerCommand::Pause]);
        assert_eq!(p.state().player, PlayerState::Loading);

        p.on_text(
            40,
            &room_msg(
                snapshot(4, 3, "waiting", 0, 0, true),
                Some(json!({"kind": "show"})),
                None,
            ),
        );
        assert_eq!(p.state().end_reason, Some(EndReason::Lost));
    }

    #[test]
    fn reconnects_with_backoff_and_resumes() {
        let mut now = 0;
        let mut p = joined(now, snapshot(1, 0, "idle", 0, 0, false), Value::Null);
        drain(&mut p, now);
        p.on_drop(now, Some(1006));
        assert_eq!(p.state().connection, Connection::Reconnecting);
        let mut attempts = Vec::new();
        let mut last = now;
        while attempts.len() < 3 {
            now += 10;
            p.tick(now);
            if drain(&mut p, now)
                .other
                .contains(&Output::Connect(URL.into()))
            {
                attempts.push(now - last);
                last = now;
                if attempts.len() < 3 {
                    p.on_connect_failed(now);
                }
            }
        }
        assert!((250..=510).contains(&attempts[0]), "{attempts:?}");
        assert!((500..=1010).contains(&attempts[1]), "{attempts:?}");
        assert!((1000..=2010).contains(&attempts[2]), "{attempts:?}");
        assert!(p.play().is_err());

        p.on_open(URL, now, None);
        let sent = drain(&mut p, now).sent;
        assert_eq!(
            sent,
            [json!({"type": "resume", "code": "K7MQ2X", "peerId": "p1", "token": "a".repeat(43)})]
        );
        let mut room = snapshot(4, 0, "idle", 0, 0, false);
        room["participants"][1]["peerId"] = "p3".into();
        p.on_text(now, &json!({"type": "joined", "code": "K7MQ2X", "you": {"peerId": "p1", "token": "a".repeat(43)}, "room": room, "media": null, "serverTime": 0}).to_string());
        assert_eq!(p.state().connection, Connection::Connected);
        assert_eq!(drain(&mut p, now).sent[0]["type"], "status");
        let events = p.take_events();
        assert!(
            matches!(&events[..], [Event::ParticipantJoined(a), Event::ParticipantLeft(b)] if a.peer_id == "p3" && b.peer_id == "p2"),
            "{events:?}"
        );
    }

    #[test]
    fn resume_ends_after_refusal_window_or_takeover() {
        let mut p = joined(0, snapshot(1, 0, "idle", 0, 0, false), Value::Null);
        p.on_drop(0, None);
        p.tick(600);
        p.on_open(URL, 600, None);
        p.on_text(
            700,
            r#"{"type":"error","code":"resume_failed","message":"","re":"resume"}"#,
        );
        assert_eq!(p.state().end_reason, Some(EndReason::Lost));
        assert!(
            matches!(&p.take_events()[..], [Event::Ended { reason: EndReason::Lost, error: Some(e) }] if e == "resume_failed")
        );

        let mut p = joined(0, snapshot(1, 0, "idle", 0, 0, false), Value::Null);
        p.on_drop(0, None);
        let mut now = 0;
        while p.state().connection == Connection::Reconnecting {
            now += 100;
            p.tick(now);
            if drain(&mut p, now)
                .other
                .contains(&Output::Connect(URL.into()))
            {
                p.on_connect_failed(now);
            }
        }
        assert_eq!(now, RESUME_WINDOW_MS);
        assert!(
            matches!(&p.take_events()[..], [Event::Ended { reason: EndReason::Lost, error: Some(e) }] if e == "unreachable")
        );

        let mut p = joined(0, snapshot(1, 0, "idle", 0, 0, false), Value::Null);
        p.on_drop(0, Some(4000));
        assert_eq!(p.state().end_reason, Some(EndReason::Replaced));
    }

    #[test]
    fn commands_check_membership_role_and_media() {
        let mut p = Party::new("Tester", "Plaivo/0.1.0", 1).unwrap();
        assert_eq!(p.play().unwrap_err().code, "not_in_room");
        assert_eq!(
            p.start(
                Request::Join {
                    code: "nope".into(),
                    create: false
                },
                0
            )
            .unwrap_err()
            .code,
            "invalid_code"
        );
        assert!(Party::new("Tester", "Plaivo", 1).is_err());

        let mut p = joined(0, snapshot(1, 0, "idle", 0, 0, false), Value::Null);
        assert_eq!(p.play().unwrap_err().code, "invalid_message");
        assert_eq!(p.kick("p2").unwrap_err().code, "forbidden");
        let mut p = joined(0, snapshot(1, 1, "paused", 0, 0, false), movie());
        assert_eq!(p.pause(0).unwrap_err().code, "forbidden");
        let mut p = joined(0, snapshot(1, 1, "paused", 0, 0, true), movie());
        assert_eq!(p.kick("p9").unwrap_err().code, "invalid_message");
        assert_eq!(p.set_rate(2.5, 0).unwrap_err().code, "invalid_message");
        p.set_offset(1000).unwrap();
        drain(&mut p, 0);
        p.seek(5000, 0).unwrap();
        assert_eq!(
            drain(&mut p, 0).sent,
            [json!({"type": "seek", "positionMs": 4000})]
        );
        p.leave();
        assert_eq!(drain(&mut p, 0).sent, [json!({"type": "leave"})]);
        assert_eq!(p.state().end_reason, Some(EndReason::Left));
    }

    #[test]
    fn closed_kicked_ends_the_party() {
        let mut p = joined(0, snapshot(1, 0, "idle", 0, 0, false), Value::Null);
        p.on_text(5, r#"{"type":"closed","reason":"kicked"}"#);
        assert_eq!(p.state().connection, Connection::Ended);
        assert_eq!(p.state().end_reason, Some(EndReason::Kicked));
        assert!(drain(&mut p, 5).other.contains(&Output::Close(1000)));
    }

    #[test]
    fn stale_match_and_feed_from_an_earlier_epoch_are_dropped() {
        let mut p = joined(0, snapshot(1, 1, "paused", 0, 0, false), movie());
        let other = json!({"kind": "movie", "title": "Dune", "durationMs": 9_000_000});
        p.on_text(
            10,
            &room_msg(snapshot(2, 2, "paused", 0, 0, false), Some(other), None),
        );
        p.set_match(
            1,
            Match {
                state: MatchState::Matched,
                delta_ms: Some(0),
            },
        );
        p.set_player(
            1,
            PlayerFeed {
                paused: true,
                ..Default::default()
            },
            20,
        );
        assert_eq!(p.state().match_.unwrap().state, MatchState::Searching);
        p.tick(30);
        assert!(drain(&mut p, 30).commands.is_empty());
        assert_eq!(p.state().player, PlayerState::Loading);
        p.set_match(
            2,
            Match {
                state: MatchState::Matched,
                delta_ms: Some(0),
            },
        );
        p.set_player(
            2,
            PlayerFeed {
                paused: true,
                can_set_rate: true,
                ..Default::default()
            },
            40,
        );
        assert!(p.bound);
    }

    #[test]
    fn cancelling_after_the_request_keeps_the_socket_until_the_answer() {
        let mut p = Party::new("Tester", "Plaivo/0.1.0", 1).unwrap();
        p.start(Request::Create, 0).unwrap();
        p.on_open(URL, 100, None);
        drain(&mut p, 100);
        p.leave();
        assert_eq!(p.state().connection, Connection::Idle);
        assert!(matches!(&p.take_events()[..], [Event::Failed(e)] if e.code == "cancelled"));
        assert!(drain(&mut p, 100).other.is_empty());
        let token = "a".repeat(43);
        p.on_text(200, &json!({"type": "joined", "code": "K7MQ2X", "you": {"peerId": "p1", "token": token}, "room": snapshot(1, 0, "idle", 0, 0, true), "media": null, "serverTime": 0}).to_string());
        let d = drain(&mut p, 200);
        assert_eq!(d.sent, [json!({"type": "leave"})]);
        assert_eq!(d.other, [Output::Close(1000)]);
        assert_eq!(p.state().connection, Connection::Idle);

        let mut p = Party::new("Tester", "Plaivo/0.1.0", 1).unwrap();
        p.start(Request::Create, 0).unwrap();
        p.on_open(URL, 100, None);
        p.leave();
        drain(&mut p, 100);
        p.tick(100 + CONNECT_TIMEOUT_MS);
        assert_eq!(
            drain(&mut p, 100 + CONNECT_TIMEOUT_MS).other,
            [Output::Close(1000)]
        );
    }

    #[test]
    fn connecting_has_one_overall_deadline() {
        let mut p = Party::new("Tester", "Plaivo/0.1.0", 1).unwrap();
        p.start(Request::Create, 0).unwrap();
        assert_eq!(p.next_deadline(), Some(CONNECT_TIMEOUT_MS));
        p.tick(CONNECT_TIMEOUT_MS);
        assert!(matches!(&p.take_events()[..], [Event::Failed(e)] if e.code == "unreachable"));
        assert_eq!(p.next_deadline(), None);
    }
}
