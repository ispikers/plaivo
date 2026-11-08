use super::Service;
use super::http::{Body, Request};
use serde_json::{Value, json};

const MAL_API: &str = "https://api.myanimelist.net/v2";
const ANILIST_API: &str = "https://graphql.anilist.co";

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

const ANILIST_QUERY: &str = "query($id:Int){Media(id:$id,type:ANIME){episodes mediaListEntry{status progress}}}";
const ANILIST_SAVE: &str = "mutation($id:Int,$progress:Int,$status:MediaListStatus){SaveMediaListEntry(mediaId:$id,progress:$progress,status:$status){id}}";

pub(super) fn snapshot_request(service: Service, id: u64, token: &str) -> Request {
    match service {
        Service::Anilist => Request::post(ANILIST_API, json!({ "query": ANILIST_QUERY, "variables": { "id": id } })),
        _ => Request::get(format!("{MAL_API}/anime/{id}?fields=num_episodes,my_list_status")),
    }
    .bearer(token)
}

fn uint(v: &Value) -> u32 {
    v.as_u64().map_or(0, |n| n.min(u32::MAX as u64) as u32)
}

pub(super) fn parse_snapshot(service: Service, v: &Value) -> Option<Snapshot> {
    match service {
        Service::Anilist => {
            let media = v.pointer("/data/Media").filter(|m| m.is_object())?;
            let entry = &media["mediaListEntry"];
            let status = entry["status"].as_str().unwrap_or("");
            Some(Snapshot { total: uint(&media["episodes"]), progress: uint(&entry["progress"]), locked: matches!(status, "COMPLETED" | "REPEATING") })
        }
        _ => {
            let total = uint(v.get("num_episodes")?);
            let list = &v["my_list_status"];
            let watched = list.get("num_episodes_watched").or_else(|| list.get("num_watched_episodes")).map_or(0, uint);
            let locked = list["status"].as_str() == Some("completed") || list["is_rewatching"].as_bool() == Some(true);
            Some(Snapshot { total, progress: watched, locked })
        }
    }
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

pub(super) fn update_request(service: Service, id: u64, token: &str, progress: u32, complete: bool) -> Request {
    match service {
        Service::Anilist => {
            let status = if complete { "COMPLETED" } else { "CURRENT" };
            Request::post(ANILIST_API, json!({ "query": ANILIST_SAVE, "variables": { "id": id, "progress": progress, "status": status } }))
        }
        _ => {
            let status = if complete { "completed" } else { "watching" };
            Request::new("PATCH", format!("{MAL_API}/anime/{id}/my_list_status"), Body::Form(format!("status={status}&num_watched_episodes={progress}")))
        }
    }
    .bearer(token)
}
