use serde_json::Value;
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub(crate) enum Body {
    Empty,
    Json(Value),
    Form(String),
}

#[derive(Debug, PartialEq)]
pub(crate) struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Body,
}

pub(crate) struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    pub etag: Option<String>,
}

impl Response {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_default()
    }
}

impl Request {
    pub fn new(method: &'static str, url: impl Into<String>, body: Body) -> Self {
        Self { method, url: url.into(), headers: vec![("Accept", "application/json".into())], body }
    }

    pub fn get(url: impl Into<String>) -> Self {
        Self::new("GET", url, Body::Empty)
    }

    pub fn post(url: impl Into<String>, json: Value) -> Self {
        Self::new("POST", url, Body::Json(json))
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn bearer(self, token: &str) -> Self {
        self.header("Authorization", format!("Bearer {token}"))
    }

    #[cfg(test)]
    pub fn header_value(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        crate::http()
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(concat!("Plaivo/", env!("CARGO_PKG_VERSION")))
            .build()
            .into()
    })
}

pub(crate) fn send(req: &Request) -> Result<Response, String> {
    let mut b = ureq::http::Request::builder().method(req.method).uri(&req.url);
    for (k, v) in &req.headers {
        b = b.header(*k, v);
    }
    let result = match &req.body {
        Body::Empty => b.body(()).map(|r| agent().run(r)),
        Body::Json(v) => b.header("Content-Type", "application/json").body(v.to_string()).map(|r| agent().run(r)),
        Body::Form(f) => b.header("Content-Type", "application/x-www-form-urlencoded").body(f.clone()).map(|r| agent().run(r)),
    };
    let mut resp = result.map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    let etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let body = resp.body_mut().with_config().limit(16 << 20).read_to_vec().map_err(|e| e.to_string())?;
    Ok(Response { status: resp.status().as_u16(), body, etag })
}
