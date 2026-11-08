use super::protocol::{Phase, Room};

pub const TICK_MS: i64 = 500;
pub const SEEK_COOLDOWN_MS: i64 = 2000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayerCommand {
    Pause,
    SetRate(f64),
    Seek(i64),
    Play,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerFeed {
    pub position_ms: i64,
    pub duration_ms: Option<i64>,
    pub paused: bool,
    pub buffering: bool,
    pub loading: bool,
    pub can_set_rate: bool,
    pub seek_accuracy_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub actions: Vec<PlayerCommand>,
    pub nudging: bool,
    pub play_in_ms: Option<i64>,
}

pub struct StepIn<'a> {
    pub room: &'a Room,
    pub duration_ms: i64,
    pub relay_now: i64,
    pub offset_ms: i64,
    pub player: &'a PlayerFeed,
    pub applied_rate: f64,
    pub nudging: bool,
    pub since_seek_ms: Option<i64>,
}

pub fn round_rate(rate: f64) -> f64 {
    (rate * 1e6).round() / 1e6
}

pub fn hold_at(room: &Room, relay_now: i64, offset_ms: i64, player_duration: Option<i64>) -> Option<(i64, Option<i64>)> {
    let anchor = room.position_ms + offset_ms;
    let play_in = match room.phase {
        Phase::Idle => return None,
        Phase::Paused | Phase::Waiting => None,
        Phase::Playing => {
            let lead = if anchor < 0 { -anchor as f64 / room.rate } else { 0.0 };
            let start_at = room.at_ms as f64 + lead;
            if (relay_now as f64) >= start_at {
                return None;
            }
            Some((start_at - relay_now as f64).ceil() as i64)
        }
    };
    Some((clamp(anchor, end_of(player_duration)), play_in))
}

fn end_of(duration: Option<i64>) -> Option<i64> {
    duration.filter(|d| *d > 0).map(|d| (d - 1000).max(0))
}

fn clamp(x: i64, end: Option<i64>) -> i64 {
    end.map_or(x.max(0), |e| x.max(0).min(e))
}

pub fn step(s: &StepIn) -> Step {
    let (room, p) = (s.room, s.player);
    let mut out = Step::default();
    if room.phase == Phase::Idle {
        return out;
    }
    let accuracy = p.seek_accuracy_ms.max(0);
    let cooled = s.since_seek_ms.is_none_or(|t| t >= SEEK_COOLDOWN_MS);
    let end = end_of(p.duration_ms);
    let set_rate = |actions: &mut Vec<PlayerCommand>, want: f64| {
        if p.can_set_rate && (s.applied_rate - want).abs() > 0.001 {
            actions.push(PlayerCommand::SetRate(round_rate(want)));
        }
    };

    if let Some((hold, play_in)) = hold_at(room, s.relay_now, s.offset_ms, p.duration_ms) {
        if !p.paused {
            out.actions.push(PlayerCommand::Pause);
        }
        set_rate(&mut out.actions, room.rate);
        if (p.position_ms - hold).abs() > accuracy.max(500) && cooled {
            out.actions.push(PlayerCommand::Seek(hold));
        }
        out.play_in_ms = play_in;
        return out;
    }

    let target = (room.position_at(s.relay_now, s.duration_ms) + s.offset_ms).max(0);
    let mut nudging = s.nudging && p.can_set_rate;
    if let Some(e) = end
        && target >= e
        && p.position_ms >= e
    {
        set_rate(&mut out.actions, room.rate);
        return out;
    }
    if p.buffering {
        out.nudging = nudging;
        return out;
    }
    let drift = p.position_ms - target;
    let mut seek = false;
    if p.can_set_rate {
        if drift.abs() > accuracy.max(2000) {
            nudging = false;
            seek = cooled;
        } else if nudging {
            nudging = drift.abs() >= 150;
        } else if drift.abs() > 350 {
            nudging = true;
        }
    } else if drift.abs() > accuracy.max(1000) {
        seek = cooled;
    }
    let want = if nudging { room.rate * if drift > 0 { 0.96 } else { 1.04 } } else { room.rate };
    set_rate(&mut out.actions, want);
    if seek {
        out.actions.push(PlayerCommand::Seek(clamp(target + 250, end)));
    }
    if p.paused {
        out.actions.push(PlayerCommand::Play);
    }
    out.nudging = nudging;
    out
}

pub fn backoff_ms(attempt: u32, u: f64) -> i64 {
    let base = (500i64 << attempt.saturating_sub(1).min(5)).min(8000);
    (base as f64 * (0.5 + 0.5 * u)).floor() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch_together::protocol::tests::{test_room, vectors};
    use serde_json::{Value, json};

    #[test]
    fn reconcile_vectors() {
        let cases = vectors("reconcile");
        assert!(cases.len() > 70);
        for case in cases {
            let i = &case["input"];
            let pl = &i["player"];
            let player = PlayerFeed {
                position_ms: pl["positionMs"].as_i64().unwrap(),
                duration_ms: pl["durationMs"].as_i64(),
                paused: pl["paused"].as_bool().unwrap(),
                buffering: pl["buffering"].as_bool().unwrap(),
                loading: false,
                can_set_rate: pl["canSetRate"].as_bool().unwrap(),
                seek_accuracy_ms: pl["seekAccuracyMs"].as_i64().unwrap_or(0),
            };
            let room = test_room(&i["room"]);
            let got = step(&StepIn {
                room: &room,
                duration_ms: i["durationMs"].as_i64().unwrap(),
                relay_now: i["serverNowMs"].as_i64().unwrap(),
                offset_ms: i["offsetMs"].as_i64().unwrap(),
                player: &player,
                applied_rate: pl["rate"].as_f64().unwrap(),
                nudging: i["nudging"].as_bool().unwrap(),
                since_seek_ms: i["sinceSeekMs"].as_i64(),
            });
            let actions: Vec<Value> = got
                .actions
                .iter()
                .map(|a| match a {
                    PlayerCommand::Pause => json!({"do": "pause"}),
                    PlayerCommand::Play => json!({"do": "play"}),
                    PlayerCommand::Seek(p) => json!({"do": "seek", "positionMs": p}),
                    PlayerCommand::SetRate(r) => json!({"do": "setRate", "rate": r}),
                })
                .collect();
            let mut expect = case["expect"].clone();
            for a in expect["actions"].as_array_mut().unwrap() {
                if let Some(r) = a.get("rate").and_then(Value::as_f64) {
                    a["rate"] = json!(round_rate(r));
                }
            }
            assert_eq!(json!({"actions": actions, "nudging": got.nudging, "playInMs": got.play_in_ms}), expect, "{}", case["name"]);
        }
    }

    #[test]
    fn backoff_doubles_to_eight_seconds_with_jitter() {
        let full: Vec<i64> = (1..=7).map(|n| backoff_ms(n, 1.0 - f64::EPSILON)).collect();
        assert_eq!(full, [499, 999, 1999, 3999, 7999, 7999, 7999]);
        assert_eq!(backoff_ms(1, 0.0), 250);
        assert_eq!(backoff_ms(40, 0.0), 4000);
    }
}
