use std::collections::VecDeque;

const WINDOW: usize = 8;
const MAX_RTT_MS: i64 = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sample {
    pub offset_ms: i64,
    pub rtt_ms: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pong {
    Accepted,
    Stepped,
    Ignored,
}

#[derive(Clone, Debug)]
pub struct Clock {
    opened_at: i64,
    next_id: u64,
    sent: Vec<(u64, i64)>,
    window: VecDeque<Sample>,
}

impl Clock {
    pub fn new(opened_at: i64) -> Self {
        Self {
            opened_at,
            next_id: 1,
            sent: Vec::new(),
            window: VecDeque::with_capacity(WINDOW + 1),
        }
    }

    fn ping_at(&self, id: u64) -> i64 {
        let n = id as i64;
        self.opened_at
            + if n <= 3 {
                (n - 1) * 500
            } else {
                (n - 3) * 5000
            }
    }

    pub fn next_ping_at(&self) -> i64 {
        self.ping_at(self.next_id)
    }

    pub fn due_ping(&mut self, now: i64) -> Option<u64> {
        if self.next_ping_at() > now {
            return None;
        }
        while self.ping_at(self.next_id + 1) <= now {
            self.next_id += 1;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.sent(id, now);
        Some(id)
    }

    pub fn sent(&mut self, id: u64, at: i64) {
        self.sent.retain(|&(_, t)| at - t <= MAX_RTT_MS);
        self.sent.push((id, at));
    }

    pub fn on_pong(&mut self, id: u64, server_time: i64, now: i64) -> Pong {
        let Some(i) = self.sent.iter().position(|&(sid, _)| sid == id) else {
            return Pong::Ignored;
        };
        let (_, sent_at) = self.sent.swap_remove(i);
        let rtt = now - sent_at;
        if !(0..=MAX_RTT_MS).contains(&rtt) {
            return Pong::Ignored;
        }
        let sample = Sample {
            offset_ms: server_time - (sent_at + rtt / 2),
            rtt_ms: rtt,
        };
        let stepped = self
            .best()
            .is_some_and(|b| (sample.offset_ms - b.offset_ms).abs() > (rtt + b.rtt_ms) / 2 + 100);
        if stepped {
            self.window.clear();
        }
        self.window.push_back(sample);
        if self.window.len() > WINDOW {
            self.window.pop_front();
        }
        if stepped {
            Pong::Stepped
        } else {
            Pong::Accepted
        }
    }

    pub fn best(&self) -> Option<Sample> {
        self.window
            .iter()
            .copied()
            .reduce(|best, s| if s.rtt_ms < best.rtt_ms { s } else { best })
    }

    pub fn relay_now(&self, now: i64) -> Option<i64> {
        self.best().map(|b| now + b.offset_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch_together::protocol::tests::vectors;
    use serde_json::{Value, json};

    #[test]
    fn clock_sync_vectors() {
        let cases = vectors("clock-sync");
        assert!(!cases.is_empty());
        for case in cases {
            let mut clock = Clock::new(0);
            let mut got = Vec::new();
            for e in case["input"]["events"].as_array().unwrap() {
                let at = e["atMs"].as_i64().unwrap();
                if let Some(id) = e.get("ping") {
                    clock.sent(id.as_u64().unwrap(), at);
                    continue;
                }
                let result = match clock.on_pong(
                    e["pong"].as_u64().unwrap(),
                    e["serverTime"].as_i64().unwrap(),
                    at,
                ) {
                    Pong::Accepted => "accepted",
                    Pong::Stepped => "stepped",
                    Pong::Ignored => "ignored",
                };
                let best = clock.best();
                got.push(json!({"result": result, "offsetMs": best.map(|b| b.offset_ms), "rttMs": best.map(|b| b.rtt_ms)}));
            }
            assert_eq!(
                Value::from(got),
                case["expect"]["pongs"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn pings_burst_then_every_five_seconds() {
        let mut clock = Clock::new(1000);
        let mut sent = Vec::new();
        for now in (1000..=21000).step_by(50) {
            if let Some(id) = clock.due_ping(now) {
                sent.push((id, now));
            }
        }
        assert_eq!(
            sent,
            [
                (1, 1000),
                (2, 1500),
                (3, 2000),
                (4, 6000),
                (5, 11000),
                (6, 16000),
                (7, 21000)
            ]
        );
    }

    #[test]
    fn late_tick_skips_missed_pings() {
        let mut clock = Clock::new(0);
        assert_eq!(clock.due_ping(12_000), Some(5));
        assert_eq!(clock.due_ping(12_100), None);
        assert_eq!(clock.next_ping_at(), 15_000);
    }

    #[test]
    fn offset_comes_from_lowest_rtt_of_last_eight() {
        let mut clock = Clock::new(0);
        clock.sent(1, 0);
        clock.on_pong(1, 10_010, 20);
        for id in 2..=9 {
            let at = id as i64 * 1000;
            clock.sent(id, at);
            clock.on_pong(id, at + 10_000 + 30, at + 60);
        }
        assert_eq!(
            clock.best(),
            Some(Sample {
                offset_ms: 10_000,
                rtt_ms: 60
            })
        );
        assert_eq!(clock.relay_now(5), Some(10_005));
    }
}
