use super::media::Media;
use super::relays::valid_code;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashSet;

pub const MAX_INT: i64 = (1 << 53) - 1;
pub const MAX_DURATION_MS: i64 = 604_800_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    #[default]
    Idle,
    Paused,
    Waiting,
    Playing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Control {
    #[default]
    Host,
    Everyone,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayerState {
    #[default]
    Idle,
    Loading,
    Ready,
    Buffering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchState {
    Searching,
    Matched,
    Mismatched,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub state: MatchState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub peer_id: String,
    pub name: String,
    pub client: String,
    pub host: bool,
    pub connected: bool,
    pub player: PlayerState,
    #[serde(rename = "match", deserialize_with = "present")]
    pub match_: Option<Match>,
    pub offset_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room {
    pub rev: i64,
    pub code: String,
    pub epoch: i64,
    pub phase: Phase,
    pub position_ms: i64,
    pub at_ms: i64,
    pub rate: f64,
    pub control: Control,
    pub locked: bool,
    pub waiting_on: Vec<String>,
    pub participants: Vec<Participant>,
}

impl Room {
    pub fn position_at(&self, relay_now: i64, duration_ms: i64) -> i64 {
        if self.phase != Phase::Playing {
            return self.position_ms;
        }
        let elapsed = (relay_now - self.at_ms).max(0) as f64 * self.rate;
        ((self.position_ms as f64 + elapsed).floor() as i64).min(duration_ms)
    }

    pub fn running(&self, relay_now: i64) -> bool {
        self.phase == Phase::Playing && relay_now >= self.at_ms
    }

    pub fn participant(&self, peer_id: &str) -> Option<&Participant> {
        self.participants.iter().find(|p| p.peer_id == peer_id)
    }

    fn valid(&self) -> bool {
        let int = |v: i64| (0..=MAX_INT).contains(&v);
        let mut seen = HashSet::with_capacity(self.waiting_on.len());
        let waiting = (self.phase == Phase::Waiting || self.waiting_on.is_empty())
            && self
                .waiting_on
                .iter()
                .all(|p| valid_peer_id(p) && seen.insert(p.as_str()));
        let participants = self.participants.iter().filter(|p| p.host).count() == 1
            && self.participants.iter().all(|p| {
                valid_peer_id(&p.peer_id)
                    && valid_text(&p.name, 32)
                    && valid_client(&p.client)
                    && (-MAX_DURATION_MS..=MAX_DURATION_MS).contains(&p.offset_ms)
                    && p.match_.is_none_or(|m| {
                        m.delta_ms.is_none_or(|d| (-MAX_INT..=MAX_INT).contains(&d))
                    })
            });
        self.rev >= 1
            && int(self.rev)
            && int(self.epoch)
            && int(self.at_ms)
            && valid_code(&self.code)
            && (0..=MAX_DURATION_MS).contains(&self.position_ms)
            && (self.phase != Phase::Idle || self.position_ms == 0)
            && (0.5..=2.0).contains(&self.rate)
            && waiting
            && participants
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Action {
    pub kind: String,
    #[serde(deserialize_with = "present")]
    pub by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct You {
    pub peer_id: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Joined {
    pub code: String,
    pub you: You,
    pub room: Room,
    #[serde(deserialize_with = "present")]
    pub media: Option<Media>,
    pub server_time: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WireError {
    pub code: String,
    pub message: String,
    #[serde(deserialize_with = "present")]
    pub re: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ServerMsg {
    Joined(Box<Joined>),
    Room {
        room: Box<Room>,
        action: Option<Action>,
        media: Option<Value>,
    },
    Pong {
        id: Value,
        server_time: i64,
    },
    Error(WireError),
    Closed(String),
    Malformed(String),
    Ignored,
}

pub fn parse(text: &str) -> ServerMsg {
    let Ok(mut v) = serde_json::from_str::<Value>(text) else {
        return ServerMsg::Ignored;
    };
    normalize_ints(&mut v);
    let Some(kind) = v.get("type").and_then(Value::as_str).map(str::to_owned) else {
        return ServerMsg::Ignored;
    };
    let msg = match kind.as_str() {
        "joined" => serde_json::from_value::<Joined>(v)
            .ok()
            .filter(|j| {
                valid_code(&j.code)
                    && valid_peer_id(&j.you.peer_id)
                    && j.you.token.len() == 43
                    && j.you
                        .token
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    && int_ok(j.server_time)
                    && j.room.valid()
                    && j.media.as_ref().is_none_or(|m| m.check(false).is_ok())
            })
            .map(|j| ServerMsg::Joined(Box::new(j))),
        "room" => {
            let media = v.as_object_mut().and_then(|o| o.remove("media"));
            let action = match v.get("action") {
                None => Some(None),
                Some(a) => serde_json::from_value::<Action>(a.clone())
                    .ok()
                    .filter(|a| !a.kind.is_empty() && a.by.as_deref().is_none_or(valid_peer_id))
                    .map(Some),
            };
            match (action, serde_json::from_value::<Room>(v)) {
                (Some(action), Ok(room)) if room.valid() => Some(ServerMsg::Room {
                    room: Box::new(room),
                    action,
                    media,
                }),
                _ => None,
            }
        }
        "pong" => match (v.get("id"), v.get("serverTime").and_then(Value::as_i64)) {
            (Some(id), Some(t)) if int_ok(t) => Some(ServerMsg::Pong {
                id: id.clone(),
                server_time: t,
            }),
            _ => None,
        },
        "error" => serde_json::from_value::<WireError>(v)
            .ok()
            .filter(|e| !e.code.is_empty())
            .map(ServerMsg::Error),
        "closed" => v
            .get("reason")
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
            .map(|r| ServerMsg::Closed(r.to_owned())),
        _ => return ServerMsg::Ignored,
    };
    msg.unwrap_or(ServerMsg::Malformed(kind))
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ClientMsg {
    Create {
        name: String,
        client: String,
    },
    Join {
        code: String,
        name: String,
        client: String,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        create: bool,
    },
    Resume {
        code: String,
        peer_id: String,
        token: String,
    },
    Ping {
        id: Value,
    },
    Status {
        epoch: i64,
        player: PlayerState,
        #[serde(rename = "match")]
        match_: Option<Match>,
        offset_ms: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        rtt_ms: Option<i64>,
    },
    Play,
    Pause {
        #[serde(skip_serializing_if = "Option::is_none")]
        position_ms: Option<i64>,
    },
    Seek {
        position_ms: i64,
    },
    Rate {
        rate: f64,
    },
    SetMedia {
        media: Box<Media>,
        position_ms: i64,
    },
    ClearMedia,
    SetControl {
        mode: Control,
    },
    TransferHost {
        peer_id: String,
    },
    Kick {
        peer_id: String,
    },
    Lock {
        locked: bool,
    },
    Leave,
    End,
}

impl ClientMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    Option::deserialize(d)
}

fn int_ok(v: i64) -> bool {
    (0..=MAX_INT).contains(&v)
}

// SPEC.md §2: receivers accept any integral number, so 3.0 is the int 3.
pub fn normalize_ints(v: &mut Value) {
    match v {
        Value::Number(n) if !n.is_i64() && !n.is_u64() => {
            if let Some(f) = n
                .as_f64()
                .filter(|f| f.fract() == 0.0 && f.abs() <= MAX_INT as f64)
            {
                *v = Value::from(f as i64);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(normalize_ints),
        Value::Object(o) => o.values_mut().for_each(normalize_ints),
        _ => {}
    }
}

pub fn forbidden(c: char) -> bool {
    matches!(c as u32, 0..=0x1F | 0x7F..=0x9F | 0x61C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069)
}

pub fn valid_text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.chars().count() <= max && !s.chars().any(forbidden)
}

pub fn valid_client(s: &str) -> bool {
    valid_text(s, 64)
        && s.split_once('/')
            .is_some_and(|(app, version)| !app.is_empty() && !version.is_empty())
}

pub fn valid_peer_id(s: &str) -> bool {
    (1..=16).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::Path;

    fn examples() -> Vec<(String, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/watch_together/vectors/examples");
        let mut all: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| {
                let path = e.unwrap().path();
                (
                    path.file_stem().unwrap().to_string_lossy().into_owned(),
                    std::fs::read_to_string(&path).unwrap(),
                )
            })
            .collect();
        all.sort();
        all
    }

    pub(crate) fn vectors(name: &str) -> Vec<Value> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("src/watch_together/vectors/{name}.json"));
        let text = std::fs::read_to_string(path).unwrap();
        let cases = text
            .lines()
            .map(|l| l.trim().trim_end_matches(','))
            .filter(|l| l.starts_with("{\"name\""));
        cases
            .map(|l| {
                serde_json::from_str(l).unwrap_or_else(|_| {
                    let expect = &l[l.rfind("\"expect\":").unwrap() + 9..l.len() - 1];
                    serde_json::json!({"name": l, "input": null, "expect": serde_json::from_str::<Value>(expect).unwrap()})
                })
            })
            .collect()
    }

    #[test]
    fn spec_examples_parse() {
        let server = ["joined", "room", "pong", "error", "closed"];
        let mut seen = 0;
        for (name, text) in examples() {
            let kind = name.split('.').next().unwrap();
            if server.contains(&kind) {
                let msg = parse(&text);
                assert!(
                    !matches!(msg, ServerMsg::Malformed(_) | ServerMsg::Ignored),
                    "{name}: {msg:?}"
                );
            } else if kind == "descriptor" {
                let mut v: Value = serde_json::from_str(&text).unwrap();
                normalize_ints(&mut v);
                let media: Media =
                    serde_json::from_value(v).unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(media.check(false), Ok(()), "{name}");
                assert_eq!(media.check(true), Ok(()), "{name}");
            } else if kind == "snapshot" {
                let room: Room = serde_json::from_str(&text).unwrap();
                assert!(room.valid(), "{name}");
            } else if kind == "relays" {
                assert!(
                    super::super::relays::parse_list(&serde_json::from_str(&text).unwrap())
                        .is_some(),
                    "{name}"
                );
            } else if kind == "setMedia" {
                let v: Value = serde_json::from_str(&text).unwrap();
                let media = Media::parse(v["media"].clone()).unwrap_or_else(|| panic!("{name}"));
                assert_eq!(serde_json::to_value(&media).unwrap(), v["media"], "{name}");
            } else if let Some(msg) = client_example(&name) {
                let mut v: Value = serde_json::from_str(&text).unwrap();
                normalize_ints(&mut v);
                let mut written = serde_json::to_value(&msg).unwrap();
                normalize_ints(&mut written);
                assert_eq!(written, v, "{name}");
            } else {
                continue;
            }
            seen += 1;
        }
        assert!(seen > 50);
    }

    fn client_example(name: &str) -> Option<ClientMsg> {
        let s = |v: &str| v.to_owned();
        let token = s("q3Jm8f0lYc2Kx9T4bWz7RuNs6dVhA1eGpLo5iCyZk0E");
        let matched = Some(Match {
            state: MatchState::Matched,
            delta_ms: Some(-120),
        });
        Some(match name {
            "create.basic" => ClientMsg::Create {
                name: s("Edde"),
                client: s("Example/1.31.0"),
            },
            "join.basic" => ClientMsg::Join {
                code: s("K7MQ2X"),
                name: s("Anna"),
                client: s("Sample/1.4.0"),
                create: false,
            },
            "join.create" => ClientMsg::Join {
                code: s("K7MQ2X"),
                name: s("Anna"),
                client: s("Sample/1.4.0"),
                create: true,
            },
            "resume.basic" => ClientMsg::Resume {
                code: s("K7MQ2X"),
                peer_id: s("p3"),
                token,
            },
            "ping.number" => ClientMsg::Ping { id: 17.into() },
            "status.matched" => ClientMsg::Status {
                epoch: 3,
                player: PlayerState::Ready,
                match_: matched,
                offset_ms: 0,
                rtt_ms: Some(48),
            },
            "play.basic" => ClientMsg::Play,
            "pause.with-position" => ClientMsg::Pause {
                position_ms: Some(61234),
            },
            "seek.basic" => ClientMsg::Seek {
                position_ms: 120_000,
            },
            "rate.faster" => ClientMsg::Rate { rate: 1.25 },
            "setControl.everyone" => ClientMsg::SetControl {
                mode: Control::Everyone,
            },
            "transferHost.basic" => ClientMsg::TransferHost { peer_id: s("p3") },
            "kick.basic" => ClientMsg::Kick { peer_id: s("p4") },
            "lock.on" => ClientMsg::Lock { locked: true },
            "leave.basic" => ClientMsg::Leave,
            "end.basic" => ClientMsg::End,
            "clearMedia.basic" => ClientMsg::ClearMedia,
            _ => return None,
        })
    }

    #[test]
    fn client_messages_write_ints_and_omit_absent_members() {
        let status = ClientMsg::Status {
            epoch: 3,
            player: PlayerState::Ready,
            match_: None,
            offset_ms: 0,
            rtt_ms: None,
        };
        assert_eq!(
            status.to_json(),
            r#"{"type":"status","epoch":3,"player":"ready","match":null,"offsetMs":0}"#
        );
        let join = ClientMsg::Join {
            code: "K7MQ2X".into(),
            name: "A".into(),
            client: "Plaivo/0.1.0".into(),
            create: false,
        };
        assert_eq!(
            join.to_json(),
            r#"{"type":"join","code":"K7MQ2X","name":"A","client":"Plaivo/0.1.0"}"#
        );
        assert_eq!(
            ClientMsg::Pause { position_ms: None }.to_json(),
            r#"{"type":"pause"}"#
        );
        assert_eq!(
            ClientMsg::TransferHost {
                peer_id: "p2".into()
            }
            .to_json(),
            r#"{"type":"transferHost","peerId":"p2"}"#
        );
    }

    #[test]
    fn room_check_vectors() {
        let cases = vectors("room-check");
        assert!(cases.len() > 80);
        for case in cases {
            let mut room = case["input"]["room"].clone();
            let valid = room.is_object() && {
                room["type"] = "room".into();
                !matches!(
                    parse(&room.to_string()),
                    ServerMsg::Malformed(_) | ServerMsg::Ignored
                )
            };
            assert_eq!(
                valid,
                case["expect"]["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn timeline_vectors() {
        for case in vectors("timeline") {
            let i = &case["input"];
            let room = test_room(&i["room"]);
            let now = i["serverNowMs"].as_i64().unwrap();
            let duration = i["durationMs"].as_i64().unwrap();
            assert_eq!(
                room.position_at(now, duration),
                case["expect"]["positionMs"].as_i64().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                room.running(now),
                case["expect"]["running"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    pub(crate) fn test_room(timeline: &Value) -> Room {
        let mut v = serde_json::json!({
            "rev": 1, "code": "K7MQ2X", "epoch": 1, "control": "host", "locked": false, "waitingOn": [],
            "participants": [{"peerId": "p1", "name": "A", "client": "A/1", "host": true, "connected": true, "player": "ready", "match": null, "offsetMs": 0}]
        });
        for (k, val) in timeline.as_object().unwrap() {
            v[k] = val.clone();
        }
        normalize_ints(&mut v);
        serde_json::from_value(v).unwrap()
    }
}
