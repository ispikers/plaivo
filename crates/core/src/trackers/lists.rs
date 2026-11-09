use super::Service;
use super::http::{Body, Request};
use serde_json::{Value, json};

const MAL_API: &str = "https://api.myanimelist.net/v2";

pub(super) fn start_request(relay: &str, service: Service) -> Request {
    Request::post(format!("{}/auth/start", relay.trim_end_matches('/')), json!({ "service": service.key() }))
}

pub(super) fn result_request(relay: &str, session: &str) -> Request {
    Request::get(format!("{}/auth/result?session={session}", relay.trim_end_matches('/')))
}

pub(super) fn refresh_request(relay: &str, service: Service, refresh_token: &str) -> Request {
    Request::post(format!("{}/auth/refresh", relay.trim_end_matches('/')), json!({ "service": service.key(), "refresh_token": refresh_token }))
}

#[derive(Debug, PartialEq)]
pub(super) struct Snapshot {
    pub total: u32,
    pub progress: u32,
    pub locked: bool,
}

pub(super) fn snapshot_request(id: u64, token: &str) -> Request {
    Request::get(format!("{MAL_API}/anime/{id}?fields=num_episodes,my_list_status")).bearer(token)
}

fn uint(v: &Value) -> u32 {
    v.as_u64().map_or(0, |n| n.min(u32::MAX as u64) as u32)
}

pub(super) fn parse_snapshot(v: &Value) -> Option<Snapshot> {
    let total = uint(v.get("num_episodes")?);
    let list = &v["my_list_status"];
    let watched = list.get("num_episodes_watched").or_else(|| list.get("num_watched_episodes")).map_or(0, uint);
    let locked = list["status"].as_str() == Some("completed") || list["is_rewatching"].as_bool() == Some(true);
    Some(Snapshot { total, progress: watched, locked })
}

pub(super) fn decide(snap: &Snapshot, progress: u32, movie: bool) -> Option<(u32, bool)> {
    if snap.locked {
        return None;
    }
    let (progress, complete) = match (movie, snap.total) {
        (true, total) => (total.max(1), true),
        (false, 0) => (progress, false),
        (false, total) if progress > total => return None,
        (false, total) => (progress, progress == total),
    };
    (progress > snap.progress || (complete && progress >= snap.progress)).then_some((progress, complete))
}

pub(super) fn update_request(id: u64, token: &str, progress: u32, complete: bool) -> Request {
    let status = if complete { "completed" } else { "watching" };
    Request::new("PATCH", format!("{MAL_API}/anime/{id}/my_list_status"), Body::Form(format!("status={status}&num_watched_episodes={progress}"))).bearer(token)
}
