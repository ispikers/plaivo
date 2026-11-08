use super::clock::{Clock, Pong};
use super::follow::{PlayerCommand, PlayerFeed};
use super::media::Media;
use super::party::{Connection, Event, Output, Party, PartyError, PartyState, Request};
use super::protocol::{ClientMsg, Control, Match, ServerMsg, parse};
use super::relays::{Relay, Relays, fetch_list};
use std::hash::{BuildHasher, RandomState};
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::protocol::{CloseFrame, WebSocketConfig};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

type Socket = WebSocket<MaybeTlsStream<TcpStream>>;

const POLL: Duration = Duration::from_millis(50);
const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
const REFRESH_MS: i64 = 24 * 3600 * 1000;
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
const MAX_MESSAGE_BYTES: usize = 64 << 10;

type Opener = Box<dyn FnMut(String, u64, bool) + Send>;

#[derive(Clone, Debug, PartialEq)]
pub enum Update {
    State(Box<PartyState>),
    Event(Event),
    Player(PlayerCommand),
}

enum Cmd {
    Create,
    Join(String, bool),
    Leave,
    Feed(i64, PlayerFeed),
    Detach,
    Match(i64, Match),
    Offset(i64),
    Play,
    Pause,
    Seek(i64),
    Rate(f64),
    SetMedia(Box<Media>, i64),
    ClearMedia,
    Control(Control),
    TransferHost(String),
    Kick(String),
    Lock(bool),
    End,
    Opened(u64, String, Box<Socket>, Option<Clock>),
    OpenFailed(u64),
    List(Option<Vec<Relay>>),
    Shutdown,
}

pub struct WatchTogether {
    tx: Sender<Cmd>,
    alive: Arc<AtomicBool>,
    done: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

impl WatchTogether {
    pub fn new(name: &str, client: &str, relays: Relays, mut notify: impl FnMut(Update) + Send + 'static) -> Result<Self, PartyError> {
        let party = Party::new(name, client, RandomState::new().hash_one(0u8))?;
        let alive = Arc::new(AtomicBool::new(true));
        let gate = alive.clone();
        let notify = move |u| {
            if gate.load(Ordering::Acquire) {
                notify(u)
            }
        };
        let base = Instant::now();
        let (tx, rx) = mpsc::channel();
        let opener = tx.clone();
        let open = move |url, generation, race| spawn_open(url, generation, race, base, opener.clone());
        let io = Io::new(party, relays, rx, tx.clone(), base, Box::new(notify), Box::new(open), fetch_list);
        let (done_tx, done) = mpsc::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("watch-together".into())
            .spawn(move || {
                io.run();
                drop(done_tx);
            })
            .map_err(|_| PartyError::local("unreachable"))?;
        Ok(Self { tx, alive, done, thread: Some(thread) })
    }

    fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    pub fn create(&self) {
        self.send(Cmd::Create)
    }

    pub fn join(&self, code: &str, create: bool) {
        self.send(Cmd::Join(code.into(), create))
    }

    pub fn leave(&self) {
        self.send(Cmd::Leave)
    }

    pub fn update_player(&self, epoch: i64, feed: PlayerFeed) {
        self.send(Cmd::Feed(epoch, feed))
    }

    pub fn detach_player(&self) {
        self.send(Cmd::Detach)
    }

    pub fn set_match(&self, epoch: i64, m: Match) {
        self.send(Cmd::Match(epoch, m))
    }

    pub fn set_offset(&self, offset_ms: i64) {
        self.send(Cmd::Offset(offset_ms))
    }

    pub fn play(&self) {
        self.send(Cmd::Play)
    }

    pub fn pause(&self) {
        self.send(Cmd::Pause)
    }

    pub fn seek(&self, position_ms: i64) {
        self.send(Cmd::Seek(position_ms))
    }

    pub fn set_rate(&self, rate: f64) {
        self.send(Cmd::Rate(rate))
    }

    pub fn set_media(&self, media: Media, position_ms: i64) {
        self.send(Cmd::SetMedia(Box::new(media), position_ms))
    }

    pub fn clear_media(&self) {
        self.send(Cmd::ClearMedia)
    }

    pub fn set_control(&self, mode: Control) {
        self.send(Cmd::Control(mode))
    }

    pub fn transfer_host(&self, peer_id: &str) {
        self.send(Cmd::TransferHost(peer_id.into()))
    }

    pub fn kick(&self, peer_id: &str) {
        self.send(Cmd::Kick(peer_id.into()))
    }

