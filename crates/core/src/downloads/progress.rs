use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::jellyfin;
use crate::plex::{Backend, Client, Server, drain, encode};

const WATCHED: f64 = 0.9;
const TICKS_PER_MS: u64 = 10_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pending {
    pub server: String,
    pub key: String,
    pub time_ms: u64,
    pub duration_ms: u64,
    pub updated: u64,
    #[serde(default)]
    pub watched: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Replay {
    pub path: String,
    pub body: Option<Value>,
}

pub(crate) fn watched(time_ms: u64, duration_ms: u64) -> bool {
    duration_ms > 0 && time_ms as f64 >= duration_ms as f64 * WATCHED
}

pub(crate) fn coalesce(list: &mut Vec<Pending>, mut p: Pending) {
    p.watched |= watched(p.time_ms, p.duration_ms)
        || list
            .iter()
            .any(|q| q.server == p.server && q.key == p.key && q.watched);
    list.retain(|q| q.server != p.server || q.key != p.key);
    list.push(p);
}

pub(crate) fn requests(srv: &Server, p: &Pending) -> Vec<Replay> {
    let key = encode(&p.key);
    let finished = watched(p.time_ms, p.duration_ms);
    match srv.backend {
        Backend::Plex => {
            let scrobble = p.watched.then(|| Replay {
                path: format!("/:/scrobble?identifier=com.plexapp.plugins.library&key={key}"),
                body: None,
            });
            let timeline = (!finished).then(|| Replay {
                path: format!(
                    "/:/timeline?ratingKey={key}&key=%2Flibrary%2Fmetadata%2F{key}&state=stopped&time={}&duration={}&offline=1&updated={}",
                    p.time_ms, p.duration_ms, p.updated
                ),
                body: None,
            });
            scrobble.into_iter().chain(timeline).collect()
        }
        Backend::Jellyfin => {
            let mut body = json!({
                "PlaybackPositionTicks": if finished { 0 } else { p.time_ms * TICKS_PER_MS },
                "LastPlayedDate": iso(p.updated),
            });
            if p.watched {
                body["Played"] = json!(true);
            }
            vec![Replay {
                path: format!("/UserItems/{key}/UserData?userId={}", encode(&srv.user_id)),
                body: Some(body),
            }]
        }
    }
}

fn server_is_newer(current: &Value, ours: &Value) -> bool {
    let date = |v: &Value| {
        v["LastPlayedDate"]
            .as_str()
            .and_then(|d| d.get(..19))
            .map(str::to_owned)
    };
    matches!((date(current), date(ours)), (Some(theirs), Some(ours)) if theirs > ours)
}

pub(crate) fn send(c: &Client, srv: &Server, r: &Replay) -> Result<(), ureq::Error> {
    let url = format!("{}{}", srv.uri, r.path);
    let Some(body) = &r.body else {
        return c
            .headers(c.agent.get(&url), Some(&srv.token))
            .call()
            .map(drain);
    };
    let current: Value = jellyfin::authed(c.agent.get(&url), c, Some(&srv.token))
        .call()?
        .body_mut()
        .read_json()?;
    let body = match server_is_newer(&current, body) {
        false => body.clone(),
        true if body["Played"] == json!(true) => json!({ "Played": true }),
        true => return Ok(()),
    };
    jellyfin::authed(c.agent.post(&url), c, Some(&srv.token))
        .send_json(&body)
        .map(drain)
}

pub(crate) fn iso(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}
