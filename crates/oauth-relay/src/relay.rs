use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

const TTL: Duration = Duration::from_secs(600);
const MAX_SESSIONS: usize = 65536;
const MAX_PARAM: usize = 2048;

const MAL_AUTHORIZE: &str = "https://myanimelist.net/v1/oauth2/authorize";
const MAL_TOKEN: &str = "https://myanimelist.net/v1/oauth2/token";
const ANILIST_AUTHORIZE: &str = "https://anilist.co/api/v2/oauth/authorize";
const ANILIST_TOKEN: &str = "https://anilist.co/api/v2/oauth/token";

const HTML: &str = "text/html; charset=utf-8";
const JSON: &str = "application/json";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    Mal,
    Anilist,
}

impl Service {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "mal" => Some(Self::Mal),
            "anilist" => Some(Self::Anilist),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Mal => "mal",
            Self::Anilist => "anilist",
        }
    }
}

pub struct Client {
    pub id: String,
    pub secret: Option<String>,
}

pub struct Config {
    pub public_url: String,
    pub mal: Option<Client>,
    pub anilist: Option<Client>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenRequest {
    Form(&'static str, Vec<(&'static str, String)>),
    Json(&'static str, serde_json::Value),
}

#[derive(Debug)]
pub enum ExchangeError {
    Rejected,
    Unavailable,
}

pub type Exchange = Box<dyn Fn(&TokenRequest) -> Result<Tokens, ExchangeError> + Send + Sync>;

pub struct Reply {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
}

impl Reply {
    pub fn error(status: u16, code: &str) -> Self {
        Self::json(status, json!({ "error": code }))
    }

    fn json(status: u16, value: serde_json::Value) -> Self {
        Self {
            status,
            content_type: JSON,
            body: value.to_string(),
        }
    }

    fn page(status: u16, message: &str) -> Self {
        let body = format!(
            "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>Plaivo</title>\
             <body style=\"font:18px system-ui,sans-serif;text-align:center;padding:15vh 1em\"><h1>Plaivo</h1><p>{message}</p>"
        );
        Self {
            status,
            content_type: HTML,
            body,
        }
    }
}

struct Session {
    service: Service,
    verifier: String,
    expires: Instant,
    outcome: Option<Result<Tokens, &'static str>>,
}

#[derive(Default)]
struct Store {
    sessions: HashMap<String, Session>,
    states: HashMap<String, String>,
}

pub struct Relay {
    config: Config,
    exchange: Exchange,
    ttl: Duration,
    max_sessions: usize,
    store: Mutex<Store>,
}

impl Relay {
    pub fn new(config: Config, exchange: Exchange) -> Self {
        Self {
            config,
            exchange,
            ttl: TTL,
            max_sessions: MAX_SESSIONS,
            store: Mutex::default(),
        }
    }

    pub fn handle(&self, method: &str, url: &str, body: &[u8]) -> Reply {
        let (path, query) = url.split_once('?').unwrap_or((url, ""));
        match (method, path) {
            ("POST", "/auth/start") => self.start(body),
            ("GET", "/auth/result") => self.result(query),
            ("POST", "/auth/refresh") => self.refresh(body),
            ("GET", path) => match path
                .strip_prefix("/auth/")
                .and_then(|rest| rest.strip_suffix("/callback"))
                .and_then(Service::parse)
            {
                Some(service) => self.callback(service, query),
                None => Reply::error(404, "not_found"),
            },
            _ => Reply::error(404, "not_found"),
        }
    }

    fn client(&self, service: Service) -> Option<&Client> {
        match service {
            Service::Mal => self.config.mal.as_ref(),
            Service::Anilist => self.config.anilist.as_ref(),
        }
    }

    fn redirect_uri(&self, service: Service) -> String {
        format!(
            "{}/auth/{}/callback",
            self.config.public_url,
            service.name()
        )
    }