    pub fn lock(&self, locked: bool) {
        self.send(Cmd::Lock(locked))
    }

    pub fn end(&self) {
        self.send(Cmd::End)
    }
}

impl Drop for WatchTogether {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
        self.send(Cmd::Shutdown);
        if self.done.recv_timeout(SHUTDOWN_WAIT) == Err(RecvTimeoutError::Disconnected)
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

struct Io {
    party: Party,
    relays: Relays,
    rx: Receiver<Cmd>,
    tx: Sender<Cmd>,
    notify: Box<dyn FnMut(Update) + Send>,
    open: Opener,
    fetch: fn() -> Option<Vec<Relay>>,
    base: Instant,
    ws: Option<Box<Socket>>,
    generation: u64,
    pending: usize,
    refreshed: bool,
    refreshing: bool,
    last_refresh: Option<i64>,
    published: PartyState,
}

impl Io {
    #[allow(clippy::too_many_arguments)]
    fn new(party: Party, relays: Relays, rx: Receiver<Cmd>, tx: Sender<Cmd>, base: Instant, notify: Box<dyn FnMut(Update) + Send>, open: Opener, fetch: fn() -> Option<Vec<Relay>>) -> Self {
        Self {
            party,
            relays,
            rx,
            tx,
            notify,
            open,
            fetch,
            base,
            ws: None,
            generation: 0,
            pending: 0,
            refreshed: false,
            refreshing: false,
            last_refresh: None,
            published: PartyState::default(),
        }
    }

    fn now(&self) -> i64 {
        self.base.elapsed().as_millis() as i64
    }

    fn run(mut self) {
        loop {
            if let Some(ws) = self.ws.as_mut() {
                let read = ws.read();
                let now = self.now();
                match read {
                    Ok(Message::Text(text)) => self.party.on_text(now, text.as_str()),
                    Ok(Message::Close(frame)) => {
                        self.ws = None;
                        self.party.on_drop(now, frame.map(|f| f.code.into()));
                    }
                    Ok(_) => {}
                    Err(e) if would_block(&e) => {}
                    Err(_) => {
                        self.ws = None;
                        self.party.on_drop(now, None);
                    }
                }
            }
            let first = match (self.ws.is_some(), self.party.next_deadline()) {
                (true, _) => self.rx.try_recv().ok(),
                (false, Some(t)) => self.rx.recv_timeout(Duration::from_millis((t - self.now()).max(0) as u64)).ok(),
                (false, None) => self.rx.recv().ok(),
            };
            let mut cmds: Vec<Cmd> = first.into_iter().collect();
            cmds.extend(self.rx.try_iter());
            for cmd in cmds {
                if !self.handle(cmd) {
                    self.flush();
                    return;
                }
            }
            let now = self.now();
            self.party.tick(now);
            self.flush();
        }
    }

    fn handle(&mut self, cmd: Cmd) -> bool {
        let now = self.now();
        let result = match cmd {
            Cmd::Create => self.party.start(Request::Create, now).map(|_| {
                let urls: Vec<String> = self.relays.list.iter().map(|r| r.url.clone()).collect();
                let race = urls.len() > 1;
                self.connect(urls, race);
            }),
            Cmd::Join(code, create) => self.party.start(Request::Join { code, create }, now).map(|_| {
                self.refreshed = false;
                self.route_join();
            }),
            Cmd::Leave => {
                self.party.leave();
                Ok(())
            }
            Cmd::Feed(epoch, feed) => {
                self.party.set_player(epoch, feed, now);
                Ok(())
            }
            Cmd::Detach => {
                self.party.detach_player();
                Ok(())
            }
            Cmd::Match(epoch, m) => {
                self.party.set_match(epoch, m);
                Ok(())
            }
            Cmd::Offset(ms) => self.party.set_offset(ms),
            Cmd::Play => self.party.play(),
            Cmd::Pause => self.party.pause(now),
            Cmd::Seek(ms) => self.party.seek(ms, now),
            Cmd::Rate(rate) => self.party.set_rate(rate, now),
            Cmd::SetMedia(media, position_ms) => self.party.set_media(*media, position_ms),
            Cmd::ClearMedia => self.party.clear_media(),
            Cmd::Control(mode) => self.party.set_control(mode),
            Cmd::TransferHost(peer) => self.party.transfer_host(&peer),
            Cmd::Kick(peer) => self.party.kick(&peer),
            Cmd::Lock(locked) => self.party.lock(locked),
            Cmd::End => self.party.end_room(),
            Cmd::Opened(generation, url, mut ws, clock) => {
                let wanted = generation == self.generation && self.ws.is_none();
                if generation == self.generation {
                    self.pending = self.pending.saturating_sub(1);
                }
                if wanted {
                    self.generation += 1;
                    self.pending = 0;
                    self.ws = Some(ws);
                    self.party.on_open(&url, now, clock);
                } else {
                    close(&mut ws, 1000);
                }
                Ok(())
            }
            Cmd::OpenFailed(generation) => {
                if generation == self.generation {
                    self.pending = self.pending.saturating_sub(1);
                    if self.pending == 0 && self.ws.is_none() {
                        self.party.on_connect_failed(now);
                    }
                }
                Ok(())
            }
            Cmd::List(list) => {
                self.refreshing = false;
                if let Some(list) = list {
                    self.relays.list = list;
                    self.last_refresh = Some(now);
                }
                Ok(())
            }
            Cmd::Shutdown => {
                self.party.leave();
                return false;
            }
        };
        if let Err(e) = result {
            let event = if matches!(e.code.as_str(), "invalid_code" | "already_in_room") { Event::Failed(e) } else { Event::Error(e) };
            (self.notify)(Update::Event(event));
        }
        true
    }

