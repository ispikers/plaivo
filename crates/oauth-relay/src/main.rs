mod relay;

use std::io::Read;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use relay::{Client, Config, Exchange, ExchangeError, Relay, Reply, TokenRequest, Tokens};
use tiny_http::{Header, Request, Response, Server};

const MAX_BODY: usize = 4096;
const WORKERS: usize = 8;

fn main() {
    let config = config().unwrap_or_else(|message| fail(&message));
    let listen = env("PLAIVO_RELAY_LISTEN").unwrap_or_else(|| "127.0.0.1:8787".to_owned());
    let server = Arc::new(
        Server::http(&listen).unwrap_or_else(|e| fail(&format!("cannot listen on {listen}: {e}"))),
    );
    let relay = Arc::new(Relay::new(config, http_exchange()));
    eprintln!("plaivo-oauth-relay listening on {listen}");
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (server, relay) = (server.clone(), relay.clone());
            thread::spawn(move || serve(&server, &relay))
        })
        .collect();
    for worker in workers {
        let _ = worker.join();
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(2)
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn config() -> Result<Config, String> {
    let public_url = env("PLAIVO_RELAY_PUBLIC_URL")
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .ok_or("PLAIVO_RELAY_PUBLIC_URL must be set to the relay's public http(s) URL")?
        .trim_end_matches('/')
        .to_owned();
    let mal = env("PLAIVO_MAL_CLIENT_ID").map(|id| Client {
        id,
        secret: env("PLAIVO_MAL_CLIENT_SECRET"),
    });
    let anilist =
        match (
            env("PLAIVO_ANILIST_CLIENT_ID"),
            env("PLAIVO_ANILIST_CLIENT_SECRET"),
        ) {
            (Some(id), Some(secret)) => Some(Client {
                id,
                secret: Some(secret),
            }),
            (None, None) => None,
            _ => return Err(
                "set both PLAIVO_ANILIST_CLIENT_ID and PLAIVO_ANILIST_CLIENT_SECRET, or neither"
                    .into(),
            ),
        };
    if mal.is_none() && anilist.is_none() {
        return Err(
            "configure at least one service: PLAIVO_MAL_CLIENT_ID or PLAIVO_ANILIST_CLIENT_ID"
                .into(),
        );
    }
    Ok(Config {
        public_url,
        mal,
        anilist,
    })
}

fn http_exchange() -> Exchange {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    Box::new(move |request: &TokenRequest| {
        let (url, sent) = match request {
            TokenRequest::Form(url, form) => (
                url,
                agent
                    .post(*url)
                    .header("Accept", "application/json")
                    .send_form(form.iter().map(|(k, v)| (*k, v.as_str()))),
            ),
            TokenRequest::Json(url, body) => (
                url,
                agent
                    .post(*url)
                    .header("Accept", "application/json")
                    .send_json(body),
            ),
        };
        let error = match sent.and_then(|mut response| response.body_mut().read_json::<Tokens>()) {
            Ok(tokens) => return Ok(tokens),
            Err(ureq::Error::StatusCode(code)) => {
                eprintln!("token request to {url} answered {code}");
                return Err(exchange_error(code));
            }
            Err(e) => e,
        };
        eprintln!("token request to {url} failed: {error}");
        Err(ExchangeError::Unavailable)
    })
}

fn exchange_error(status: u16) -> ExchangeError {
    match status {
        400 | 401 => ExchangeError::Rejected,
        _ => ExchangeError::Unavailable,
    }
}

fn reply(request: &mut Request, relay: &Relay) -> Reply {
    let chunked = request
        .headers()
        .iter()
        .any(|h| h.field.equiv("Transfer-Encoding"));
    match request.body_length() {
        Some(length) if length > MAX_BODY => return Reply::error(413, "body_too_large"),
        None if chunked => return Reply::error(413, "body_too_large"),
        _ => {}
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take(MAX_BODY as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        return Reply::error(400, "invalid_body");
    }
    relay.handle(request.method().as_str(), request.url(), &body)
}

fn serve(server: &Server, relay: &Relay) {
    while let Ok(mut request) = server.recv() {
        let reply = reply(&mut request, relay);
        let path = request.url().split('?').next().unwrap_or_default();
        eprintln!("{} {path} {}", request.method(), reply.status);
        let response = Response::from_string(reply.body)
            .with_status_code(reply.status)
            .with_header(header("Content-Type", reply.content_type))
            .with_header(header("Cache-Control", "no-store"))
            .with_header(header("Referrer-Policy", "no-referrer"));
        let _ = request.respond(response);
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("valid static header")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpStream;

    #[test]
    fn only_bad_request_and_unauthorized_reject_tokens() {
        assert!(matches!(exchange_error(400), ExchangeError::Rejected));
        assert!(matches!(exchange_error(401), ExchangeError::Rejected));
        assert!(matches!(exchange_error(403), ExchangeError::Unavailable));
        assert!(matches!(exchange_error(429), ExchangeError::Unavailable));
        assert!(matches!(exchange_error(500), ExchangeError::Unavailable));
    }

    fn status_of(raw: &str) -> String {
        let server = Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let config = Config {
            public_url: "http://relay.test".into(),
            mal: Some(Client {
                id: "id".into(),
                secret: None,
            }),
            anilist: None,
        };
        let relay = Relay::new(config, Box::new(|_| Err(ExchangeError::Unavailable)));
        thread::spawn(move || serve(&server, &relay));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(raw.as_bytes()).unwrap();
        let mut response = [0u8; 12];
        stream.read_exact(&mut response).unwrap();
        String::from_utf8_lossy(&response[9..]).into_owned()
    }

    #[test]
    fn oversized_and_chunked_bodies_are_refused_unread() {
        let head = "POST /auth/start HTTP/1.1\r\nHost: x\r\n";
        assert_eq!(
            status_of(&format!("{head}Content-Length: 5000\r\n\r\n")),
            "413"
        );
        assert_eq!(
            status_of(&format!("{head}Transfer-Encoding: chunked\r\n\r\n")),
            "413"
        );
        let body = r#"{"service":"mal"}"#;
        assert_eq!(
            status_of(&format!(
                "{head}Content-Length: {}\r\n\r\n{body}",
                body.len()
            )),
            "200"
        );
    }
}
