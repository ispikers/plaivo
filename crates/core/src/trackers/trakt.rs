use super::Media;
use super::http::Request;
use serde_json::{Map, Value, json};

const API: &str = "https://api.trakt.tv";
pub(super) const WATCHED_PCT: f64 = 80.0;

fn api(req: Request, client_id: &str) -> Request {
    req.header("trakt-api-version", "2").header("trakt-api-key", client_id)
}

pub(super) fn device_code_request(client_id: &str) -> Request {
    api(Request::post(format!("{API}/oauth/device/code"), json!({ "client_id": client_id })), client_id)
}

pub(super) fn device_token_request(client_id: &str, device_code: &str) -> Request {
    api(Request::post(format!("{API}/oauth/device/token"), json!({ "code": device_code, "client_id": client_id })), client_id)
}

pub(super) fn refresh_request(client_id: &str, refresh_token: &str) -> Request {
    let body = json!({
        "refresh_token": refresh_token,
        "client_id": client_id,
        "redirect_uri": "urn:ietf:wg:oauth:2.0:oob",
        "grant_type": "refresh_token",
    });
    api(Request::post(format!("{API}/oauth/token"), body), client_id)
}

pub(super) fn scrobble_request(client_id: &str, token: &str, action: &str, body: Value) -> Request {
    api(Request::post(format!("{API}/scrobble/{action}"), body), client_id).bearer(token)
}

fn trakt_ids(ids: &super::Ids) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(imdb) = ids.get("imdb").filter(|v| v.starts_with("tt")) {
        out.insert("imdb".into(), imdb.clone().into());
    }
    for key in ["tmdb", "tvdb"] {
        if let Some(n) = ids.get(key).and_then(|v| v.parse::<u64>().ok()) {
            out.insert(key.into(), n.into());
        }
    }
    out
}

pub(super) fn scrobble_body(media: &Media, pct: f64) -> Option<Value> {
    let progress = (pct.clamp(0.0, 100.0) * 100.0).round() / 100.0;
    if media.movie {
        let ids = trakt_ids(&media.ids);
        return (!ids.is_empty()).then(|| json!({ "movie": { "ids": ids }, "progress": progress }));
    }
    let show = trakt_ids(&media.show_ids);
    if let (false, Some(season), Some(number)) = (show.is_empty(), media.season, media.episode) {
        return Some(json!({ "show": { "ids": show }, "episode": { "season": season, "number": number }, "progress": progress }));
    }
    let ids = trakt_ids(&media.ids);
    (!ids.is_empty()).then(|| json!({ "episode": { "ids": ids }, "progress": progress }))
}