    fn flush(&mut self) {
        loop {
            let outputs = self.party.take_outputs();
            if outputs.is_empty() {
                break;
            }
            for output in outputs {
                match output {
                    Output::Send(text) => {
                        let Some(ws) = self.ws.as_mut() else { continue };
                        if let Err(e) = ws.send(Message::text(text))
                            && !would_block(&e)
                        {
                            self.ws = None;
                            let now = self.now();
                            self.party.on_drop(now, None);
                        }
                    }
                    Output::Close(code) => {
                        if let Some(mut ws) = self.ws.take() {
                            close(&mut ws, code);
                        }
                    }
                    Output::Connect(url) => self.connect(vec![url], false),
                    Output::Player(command) => (self.notify)(Update::Player(command)),
                    Output::Refused(err) => self.refused(err),
                }
            }
        }
        for event in self.party.take_events() {
            (self.notify)(Update::Event(event));
        }
        if self.party.state() != &self.published {
            let joined = self.party.state().connection == Connection::Connected && self.published.connection != Connection::Connected;
            self.published = self.party.state().clone();
            (self.notify)(Update::State(Box::new(self.published.clone())));
            if joined {
                self.background_refresh();
            }
        }
    }

    fn connect(&mut self, urls: Vec<String>, race: bool) {
        self.generation += 1;
        self.pending = urls.len();
        if urls.is_empty() {
            let now = self.now();
            self.party.on_connect_failed(now);
        }
        for url in urls {
            (self.open)(url, self.generation, race);
        }
    }

    fn route_join(&mut self) {
        let Some(Request::Join { code, .. }) = self.party.request().cloned() else { return };
        if self.relays.relay_for(&code).is_none() {
            self.refresh_now();
        }
        match self.relays.relay_for(&code).map(|r| r.url.clone()) {
            Some(url) => self.connect(vec![url], false),
            None => self.party.fail(PartyError::local("room_not_found")),
        }
    }

    fn refused(&mut self, err: PartyError) {
        let next = match (self.party.request().cloned(), err.code.as_str()) {
            (Some(Request::Create), "relay_full") => self.relays.list.iter().map(|r| r.url.clone()).find(|u| !self.party.tried().contains(u)),
            (Some(Request::Join { code, .. }), "room_not_found") if !self.refreshed => {
                self.refresh_now();
                self.relays.relay_for(&code).map(|r| r.url.clone()).filter(|u| !self.party.tried().contains(u))
            }
            _ => None,
        };
        match next {
            Some(url) => {
                let now = self.now();
                self.party.retry(now);
                self.connect(vec![url], false);
            }
            None => self.party.fail(err),
        }
    }

    fn refresh_now(&mut self) {
        if self.relays.custom || self.refreshed {
            return;
        }
        self.refreshed = true;
        if let Some(list) = (self.fetch)() {
            self.relays.list = list;
            self.last_refresh = Some(self.now());
        }
    }