    fn start(&self, body: &[u8]) -> Reply {
        #[derive(Deserialize)]
        struct Start {
            service: String,
        }
        let Some(service) = serde_json::from_slice::<Start>(body)
            .ok()
            .and_then(|start| Service::parse(&start.service))
        else {
            return Reply::error(400, "invalid_service");
        };
        let Some(client) = self.client(service) else {
            return Reply::error(503, "service_not_configured");
        };
        let (id, state, verifier) = (random_hex(32), random_hex(32), random_hex(48));
        let mut query = form_urlencoded::Serializer::new(String::new());
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client.id)
            .append_pair("redirect_uri", &self.redirect_uri(service))
            .append_pair("state", &state);
        let base = match service {
            Service::Mal => {
                query
                    .append_pair("code_challenge", &verifier)
                    .append_pair("code_challenge_method", "plain");
                MAL_AUTHORIZE
            }
            Service::Anilist => ANILIST_AUTHORIZE,
        };
        let authorize_url = format!("{base}?{}", query.finish());

        let mut store = self.store.lock().unwrap();
        let Store { sessions, states } = &mut *store;
        let now = Instant::now();
        sessions.retain(|_, session| session.expires > now);
        states.retain(|_, id| sessions.contains_key(id));
        if sessions.len() >= self.max_sessions {
            return Reply::error(503, "busy");
        }
        sessions.insert(
            id.clone(),
            Session {
                service,
                verifier,
                expires: now + self.ttl,
                outcome: None,
            },
        );
        states.insert(state, id.clone());
        Reply::json(
            200,
            json!({ "session": id, "authorize_url": authorize_url, "expires_in": self.ttl.as_secs() }),
        )
    }

    fn callback(&self, service: Service, query: &str) -> Reply {
        const EXPIRED: &str =
            "This sign-in link has expired or was already used. Start again from Plaivo.";
        let params: HashMap<String, String> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        let param = |key: &str| {
            params
                .get(key)
                .map(String::as_str)
                .filter(|v| !v.is_empty() && v.len() <= MAX_PARAM)
        };
        let Some(state) = param("state") else {
            return Reply::page(400, EXPIRED);
        };

        let (id, verifier) = {
            let mut store = self.store.lock().unwrap();
            let Store { sessions, states } = &mut *store;
            let Some(session) = states
                .get(state)
                .and_then(|id| sessions.get_mut(id))
                .filter(|s| {
                    s.service == service && s.outcome.is_none() && s.expires > Instant::now()
                })
            else {
                return Reply::page(400, EXPIRED);
            };
            let verifier = std::mem::take(&mut session.verifier);
            (
                states.remove(state).expect("state was just found"),
                verifier,
            )
        };

        let outcome = match (param("error"), param("code")) {
            (Some("access_denied"), _) => Err("access_denied"),
            (None, Some(code)) => (self.exchange)(&self.code_request(service, code, &verifier))
                .map_err(|_| "exchange_failed"),
            _ => Err("exchange_failed"),
        };
        let reply = match outcome {
            Ok(_) => Reply::page(
                200,
                "Signed in. You can close this tab and return to Plaivo.",
            ),
            Err("access_denied") => {
                Reply::page(200, "Sign-in was cancelled. You can close this tab.")
            }
            Err(_) => Reply::page(
                502,
                "Sign-in failed. Close this tab and try again from Plaivo.",
            ),
        };
        if let Some(session) = self.store.lock().unwrap().sessions.get_mut(&id) {
            session.outcome = Some(outcome);
            session.expires = Instant::now() + self.ttl;
        }
        reply
    }

    fn code_request(&self, service: Service, code: &str, verifier: &str) -> TokenRequest {
        let client = self
            .client(service)
            .expect("session exists only for configured services");
        let redirect_uri = self.redirect_uri(service);
        match service {
            Service::Mal => {
                let mut form = mal_credentials(client);
                form.extend([
                    ("grant_type", "authorization_code".to_owned()),
                    ("code", code.to_owned()),
                    ("redirect_uri", redirect_uri),
                    ("code_verifier", verifier.to_owned()),
                ]);
                TokenRequest::Form(MAL_TOKEN, form)
            }
            Service::Anilist => TokenRequest::Json(
                ANILIST_TOKEN,
                json!({
                    "grant_type": "authorization_code",
                    "client_id": client.id,
                    "client_secret": client.secret.as_deref().unwrap_or_default(),
                    "redirect_uri": redirect_uri,
                    "code": code,
                }),
            ),
        }
    }

