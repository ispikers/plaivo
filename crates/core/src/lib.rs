pub mod detail;
pub mod downloads;
pub mod jellyfin;
pub mod menu;
pub mod plex;
pub mod prefs;
pub mod settings;
pub mod store;
pub mod trackers;
pub mod watch_together;

use plex::Server;
use serde::{Deserialize, Serialize};
use settings::Settings;

#[derive(Serialize, Deserialize, Default)]
pub struct Account {
    pub client_id: String,
    pub token: Option<String>,
    #[serde(default)]
    pub servers: Vec<Server>,
    #[serde(default)]
    pub jellyfin: Vec<Server>,
    #[serde(default, skip_serializing)]
    server: Option<Server>,
}

impl Account {
    fn path() -> std::path::PathBuf {
        store::config_dir().join("config.json")
    }

    pub fn load() -> Self {
        let mut a: Account = store::read_json(&Self::path()).unwrap_or_default();
        if let Some(legacy) = a.server.take()
            && a.servers.is_empty()
        {
            a.servers.push(legacy);
        }
        if a.client_id.is_empty() {
            use std::hash::{BuildHasher, RandomState};
            let rs = RandomState::new();
            a.client_id = format!("{:016x}{:016x}", rs.hash_one(1u8), rs.hash_one(2u8));
            a.save();
        }
        a
    }

    pub fn save(&self) {
        store::write_json(&Self::path(), self);
    }

    pub fn all_servers(&self) -> Vec<Server> {
        self.servers.iter().chain(&self.jellyfin).cloned().collect()
    }

    pub fn server(&self, id: &str) -> Option<&Server> {
        self.servers
            .iter()
            .chain(&self.jellyfin)
            .find(|s| s.id == id)
    }

    pub fn add_jellyfin(&mut self, srv: Server) {
        match self.jellyfin.iter_mut().find(|s| s.id == srv.id) {
            Some(old) => *old = srv,
            None => self.jellyfin.push(srv),
        }
    }

    pub fn has_servers(&self) -> bool {
        self.token.is_some() || !self.jellyfin.is_empty()
    }
}

pub const DISK_CACHE_BYTES: u64 = 768 << 20;

pub fn sign_out(account: &mut Account, settings: &mut Settings) {
    account.token = None;
    account.servers.clear();
    account.save();
    prefs::forget();
    settings.home_server.clear();
    settings.recent_searches.clear();
    settings.save();
    std::thread::spawn(store::clear_cache);
}

#[cfg(not(target_os = "android"))]
pub(crate) fn http() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder().tls_config(
        TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::PlatformVerifier)
            .build(),
    )
}

#[cfg(target_os = "android")]
pub(crate) fn http() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    ureq::Agent::config_builder()
}

pub fn first<'a>(options: &[&'a str]) -> &'a str {
    options
        .iter()
        .copied()
        .find(|s| !s.is_empty())
        .unwrap_or("")
}

pub fn redact(line: &str) -> String {
    ["X-Plex-Token", "ApiKey", "Token"]
        .iter()
        .fold(line.to_owned(), |l, key| redact_key(&l, key))
}

fn redact_key(line: &str, key: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(i) = rest.find(key) {
        let after = &rest[i + key.len()..];
        let Some(sep) = after.chars().next().filter(|c| *c == '=' || *c == ':') else {
            out.push_str(&rest[..i + key.len()]);
            rest = after;
            continue;
        };
        let value = after[1..].trim_start();
        let (quote, value) = match value.strip_prefix('"') {
            Some(v) => ("\"", v),
            None => ("", value),
        };
        let end = value
            .find(|c: char| c == '&' || c == '"' || c == '\'' || c == ',' || c.is_whitespace())
            .unwrap_or(value.len());
        out.push_str(&rest[..i + key.len()]);
        out.push(sep);
        out.push_str(quote);
        out.push_str("<redacted>");
        rest = &value[end..];
    }
    out.push_str(rest);
    out
}

pub fn join(parts: &[String]) -> String {
    parts
        .iter()
        .filter(|p| !p.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use plex::Backend;

    #[test]
    fn old_config_loads_and_jellyfin_servers_round_trip() {
        let old = r#"{"client_id":"abc","token":"t","servers":[{"name":"Home","id":"p1","uri":"https://1-2-3-4.x.plex.direct:32400","token":"pt","owned":true}]}"#;
        let mut a: Account = serde_json::from_str(old).unwrap();
        assert!(a.jellyfin.is_empty());
        assert_eq!(
            (a.servers[0].backend, a.servers[0].user_id.as_str()),
            (Backend::Plex, "")
        );
        a.jellyfin.push(Server {
            name: "nas".into(),
            id: "j1".into(),
            uri: "http://nas:8096".into(),
            token: "jt".into(),
            backend: Backend::Jellyfin,
            user_id: "u1".into(),
            ..Default::default()
        });
        let back: Account = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
        assert_eq!(back.servers, a.servers);
        assert_eq!(back.jellyfin, a.jellyfin);
        assert_eq!(back.client_id, "abc");
    }

    #[test]
    fn tokens_never_reach_the_log() {
        assert_eq!(
            redact("GET http://h/a?X-Plex-Token=abc123&b=1 ok"),
            "GET http://h/a?X-Plex-Token=<redacted>&b=1 ok"
        );
        assert_eq!(
            redact("hdr X-Plex-Token: s3cr3t done"),
            "hdr X-Plex-Token:<redacted> done"
        );
        assert_eq!(
            redact("Playing: http://nas:8096/Videos/i/stream?Static=true&ApiKey=0b5d8e2f done"),
            "Playing: http://nas:8096/Videos/i/stream?Static=true&ApiKey=<redacted> done"
        );
        assert_eq!(
            redact(r#"Authorization: MediaBrowser Client="Plaivo", Token="0b5d8e2f", x"#),
            r#"Authorization: MediaBrowser Client="Plaivo", Token="<redacted>", x"#
        );
        assert_eq!(redact("no secrets here"), "no secrets here");
    }

    #[test]
    fn jellyfin_servers_merge_after_plex_and_re_add_replaces() {
        let jf = |id: &str, token: &str| Server {
            id: id.into(),
            token: token.into(),
            backend: Backend::Jellyfin,
            ..Default::default()
        };
        let mut a = Account {
            servers: vec![Server {
                id: "p1".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(!a.has_servers());
        a.add_jellyfin(jf("j1", "old"));
        a.add_jellyfin(jf("j2", "x"));
        a.add_jellyfin(jf("j1", "new"));
        assert!(a.has_servers());
        assert_eq!(
            a.all_servers()
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["p1", "j1", "j2"]
        );
        assert_eq!(a.server("j1").map(|s| s.token.as_str()), Some("new"));
        assert_eq!(a.server("p1").map(|s| s.backend), Some(Backend::Plex));
        assert!(a.server("nope").is_none());
    }
}