    fn background_refresh(&mut self) {
        let now = self.now();
        if self.relays.custom || self.refreshing || self.last_refresh.is_some_and(|t| now - t < REFRESH_MS) {
            return;
        }
        self.refreshing = true;
        let (tx, fetch) = (self.tx.clone(), self.fetch);
        std::thread::spawn(move || {
            let _ = tx.send(Cmd::List(fetch()));
        });
    }
}

fn spawn_open(url: String, generation: u64, race: bool, base: Instant, tx: Sender<Cmd>) {
    std::thread::spawn(move || {
        let deadline = base.elapsed().as_millis() as i64 + OPEN_TIMEOUT.as_millis() as i64;
        let opened = open(&url).and_then(|mut ws| {
            let clock = if race { Some(first_sample(&mut ws, base, deadline)?) } else { None };
            Some((ws, clock))
        });
        let _ = tx.send(match opened {
            Some((ws, clock)) => Cmd::Opened(generation, url, Box::new(ws), clock),
            None => Cmd::OpenFailed(generation),
        });
    });
}

fn close(ws: &mut Socket, code: u16) {
    let _ = ws.close(Some(CloseFrame { code: code.into(), reason: "".into() }));
    let _ = ws.flush();
}

fn would_block(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

fn open(url: &str) -> Option<Socket> {
    let authority = url.split_once("://")?.1.split('/').next()?;
    let has_port = authority.rfind(':').is_some_and(|i| !authority[i..].contains(']'));
    let port = if url.starts_with("wss://") { 443 } else { 80 };
    let address = if has_port { authority.to_owned() } else { format!("{authority}:{port}") };
    let deadline = Instant::now() + OPEN_TIMEOUT;
    let remaining = || deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero());
    let tcp = address.to_socket_addrs().ok()?.find_map(|a| TcpStream::connect_timeout(&a, remaining()?).ok())?;
    tcp.set_read_timeout(Some(remaining()?)).ok()?;
    tcp.set_write_timeout(Some(OPEN_TIMEOUT)).ok()?;
    let _ = tcp.set_nodelay(true);
    let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE_BYTES)).max_frame_size(Some(MAX_MESSAGE_BYTES));
    let (mut ws, _) = tungstenite::client_tls_with_config(url, tcp, Some(config), None).ok()?;
    let tcp = match ws.get_mut() {
        MaybeTlsStream::Plain(s) => s,
        #[cfg(not(target_os = "android"))]
        MaybeTlsStream::NativeTls(s) => s.get_mut(),
        #[cfg(target_os = "android")]
        MaybeTlsStream::Rustls(s) => &mut s.sock,
        _ => return None,
    };
    tcp.set_read_timeout(Some(POLL)).ok()?;
    Some(ws)
}