    fn result(&self, query: &str) -> Reply {
        let Some((_, id)) = form_urlencoded::parse(query.as_bytes())
            .find(|(key, value)| key == "session" && !value.is_empty() && value.len() <= MAX_PARAM)
        else {
            return Reply::error(400, "invalid_session");
        };
        let mut store = self.store.lock().unwrap();
        let sessions = &mut store.sessions;
        let Some(session) = sessions
            .remove(id.as_ref())
            .filter(|s| s.expires > Instant::now())
        else {
            return Reply::error(410, "expired");
        };
        match session.outcome {
            None => {
                sessions.insert(id.into_owned(), session);
                Reply {
                    status: 204,
                    content_type: JSON,
                    body: String::new(),
                }
            }
            Some(Ok(tokens)) => Reply::json(200, json!(tokens)),
            Some(Err(code)) => Reply::error(403, code),
        }
    }

    fn refresh(&self, body: &[u8]) -> Reply {
        #[derive(Deserialize)]
        struct Refresh {
            service: String,
            refresh_token: String,
        }
        let Some(request) = serde_json::from_slice::<Refresh>(body)
            .ok()
            .filter(|r| !r.refresh_token.is_empty() && r.refresh_token.len() <= MAX_PARAM)
        else {
            return Reply::error(400, "invalid_request");
        };
        match Service::parse(&request.service) {
            None => Reply::error(400, "invalid_service"),
            Some(Service::Anilist) => Reply::error(400, "refresh_unsupported"),
            Some(Service::Mal) => {
                let Some(client) = self.client(Service::Mal) else {
                    return Reply::error(503, "service_not_configured");
                };
                let mut form = mal_credentials(client);
                form.extend([
                    ("grant_type", "refresh_token".to_owned()),
                    ("refresh_token", request.refresh_token),
                ]);
                match (self.exchange)(&TokenRequest::Form(MAL_TOKEN, form)) {
                    Ok(tokens) => Reply::json(200, json!(tokens)),
                    Err(ExchangeError::Rejected) => Reply::error(401, "refresh_rejected"),
                    Err(ExchangeError::Unavailable) => Reply::error(502, "provider_unavailable"),
                }
            }
        }
    }
}

