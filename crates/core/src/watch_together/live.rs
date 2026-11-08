use super::*;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const DURATION: i64 = 3_487_000;

struct Sim {
    base: f64,
    since: Instant,
    paused: bool,
    rate: f64,
    play_blocked_until: Option<Instant>,
}

impl Sim {
    fn position(&self) -> i64 {
        let moving = if self.paused { 0.0 } else { self.since.elapsed().as_millis() as f64 * self.rate };
        (self.base + moving) as i64
    }

    fn rebase(&mut self) {
        self.base = self.position() as f64;
        self.since = Instant::now();
    }

    fn apply(&mut self, c: PlayerCommand) {
        self.rebase();
        match c {
            PlayerCommand::Pause => self.paused = true,
            PlayerCommand::Play => {
                let blocked = self.play_blocked_until.get_or_insert_with(|| Instant::now() + Duration::from_millis(2500));
                if Instant::now() >= *blocked {
                    self.paused = false;
                }
            }
            PlayerCommand::Seek(p) => self.base = p as f64,
            PlayerCommand::SetRate(r) => self.rate = r,
        }
    }
}

struct Peer {
    wt: WatchTogether,
    rx: Receiver<Update>,
    state: PartyState,
    sim: Sim,
    attached: bool,
    log: Vec<String>,
}

impl Peer {
    fn new(name: &str, late_start: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        let wt = WatchTogether::new(name, "Plaivo/0.1.0-test", Relays::public(), move |u| {
            let _ = tx.send(u);
        })
        .unwrap();
        let blocked = (!late_start).then(Instant::now);
        let sim = Sim { base: 0.0, since: Instant::now(), paused: true, rate: 1.0, play_blocked_until: blocked };
        Self { wt, rx, state: PartyState::default(), sim, attached: false, log: Vec::new() }
    }

    fn pump(&mut self) {
        for u in self.rx.try_iter() {
            match u {
                Update::State(s) => self.state = *s,
                Update::Player(c) => {
                    self.log.push(format!("{c:?}"));
                    self.sim.apply(c);
                }
                Update::Event(e) => self.log.push(format!("{e:?}")),
            }
        }
        if self.attached
            && let Some(room) = &self.state.room
        {
            self.wt.update_player(room.epoch, PlayerFeed {
                position_ms: self.sim.position(),
                duration_ms: Some(DURATION),
                paused: self.sim.paused,
                buffering: false,
                loading: false,
                can_set_rate: true,
                seek_accuracy_ms: 0,
            });
        }
    }

    fn phase(&self) -> Option<Phase> {
        self.state.room.as_ref().map(|r| r.phase)
    }
}

fn wait(peers: &mut [&mut Peer], what: &str, timeout_s: u64, done: impl Fn(&[&mut Peer]) -> bool) -> Duration {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(timeout_s) {
        for p in peers.iter_mut() {
            p.pump();
        }
        if done(peers) {
            println!("{what}: {} ms", start.elapsed().as_millis());
            return start.elapsed();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for p in peers.iter() {
        println!("{:?}\n{:?}\n{}", p.state.room, p.sim.position(), p.log.join("\n"));
    }
    panic!("timed out waiting for {what}");
}

fn soak(peers: &mut [&mut Peer], ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        peers.iter_mut().for_each(|p| p.pump());
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore]
fn two_clients_sync_on_the_public_relay() {
    let started = Instant::now();
    assert_eq!(super::relays::fetch_list().unwrap(), Relays::public().list);
    let mut a = Peer::new("Alice", false);
    let mut b = Peer::new("Bob", true);
    a.wt.create();
    wait(&mut [&mut a], "create", 20, |p| p[0].state.connection == Connection::Connected);
    let code = a.state.room.as_ref().unwrap().code.clone();
    println!("room {} on {}", format_code(&code).unwrap(), Relays::public().relay_for(&code).unwrap().id);

    b.wt.join(&code.to_lowercase(), false);
    wait(&mut [&mut a, &mut b], "join", 20, |p| p[1].state.connection == Connection::Connected && p[0].state.room.as_ref().is_some_and(|r| r.participants.len() == 2));

    let mut media = Media {
        kind: MediaKind::Movie,
        title: "Arrival".into(),
        year: Some(2016),
        ids: ids_from_guids(["imdb://tt2543164", "tmdb://329865"]),
        series: None,
        season: None,
        episode: None,
        duration_ms: DURATION,
        file: None,
        ext: Default::default(),
    };
    media.set_plaivo(&PlaivoExt { server: "test".into(), item: "1".into() });
    a.wt.set_media(media.clone(), 0);
    a.attached = true;
    wait(&mut [&mut a, &mut b], "media", 20, |p| p[1].state.media.is_some());
    assert_eq!(b.state.media.as_ref(), Some(&media));
    assert_eq!(a.state.match_.unwrap().state, MatchState::Matched);
    b.wt.set_match(b.state.room.as_ref().unwrap().epoch, Match { state: MatchState::Matched, delta_ms: Some(0) });
    b.attached = true;

    wait(&mut [&mut a, &mut b], "playing", 40, |p| p.iter().all(|p| p.phase() == Some(Phase::Playing) && !p.sim.paused));
    let converge = wait(&mut [&mut a, &mut b], "converge after late start", 20, |p| (p[0].sim.position() - p[1].sim.position()).abs() <= 350);
    soak(&mut [&mut a, &mut b], 4000);
    let drift = (a.sim.position() - b.sim.position()).abs();
    println!("playing drift after soak: {drift} ms (positions {} / {})", a.sim.position(), b.sim.position());
    assert!(drift <= 350, "drift {drift}");
    assert!(b.log.iter().any(|l| l.starts_with("Seek")), "late start caught up by seek: {:?}", b.log);
    println!("converged {} ms after both were playing", converge.as_millis());

    a.wt.pause();
    wait(&mut [&mut a, &mut b], "pause", 10, |p| {
        p.iter().all(|p| p.phase() == Some(Phase::Paused) && p.sim.paused && (p.sim.position() - p.state.room.as_ref().unwrap().position_ms).abs() <= 500)
    });
    let paused_at = a.state.room.as_ref().unwrap().position_ms;
    println!("paused at {paused_at}: {} / {}", a.sim.position(), b.sim.position());

    a.wt.seek(1_800_000);
    wait(&mut [&mut a, &mut b], "seek", 10, |p| {
        p.iter().all(|p| p.state.room.as_ref().unwrap().position_ms == 1_800_000 && p.sim.paused && (p.sim.position() - 1_800_000).abs() <= 500)
    });

    a.wt.play();
    wait(&mut [&mut a, &mut b], "play again", 20, |p| p.iter().all(|p| p.phase() == Some(Phase::Playing) && !p.sim.paused));
    soak(&mut [&mut a, &mut b], 3000);
    let drift = (a.sim.position() - b.sim.position()).abs();
    println!("drift after play: {drift} ms (positions {} / {})", a.sim.position(), b.sim.position());
    assert!(drift <= 350, "drift {drift}");
    assert!(a.sim.position() > 1_800_000);

    a.wt.leave();
    b.wt.leave();
    wait(&mut [&mut a, &mut b], "leave", 10, |p| p.iter().all(|p| p.state.end_reason == Some(EndReason::Left)));
    println!("total {} ms", started.elapsed().as_millis());
}
