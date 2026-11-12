use crate::app::{Page, Plaivo};
use crate::plex::{Ctx, Item, Source};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::theme::Icon;
use futures::StreamExt;
use futures::channel::mpsc::UnboundedSender;
use gpui::{AppContext, Context, Entity, SharedString};
use plaivo_core::watch_together::{
    Connection, Event, Match, MatchState, Media, PartyError, PartyState, PlaivoExt, PlayerCommand,
    PlayerFeed, Relays, Update, WatchTogether, clean, describe, find_copy, match_for,
    normalize_code,
};
use std::time::{Duration, Instant};

const CLIENT: &str = concat!("Plaivo/", env!("CARGO_PKG_VERSION"));
const FEED_EVERY: Duration = Duration::from_millis(400);
const SAVE_AFTER: Duration = Duration::from_millis(600);
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
    pub account_name: Option<String>,
}

impl Watch {
    pub fn new(name: &str, relay: &str, cx: &mut Context<Plaivo>) -> Self {
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<(u64, Update)>();
        cx.spawn(async move |this, cx| {
            while let Some((generation, update)) = rx.next().await {
                if this
                    .update(cx, |this, cx| this.on_watch(generation, update, cx))
                    .is_err()
                {
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
        cx.subscribe(
            &code,
            |this: &mut Plaivo, _, ev: &InputEvent, cx| match ev {
                InputEvent::Submit(code) => this.watch_join(code.to_string(), cx),
                InputEvent::Changed(_) => {
                    if this.watch.error.take().is_some() {
                        cx.notify();
                    }
                }
            },
        )
        .detach();
        cx.subscribe(
            &name_input,
            |this: &mut Plaivo, _, ev: &InputEvent, cx| match ev {
                InputEvent::Changed(name) => {
                    this.settings.watch_name = name.trim().to_owned();
                    this.save_settings_soon(cx);
                }
                InputEvent::Submit(_) => this.save_settings_now(),
            },
        )
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
            account_name: None,
        }
    }

    pub fn in_room(&self) -> bool {
        matches!(
            self.state.connection,
            Connection::Connected | Connection::Reconnecting
        )
    }

    pub fn can_open(&self) -> bool {
        self.file.is_some()
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
    if setting.is_empty() {
        Relays::public()
    } else {
        Relays::custom(setting).unwrap_or_else(Relays::public)
    }
}

pub fn pick_name(
    setting: &str,
    account: impl FnOnce() -> Option<String>,
    jellyfin: Option<&str>,
) -> String {
    let usable = |s: &str| Some(clean(s, NAME_MAX)).filter(|s| !s.is_empty());
    usable(setting)
        .or_else(|| account().and_then(|n| usable(&n)))
        .or_else(|| jellyfin.and_then(usable))
        .unwrap_or_else(|| "Plaivo".into())
}

#[derive(Debug, PartialEq)]
pub enum Bind {
    Resolve(Box<Media>, i64),
    Own(String, String),
    Clear,
}

pub fn on_epoch(s: &PartyState) -> Bind {
    let epoch = s.room.as_ref().map(|r| r.epoch);
    match (&s.media, epoch, s.match_.map(|m| m.state)) {
        (Some(media), Some(epoch), Some(MatchState::Searching)) => {
            Bind::Resolve(Box::new(media.clone()), epoch)
        }
        (Some(media), Some(_), Some(MatchState::Matched | MatchState::Mismatched)) => media
            .plaivo()
            .map_or(Bind::Clear, |h| Bind::Own(h.server, h.item)),
        _ => Bind::Clear,
    }
}

pub fn holds(
    s: &PartyState,
    file: Option<&(String, String)>,
    playing: Option<(&str, &str)>,
) -> bool {
    matches!(
        s.connection,
        Connection::Connected | Connection::Reconnecting
    ) && s
        .match_
        .is_some_and(|m| matches!(m.state, MatchState::Matched | MatchState::Mismatched))
        && file
            .zip(playing)
            .is_some_and(|((fs, fk), (ps, pk))| fs == ps && fk == pk)
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
        w.share = None;
        let generation = w.generation;
        let (old, tx) = (w.wt.take(), w.tx.clone());
        let (setting, relays, client, token) = (
            self.settings.watch_name.clone(),
            relays(&self.settings.watch_relay),
            self.client.clone(),
            self.account.token.clone(),
        );
        let (jellyfin, mut account) = (
            self.account.jellyfin.first().map(|s| s.user.clone()),
            w.account_name.clone(),
        );
        Self::bg(
            cx,
            move || {
                drop(old);
                let name = pick_name(
                    &setting,
                    || {
                        if account.is_none() {
                            account = token.and_then(|t| client.user(&t).ok().map(|u| u.name));
                        }
                        account.clone()
                    },
                    jellyfin.as_deref(),
                );
                let wt = WatchTogether::new(&name, CLIENT, relays, move |u| {
                    let _ = tx.unbounded_send((generation, u));
                });
                if let Ok(wt) = &wt {
                    match code {
                        Some(code) => wt.join(&code, false),
                        None => wt.create(),
                    }
                }
                (wt, account)
            },
            move |this, (wt, account): (Result<WatchTogether, PartyError>, Option<String>), cx| {
                if this.watch.generation != generation {
                    return Self::detach(cx, move || drop(wt));
                }
                if account.is_some() {
                    this.watch.account_name = account;
                }
                this.watch.starting = false;
                match wt {
                    Ok(wt) => {
                        if let Some((media, at)) = this.watch.share.take() {
                            wt.set_media(media, at);
                        }
                        this.watch.wt = Some(wt);
                    }
                    Err(e) => {
                        log::warn!("watch together: could not start: {e:?}");
                        this.watch.creating = false;
                        this.watch.error = Some(
                            "Couldn't start Watch Together. Check your display name in Settings."
                                .into(),
                        );
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
        self.watch.relay_error =
            (!valid).then(|| "Enter the relay's address, starting with https:// or http://".into());
        if valid && self.settings.watch_relay != url {
            self.settings.watch_relay = url.to_owned();
            self.save_settings_soon(cx);
        }
        cx.notify();
    }

    // Settings writes are coalesced off the click path; quit flushes a pending one.
    pub fn save_settings_soon(&mut self, cx: &mut Context<Self>) {
        self.settings_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SAVE_AFTER).await;
            this.update(cx, |this, _| this.save_settings_now()).ok();
        }));
    }

    pub fn save_settings_now(&mut self) {
        if self.settings_save.take().is_some() {
            self.settings.save();
        }
    }

    pub fn use_public_relay(&mut self, public: bool, cx: &mut Context<Self>) {
        self.watch.custom_relay = !public;
        if public {
            self.watch.relay_error = None;
            self.settings.watch_relay.clear();
            self.save_settings_soon(cx);
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
        let same = self.playing()
            && self.player.key == key
            && self.player.server.as_ref().is_some_and(|s| s.id == server);
        if same {
            self.cancel_pending_play();
            let at = (self.player.time * 1000.0) as i64;
            self.watch_share(server, key, None, at, cx);
            self.expand_player(cx);
        } else {
            self.play(server, key, cx);
        }
    }

    pub fn watch_share(
        &mut self,
        server: String,
        key: String,
        item: Option<Item>,
        position_ms: i64,
        cx: &mut Context<Self>,
    ) {
        let w = &mut self.watch;
        if !(w.creating || (w.wt.is_some() && w.state.is_host())) {
            return;
        }
        let hint = PlaivoExt {
            server: server.clone(),
            item: key.clone(),
        };
        if w.state.media.as_ref().and_then(Media::plaivo).as_ref() == Some(&hint) {
            return;
        }
        let (srv, downloads, c) = (
            self.server(&server),
            self.downloads.clone(),
            self.client.clone(),
        );
        let file = (server.clone(), key.clone());
        Self::bg(
            cx,
            move || {
                let ctx = Ctx::new(c, Source::Network);
                let fetch = |k: &str| {
                    downloads
                        .as_ref()
                        .and_then(|d| d.item(&server, k))
                        .filter(|i| !i.guids.is_empty())
                        .or_else(|| srv.as_ref()?.metadata(&ctx, k).ok())
                };
                let it = item.or_else(|| fetch(&key))?;
                let show = (it.kind == "episode" && !it.grandparent_rating_key.is_empty())
                    .then(|| fetch(&it.grandparent_rating_key))
                    .flatten();
                describe(&server, &it, show.as_ref()).ok()
            },
            move |this, media, cx| {
                if this.playing_file() != Some((file.0.as_str(), file.1.as_str())) {
                    return;
                }
                let Some(media) = media else {
                    return this.toast("This video can't be shared with the room.", cx);
                };
                let w = &mut this.watch;
                match &w.wt {
                    Some(wt) => wt.set_media(media, position_ms.max(0)),
                    None => w.share = Some((media, position_ms.max(0))),
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
            w.unavailable = false;
            w.file = None;
            match on_epoch(&s) {
                Bind::Resolve(media, epoch) => resolve = Some((*media, epoch)),
                Bind::Own(server, key) => w.file = Some((server, key)),
                Bind::Clear => {}
            }
        }
        if matches!(s.connection, Connection::Idle | Connection::Ended) {
            w.creating = false;
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
        Self::bg(
            cx,
            move || find_copy(&theirs, &servers, &c),
            move |this, found, cx| {
                let w = &mut this.watch;
                if w.epoch != Some(epoch) || !w.in_room() {
                    return;
                }
                let Some(wt) = &w.wt else { return };
                let Some((server, it)) = found else {
                    wt.set_match(
                        epoch,
                        Match {
                            state: MatchState::Unavailable,
                            delta_ms: None,
                        },
                    );
                    w.unavailable = true;
                    return this.toast(
                        format!(
                            "{} isn't in your libraries.",
                            media.series.as_ref().map_or(&media.title, |s| &s.title)
                        ),
                        cx,
                    );
                };
                wt.set_match(
                    epoch,
                    match_for(it.duration.map(|d| d as i64), media.duration_ms),
                );
                w.file = Some((server.clone(), it.rating_key.clone()));
                if this.playing_file() != Some((server.as_str(), it.rating_key.as_str())) {
                    this.play(server, it.rating_key, cx);
                }
            },
        );
    }

    fn watch_event(&mut self, event: Event, cx: &mut Context<Self>) {
        let text = event.text(&self.watch.state);
        let text = match event {
            Event::Failed(_) => {
                self.watch.error = text.clone().map(SharedString::from);
                text.filter(|_| self.page != Page::Watch)
            }
            Event::Ended { .. } => {
                self.watch.creating = false;
                text
            }
            _ => text,
        };
        if let Some(text) = text {
            if self.page == Page::Player {
                self.hint(Icon::Group, text, cx);
            } else {
                self.toast(text, cx);
            }
        }
    }

    fn playing_file(&self) -> Option<(&str, &str)> {
        let server = self.player.server.as_ref().filter(|_| self.playing())?;
        Some((server.id.as_str(), self.player.key.as_str()))
    }

    pub fn watch_open(&mut self, cx: &mut Context<Self>) {
        let Some((server, key)) = self.watch.file.clone() else {
            return;
        };
        if self.playing_file() == Some((server.as_str(), key.as_str())) {
            self.expand_player(cx);
        } else {
            self.play(server, key, cx);
        }
    }

    pub fn watch_holds(&self) -> bool {
        holds(
            &self.watch.state,
            self.watch.file.as_ref(),
            self.playing_file(),
        )
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
        let (Some(wt), Some(epoch)) = (&w.wt, w.epoch) else {
            return;
        };
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
    use plaivo_core::watch_together::{Control, MediaKind, Participant, Phase, PlayerState, Room};

    fn state(
        connection: Connection,
        host: bool,
        epoch: i64,
        media: Option<(&str, &str)>,
        m: Option<MatchState>,
    ) -> PartyState {
        let me = Participant {
            peer_id: "me".into(),
            name: "Me".into(),
            client: CLIENT.into(),
            host,
            connected: true,
            player: PlayerState::Ready,
            match_: None,
            offset_ms: 0,
        };
        let room = Room {
            rev: 1,
            code: "ABCDEF".into(),
            epoch,
            phase: Phase::Paused,
            position_ms: 0,
            at_ms: 0,
            rate: 1.0,
            control: Control::Host,
            locked: false,
            waiting_on: vec![],
            participants: vec![me],
        };
        let media = media.map(|(server, item)| {
            let mut media = Media {
                kind: MediaKind::Video,
                title: "x".into(),
                year: None,
                ids: Default::default(),
                series: None,
                season: None,
                episode: None,
                duration_ms: 1,
                file: None,
                ext: Default::default(),
            };
            media.set_plaivo(&PlaivoExt {
                server: server.into(),
                item: item.into(),
            });
            media
        });
        PartyState {
            connection,
            peer_id: Some("me".into()),
            room: Some(room),
            media,
            match_: m.map(|state| Match {
                state,
                delta_ms: None,
            }),
            ..Default::default()
        }
    }

    fn file(server: &str, key: &str) -> (String, String) {
        (server.into(), key.into())
    }

    #[test]
    fn a_new_epoch_binds_own_media_resolves_others_and_clears_otherwise() {
        let own = state(
            Connection::Connected,
            true,
            2,
            Some(("srv", "ep2")),
            Some(MatchState::Matched),
        );
        assert_eq!(on_epoch(&own), Bind::Own("srv".into(), "ep2".into()));
        let theirs = state(
            Connection::Connected,
            false,
            3,
            Some(("other", "9")),
            Some(MatchState::Searching),
        );
        assert!(matches!(on_epoch(&theirs), Bind::Resolve(_, 3)));
        assert_eq!(
            on_epoch(&state(Connection::Connected, true, 4, None, None)),
            Bind::Clear
        );
    }

    #[test]
    fn only_the_bound_file_of_a_matched_room_is_held() {
        let s = state(
            Connection::Connected,
            true,
            1,
            Some(("srv", "ep1")),
            Some(MatchState::Matched),
        );
        let ep1 = file("srv", "ep1");
        assert!(holds(&s, Some(&ep1), Some(("srv", "ep1"))));
        assert!(
            !holds(&s, Some(&ep1), Some(("srv", "ep2"))),
            "Up next's new episode must not feed the old epoch"
        );
        assert!(!holds(&s, Some(&ep1), Some(("other", "ep1"))));
        assert!(!holds(&s, Some(&ep1), None));
        assert!(!holds(&s, None, Some(("srv", "ep1"))));
        assert!(holds(
            &state(
                Connection::Reconnecting,
                true,
                1,
                Some(("srv", "ep1")),
                Some(MatchState::Mismatched)
            ),
            Some(&ep1),
            Some(("srv", "ep1"))
        ));
        assert!(!holds(
            &state(
                Connection::Connected,
                false,
                1,
                Some(("srv", "ep1")),
                Some(MatchState::Searching)
            ),
            Some(&ep1),
            Some(("srv", "ep1"))
        ));
        assert!(!holds(
            &state(
                Connection::Ended,
                true,
                1,
                Some(("srv", "ep1")),
                Some(MatchState::Matched)
            ),
            Some(&ep1),
            Some(("srv", "ep1"))
        ));
    }

    #[test]
    fn names_fall_back_and_stay_protocol_safe() {
        assert_eq!(pick_name("  Alex  ", || panic!("not asked"), None), "Alex");
        assert_eq!(
            pick_name("", || Some("plexuser".into()), Some("jf")),
            "plexuser"
        );
        assert_eq!(pick_name("\u{200E}", || None, Some("jf")), "jf");
        assert_eq!(pick_name("", || None, None), "Plaivo");
        assert_eq!(pick_name(&"x".repeat(40), || None, None).len(), NAME_MAX);
    }

    #[test]
    fn relay_setting_falls_back_to_public() {
        assert!(!relays("").custom);
        assert!(relays("https://relay.example.com").custom);
        assert!(!relays("ftp://nope").custom);
    }
}
