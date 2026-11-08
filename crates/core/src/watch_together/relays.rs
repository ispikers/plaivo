use super::protocol::valid_text;
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(not(target_os = "android"))]
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub const ALPHABET: &str = "23456789ABCDEFGHJKMNPQRSTUVWXYZ";
pub const LIST_URL: &str = "https://openwatch.party/relays.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relay {
    pub id: String,
    pub name: String,
    pub url: String,
    pub prefixes: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relays {
    pub list: Vec<Relay>,
    pub custom: bool,
}

impl Relays {
    pub fn public() -> Self {
        let embedded = serde_json::from_str(include_str!("relays.json")).ok();
        Self { list: embedded.as_ref().and_then(parse_list).unwrap_or_default(), custom: false }
    }

    pub fn custom(base: &str) -> Option<Self> {
        let base = base.trim().trim_end_matches('/');
        let (scheme, rest) = match base.strip_prefix("https://") {
            Some(rest) => ("wss://", rest),
            None => ("ws://", base.strip_prefix("http://")?),
        };
        if rest.is_empty() || rest.starts_with(['/', '?', '#']) {
            return None;
        }
        let url = format!("{scheme}{rest}/v1/ws");
        valid_url(&url, true).then(|| Self {
            list: vec![Relay { id: "custom".into(), name: base.into(), url, prefixes: ALPHABET.into() }],
            custom: true,
        })
    }

    pub fn relay_for(&self, code: &str) -> Option<&Relay> {
        let first = normalize_code(code)?.chars().next()?;
        self.list.iter().find(|r| r.prefixes.contains(first))
    }
}

pub fn normalize_code(code: &str) -> Option<String> {
    let separator = |c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{A0}' | '\u{2007}' | '\u{202F}' | '-' | '\u{2010}'..='\u{2014}' | '\u{2212}');
    let code: String = code.chars().filter(|c| !separator(*c)).map(|c| c.to_ascii_uppercase()).collect();
    valid_code(&code).then_some(code)
}

pub fn format_code(code: &str) -> Option<String> {
    normalize_code(code).map(|c| format!("{}-{}", &c[..3], &c[3..]))
}

pub fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.chars().all(|c| ALPHABET.contains(c))
}

pub fn parse_list(doc: &Value) -> Option<Vec<Relay>> {
    let entries = doc.get("relays")?.as_array().filter(|a| !a.is_empty())?;
    let mut list: Vec<Relay> = Vec::with_capacity(entries.len());
    let mut owned = String::new();
    for e in entries {
        let field = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
        let relay = Relay { id: field("id")?, name: field("name")?, url: field("url")?, prefixes: field("prefixes")? };
        let id_ok = (1..=32).contains(&relay.id.len())
            && !relay.id.starts_with('-')
            && relay.id.bytes().all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
            && list.iter().all(|r| r.id != relay.id);
        let prefixes_ok = !relay.prefixes.is_empty()
            && relay.prefixes.chars().all(|c| ALPHABET.contains(c) && !owned.contains(c) && relay.prefixes.matches(c).count() == 1);
        if !id_ok || !prefixes_ok || !valid_text(&relay.name, 64) || !valid_url(&relay.url, false) {
            return None;
        }
        owned.push_str(&relay.prefixes);
        list.push(relay);
    }
    Some(list)
}

fn valid_url(url: &str, allow_ws: bool) -> bool {
    let rest = url.strip_prefix("wss://").or_else(|| url.strip_prefix("ws://").filter(|_| allow_ws));
    rest.is_some_and(|r| !r.starts_with('/') && r.ends_with("/v1/ws") && r.bytes().all(|b| (0x21..=0x7E).contains(&b) && b != b'?' && b != b'#'))
}

pub fn fetch_list() -> Option<Vec<Relay>> {
    let config = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(5))).max_redirects(0);
    #[cfg(not(target_os = "android"))]
    let config = config.tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).root_certs(RootCerts::PlatformVerifier).build());
    let agent: ureq::Agent = config.build().into();
    let mut resp = agent.get(LIST_URL).call().ok().filter(|r| r.status() == 200)?;
    let text = resp.body_mut().with_config().limit(65536).read_to_string().ok()?;
    parse_list(&serde_json::from_str(&text).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch_together::protocol::tests::vectors;

    #[test]
    fn code_routing_vectors() {
        for case in vectors("code-routing") {
            let i = &case["input"];
            let relays = Relays {
                list: i["relays"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|r| Relay { id: r["id"].as_str().unwrap().into(), name: "x".into(), url: "wss://x/v1/ws".into(), prefixes: r["prefixes"].as_str().unwrap().into() })
                    .collect(),
                custom: false,
            };
            let code = i["code"].as_str().unwrap_or("");
            let got = match normalize_code(code) {
                None => serde_json::json!({"error": "invalid_code"}),
                Some(c) => serde_json::json!({"code": c, "relay": relays.relay_for(code).map(|r| r.id.clone())}),
            };
            assert_eq!(got, case["expect"], "{}", case["name"]);
        }
    }

    #[test]
    fn relay_list_vectors() {
        for case in vectors("relay-list") {
            let got = match parse_list(&case["input"]["list"]) {
                None => serde_json::json!({"error": "invalid_list"}),
                Some(list) => serde_json::json!({"relays": list}),
            };
            assert_eq!(got, case["expect"], "{}", case["name"]);
        }
    }

    #[test]
    fn public_list_is_embedded_and_routes_by_first_symbol() {
        let relays = Relays::public();
        assert_eq!(relays.list.len(), 3);
        assert_eq!(relays.relay_for("k7m-q2x").unwrap().id, "asia");
        assert_eq!(relays.relay_for("2ABCDE").unwrap().id, "na");
        assert!(relays.relay_for("QABCDE").is_none());
        assert_eq!(format_code("k7mq2x").as_deref(), Some("K7M-Q2X"));
    }

    #[test]
    fn custom_relay_owns_every_code() {
        let r = Relays::custom(" https://relay.example.com/owp/ ").unwrap();
        assert_eq!(r.list[0].url, "wss://relay.example.com/owp/v1/ws");
        assert_eq!(r.relay_for("QABCDE").unwrap().id, "custom");
        assert_eq!(Relays::custom("http://192.168.1.5:8080").unwrap().list[0].url, "ws://192.168.1.5:8080/v1/ws");
        assert!(Relays::custom("ftp://x").is_none());
        assert!(Relays::custom("https://").is_none());
        assert!(Relays::custom("https://a b").is_none());
    }
}