fn mal_credentials(client: &Client) -> Vec<(&'static str, String)> {
    let mut form = vec![("client_id", client.id.clone())];
    form.extend(
        client
            .secret
            .clone()
            .map(|secret| ("client_secret", secret)),
    );
    form
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn tokens() -> Tokens {
        Tokens {
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_in: Some(3600),
            token_type: Some("Bearer".into()),
        }
    }

    fn fake(
        reply: fn() -> Result<Tokens, ExchangeError>,
    ) -> (Relay, Arc<Mutex<Vec<TokenRequest>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let config = Config {
            public_url: "https://relay.test".into(),
            mal: Some(Client {
                id: "mal-id".into(),
                secret: Some("mal-secret".into()),
            }),
            anilist: Some(Client {
                id: "ani-id".into(),
                secret: Some("ani-secret".into()),
            }),
        };
        let exchange: Exchange = Box::new(move |request: &TokenRequest| {
            log.lock().unwrap().push(request.clone());
            reply()
        });
        (Relay::new(config, exchange), seen)
    }

    fn start(relay: &Relay, service: &str) -> (String, HashMap<String, String>) {
        let reply = relay.handle(
            "POST",
            "/auth/start",
            format!(r#"{{"service":"{service}"}}"#).as_bytes(),
        );
        assert_eq!(reply.status, 200);
        let body: serde_json::Value = serde_json::from_str(&reply.body).unwrap();
        let url = body["authorize_url"].as_str().unwrap();
        let query = url.split_once('?').unwrap().1;
        let params = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();
        (body["session"].as_str().unwrap().to_owned(), params)
    }

    fn result(relay: &Relay, session: &str) -> Reply {
        relay.handle("GET", &format!("/auth/result?session={session}"), b"")
    }

    #[test]
    fn mal_flow_delivers_tokens_once() {
        let (relay, seen) = fake(|| Ok(tokens()));
        let (session, params) = start(&relay, "mal");
        assert_eq!(params["client_id"], "mal-id");
        assert_eq!(
            params["redirect_uri"],
            "https://relay.test/auth/mal/callback"
        );
        assert_eq!(params["code_challenge_method"], "plain");
        assert_eq!(result(&relay, &session).status, 204);

        let state = &params["state"];
        let page = relay.handle(
            "GET",
            &format!("/auth/mal/callback?code=abc&state={state}"),
            b"",
        );
        assert_eq!(page.status, 200);
        assert_eq!(page.content_type, HTML);

        let got = result(&relay, &session);
        assert_eq!(got.status, 200);
        assert_eq!(serde_json::from_str::<Tokens>(&got.body).unwrap(), tokens());
        assert_eq!(result(&relay, &session).status, 410);

        let seen = seen.lock().unwrap();
        let TokenRequest::Form(url, form) = &seen[0] else {
            panic!("MAL uses a form body")
        };
        assert_eq!(*url, MAL_TOKEN);
        let form: HashMap<_, _> = form.iter().cloned().collect();
        assert_eq!(form["code"], "abc");
        assert_eq!(form["code_verifier"], params["code_challenge"]);
        assert_eq!(form["client_secret"], "mal-secret");
        assert_eq!(form["redirect_uri"], params["redirect_uri"]);
    }

    #[test]
    fn anilist_flow_uses_client_secret() {
        let (relay, seen) = fake(|| Ok(tokens()));
        let (session, params) = start(&relay, "anilist");
        assert!(!params.contains_key("code_challenge"));
        let state = &params["state"];
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/anilist/callback?code=xyz&state={state}"),
                    b""
                )
                .status,
            200
        );
        assert_eq!(result(&relay, &session).status, 200);
        let seen = seen.lock().unwrap();
        let TokenRequest::Json(url, body) = &seen[0] else {
            panic!("AniList uses a JSON body")
        };
        assert_eq!(*url, ANILIST_TOKEN);
        assert_eq!(body["client_secret"], "ani-secret");
        assert_eq!(body["code"], "xyz");
        assert_eq!(body["grant_type"], "authorization_code");
    }

    #[test]
    fn unknown_or_reused_state_is_rejected() {
        let (relay, seen) = fake(|| Ok(tokens()));
        let (session, params) = start(&relay, "mal");
        let state = &params["state"];
        assert_eq!(
            relay
                .handle("GET", "/auth/mal/callback?code=abc&state=nope", b"")
                .status,
            400
        );
        assert_eq!(
            relay
                .handle("GET", "/auth/mal/callback?code=abc", b"")
                .status,
            400
        );
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/anilist/callback?code=abc&state={state}"),
                    b""
                )
                .status,
            400
        );
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/mal/callback?code=abc&state={state}"),
                    b""
                )
                .status,
            200
        );
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/mal/callback?code=abc&state={state}"),
                    b""
                )
                .status,
            400
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(result(&relay, &session).status, 200);
    }

    #[test]
    fn denied_and_failed_exchanges_report_errors() {
        let (relay, seen) = fake(|| Ok(tokens()));
        let (session, params) = start(&relay, "mal");
        let state = &params["state"];
        relay.handle(
            "GET",
            &format!("/auth/mal/callback?error=access_denied&state={state}"),
            b"",
        );
        let got = result(&relay, &session);
        assert_eq!(
            (got.status, got.body.as_str()),
            (403, r#"{"error":"access_denied"}"#)
        );
        assert!(seen.lock().unwrap().is_empty());

        let (relay, _) = fake(|| Err(ExchangeError::Rejected));
        let (session, params) = start(&relay, "anilist");
        let state = &params["state"];
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/anilist/callback?code=c&state={state}"),
                    b""
                )
                .status,
            502
        );
        assert_eq!(
            result(&relay, &session).body,
            r#"{"error":"exchange_failed"}"#
        );
        assert_eq!(result(&relay, &session).status, 410);
    }

    #[test]
    fn other_provider_errors_are_failures() {
        let (relay, seen) = fake(|| Ok(tokens()));
        let (session, params) = start(&relay, "mal");
        let state = &params["state"];
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/mal/callback?error=server_error&code=c&state={state}"),
                    b""
                )
                .status,
            502
        );
        assert_eq!(
            result(&relay, &session).body,
            r#"{"error":"exchange_failed"}"#
        );
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn session_cap_answers_busy() {
        let (mut relay, _) = fake(|| Ok(tokens()));
        relay.max_sessions = 2;
        start(&relay, "mal");
        start(&relay, "anilist");
        let busy = relay.handle("POST", "/auth/start", br#"{"service":"mal"}"#);
        assert_eq!(
            (busy.status, busy.body.as_str()),
            (503, r#"{"error":"busy"}"#)
        );
    }

    #[test]
    fn expired_sessions_are_gone() {
        let (mut relay, _) = fake(|| Ok(tokens()));
        relay.ttl = Duration::ZERO;
        let (session, params) = start(&relay, "mal");
        let state = &params["state"];
        assert_eq!(result(&relay, &session).status, 410);
        assert_eq!(
            relay
                .handle(
                    "GET",
                    &format!("/auth/mal/callback?code=abc&state={state}"),
                    b""
                )
                .status,
            400
        );
    }

    #[test]
    fn invalid_requests_are_rejected() {
        let (mut relay, _) = fake(|| Ok(tokens()));
        assert_eq!(
            relay
                .handle("POST", "/auth/start", b"{\"service\":\"trakt\"}")
                .status,
            400
        );
        assert_eq!(relay.handle("POST", "/auth/start", b"not json").status, 400);
        assert_eq!(relay.handle("GET", "/auth/result", b"").status, 400);
        assert_eq!(result(&relay, "unknown").status, 410);
        assert_eq!(
            relay
                .handle("GET", "/auth/trakt/callback?code=a&state=b", b"")
                .status,
            404
        );
        assert_eq!(relay.handle("GET", "/", b"").status, 404);
        relay.config.anilist = None;
        assert_eq!(
            relay
                .handle("POST", "/auth/start", b"{\"service\":\"anilist\"}")
                .status,
            503
        );
    }

    #[test]
    fn refresh_uses_the_secret_and_maps_errors() {
        let (relay, seen) = fake(|| Ok(tokens()));
        let got = relay.handle(
            "POST",
            "/auth/refresh",
            br#"{"service":"mal","refresh_token":"r1"}"#,
        );
        assert_eq!(got.status, 200);
        assert_eq!(serde_json::from_str::<Tokens>(&got.body).unwrap(), tokens());
        let TokenRequest::Form(_, form) = &seen.lock().unwrap()[0] else {
            panic!("MAL uses a form body")
        };
        let form: HashMap<_, _> = form.iter().cloned().collect();
        assert_eq!(
            (form["grant_type"].as_str(), form["refresh_token"].as_str()),
            ("refresh_token", "r1")
        );
        assert_eq!(form["client_secret"], "mal-secret");

        assert_eq!(
            relay
                .handle(
                    "POST",
                    "/auth/refresh",
                    br#"{"service":"anilist","refresh_token":"r"}"#
                )
                .status,
            400
        );
        assert_eq!(
            relay
                .handle(
                    "POST",
                    "/auth/refresh",
                    br#"{"service":"mal","refresh_token":""}"#
                )
                .status,
            400
        );
        let (rejecting, _) = fake(|| Err(ExchangeError::Rejected));
        assert_eq!(
            rejecting
                .handle(
                    "POST",
                    "/auth/refresh",
                    br#"{"service":"mal","refresh_token":"r"}"#
                )
                .status,
            401
        );
        let (down, _) = fake(|| Err(ExchangeError::Unavailable));
        assert_eq!(
            down.handle(
                "POST",
                "/auth/refresh",
                br#"{"service":"mal","refresh_token":"r"}"#
            )
            .status,
            502
        );
    }
}