fn first_sample(ws: &mut Socket, base: Instant, deadline: i64) -> Option<Clock> {
    let now = || base.elapsed().as_millis() as i64;
    let mut clock = Clock::new(now());
    while now() < deadline {
        if let Some(id) = clock.due_ping(now()) {
            ws.send(Message::text(ClientMsg::Ping { id: id.into() }.to_json())).ok()?;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if let ServerMsg::Pong { id, server_time } = parse(text.as_str())
                    && let Some(id) = id.as_u64()
                    && clock.on_pong(id, server_time, now()) != Pong::Ignored
                {
                    return Some(clock);
                }
            }
            Ok(_) => {}
            Err(e) if would_block(&e) => {}
            Err(_) => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::Mutex;
    use tungstenite::protocol::Role;

    fn relay(id: &str, prefixes: &str) -> Relay {
        Relay { id: id.into(), name: id.into(), url: url(id), prefixes: prefixes.into() }
    }

    fn url(id: &str) -> String {
        format!("ws://{id}/v1/ws")
    }

    fn moved() -> Option<Vec<Relay>> {
        Some(vec![relay("a", "2345678"), relay("b", "ABCDEF"), relay("c", "9GHJKMNP"), relay("d", "Q")])
    }

    fn offline() -> Option<Vec<Relay>> {
        None
    }

    struct Harness {
        io: Io,
        opens: Arc<Mutex<Vec<(String, bool)>>>,
        updates: Arc<Mutex<Vec<Update>>>,
        servers: Vec<TcpStream>,
    }

    impl Harness {
        fn new(fetch: fn() -> Option<Vec<Relay>>) -> Self {
            let relays = Relays { list: vec![relay("a", "2345678"), relay("b", "9ABCDEF"), relay("c", "GHJKMNP")], custom: false };
            let (tx, rx) = mpsc::channel();
            let opens = Arc::new(Mutex::new(Vec::new()));
            let updates = Arc::new(Mutex::new(Vec::new()));
            let (o, u) = (opens.clone(), updates.clone());
            let party = Party::new("Tester", "Plaivo/0.1.0", 1).unwrap();
            let open: Opener = Box::new(move |url, _, race| o.lock().unwrap().push((url, race)));
            let notify = Box::new(move |update| u.lock().unwrap().push(update));
            Self { io: Io::new(party, relays, rx, tx, Instant::now(), notify, open, fetch), opens, updates, servers: Vec::new() }
        }

        fn cmd(&mut self, cmd: Cmd) {
            assert!(self.io.handle(cmd));
            self.io.flush();
        }

        fn opens(&self) -> Vec<(String, bool)> {
            std::mem::take(&mut *self.opens.lock().unwrap())
        }

        fn accept(&mut self, id: &str) {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let tcp = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            self.servers.push(listener.accept().unwrap().0);
            let ws = WebSocket::from_raw_socket(MaybeTlsStream::Plain(tcp), Role::Client, None);
            let generation = self.io.generation;
            self.cmd(Cmd::Opened(generation, url(id), Box::new(ws), None));
        }

        fn refuse(&mut self, code: &str, re: &str) {
            let now = self.io.now();
            self.io.party.on_text(now, &format!(r#"{{"type":"error","code":"{code}","message":"","re":"{re}"}}"#));
            self.io.flush();
        }

        fn failure(&self) -> Option<String> {
            self.updates.lock().unwrap().iter().rev().find_map(|u| match u {
                Update::Event(Event::Failed(e)) => Some(e.code.clone()),
                _ => None,
            })
        }
    }

    #[test]
    fn create_races_every_relay_and_falls_back_on_relay_full() {
        let mut h = Harness::new(moved);
        h.cmd(Cmd::Create);
        assert_eq!(h.opens(), [(url("a"), true), (url("b"), true), (url("c"), true)]);
        assert_eq!(h.io.pending, 3);
        h.accept("a");
        assert_eq!(h.io.pending, 0);
        h.accept("b");
        assert!(h.opens().is_empty());

        h.refuse("relay_full", "create");
        assert_eq!(h.opens(), [(url("b"), false)]);
        h.accept("b");
        h.refuse("relay_full", "create");
        assert_eq!(h.opens(), [(url("c"), false)]);
        h.accept("c");
        h.refuse("relay_full", "create");
        assert!(h.opens().is_empty());
        assert_eq!(h.failure().as_deref(), Some("relay_full"));
        assert_eq!(h.io.party.state().connection, Connection::Idle);

        h.cmd(Cmd::Join("9ABCDE".into(), false));
        assert_eq!(h.opens(), [(url("b"), false)]);
        h.accept("b");
        h.refuse("room_not_found", "join");
        assert_eq!(h.opens(), [(url("c"), false)], "a relay tried by the earlier create stays usable");
        h.accept("c");
        h.refuse("room_not_found", "join");
        assert!(h.opens().is_empty());
        assert_eq!(h.failure().as_deref(), Some("room_not_found"));
    }

    #[test]
    fn join_refreshes_the_list_for_an_unowned_symbol() {
        let mut h = Harness::new(moved);
        h.cmd(Cmd::Join("q-abcde".into(), false));
        assert_eq!(h.opens(), [(url("d"), false)]);

        let mut h = Harness::new(offline);
        h.cmd(Cmd::Join("QABCDE".into(), false));
        assert!(h.opens().is_empty());
        assert_eq!(h.failure().as_deref(), Some("room_not_found"));

        let mut h = Harness::new(offline);
        h.cmd(Cmd::Join("2ABCDE".into(), false));
        assert_eq!(h.opens(), [(url("a"), false)]);
        h.accept("a");
        h.refuse("room_not_found", "join");
        assert!(h.opens().is_empty());
        assert_eq!(h.failure().as_deref(), Some("room_not_found"));
    }

    #[test]
    fn stale_open_results_are_ignored_and_the_last_failure_fails_create() {
        let mut h = Harness::new(offline);
        h.cmd(Cmd::Create);
        let generation = h.io.generation;
        h.cmd(Cmd::OpenFailed(generation - 1));
        h.cmd(Cmd::OpenFailed(generation));
        h.cmd(Cmd::OpenFailed(generation));
        assert_eq!(h.failure(), None);
        h.cmd(Cmd::OpenFailed(generation));
        assert_eq!(h.failure().as_deref(), Some("unreachable"));
        h.cmd(Cmd::Join("2ABCDE".into(), false));
        let generation = h.io.generation;
        h.cmd(Cmd::OpenFailed(generation - 1));
        assert_eq!(h.io.pending, 1);
    }
}
