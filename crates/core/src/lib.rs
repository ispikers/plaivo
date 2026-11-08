pub mod plex;
pub mod settings;
pub mod store;
pub mod trackers;
pub mod watch_together;

use plex::Server;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct Account {
    pub client_id: String,
    pub token: Option<String>,
    #[serde(default)]
    pub servers: Vec<Server>,
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
}

pub fn first<'a>(options: &[&'a str]) -> &'a str {
    options.iter().copied().find(|s| !s.is_empty()).unwrap_or("")
}

pub fn join(parts: &[String]) -> String {
    parts.iter().filter(|p| !p.is_empty()).cloned().collect::<Vec<_>>().join(" · ")
}
