use super::protocol::{MAX_DURATION_MS, MAX_INT, normalize_ints, valid_text};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const PLAIVO_EXT: &str = "app.plaivo";
const NAMESPACES: [&str; 8] = [
    "imdb", "tmdb", "tvdb", "plex", "anilist", "anidb", "kitsu", "mal",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Movie,
    Episode,
    Video,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    pub kind: MediaKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ids: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<Series>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<i64>,
    pub duration_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<MediaFile>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub ext: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ids: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oshash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaivoExt {
    pub server: String,
    pub item: String,
}

impl Media {
    pub fn parse(mut v: Value) -> Option<Self> {
        normalize_ints(&mut v);
        serde_json::from_value::<Media>(v)
            .ok()
            .filter(|m| m.check(false).is_ok())
    }

    pub fn check(&self, sending: bool) -> Result<(), &'static str> {
        let text = |s: &str| valid_text(s, usize::MAX);
        let ids = |ids: &BTreeMap<String, String>| ids.values().all(|v| !v.is_empty());
        let count = |v: Option<i64>| v.is_none_or(|v| (0..=MAX_INT).contains(&v));
        let year = |v: Option<i64>| v.is_none_or(|v| (-MAX_INT..=MAX_INT).contains(&v));
        if !text(&self.title) {
            return Err("title");
        }
        if !year(self.year) || !ids(&self.ids) {
            return Err("ids");
        }
        match &self.series {
            None if self.kind == MediaKind::Episode => return Err("series"),
            Some(s) if !text(&s.title) || !year(s.year) || !ids(&s.ids) => return Err("series"),
            _ => {}
        }
        if !count(self.season) || !count(self.episode) {
            return Err("season");
        }
        if !(1..=MAX_DURATION_MS).contains(&self.duration_ms) {
            return Err("durationMs");
        }
        if let Some(f) = &self.file {
            let oshash = f.oshash.as_deref().is_none_or(|h| {
                h.len() == 16 && h.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            });
            let name = f
                .name
                .as_deref()
                .is_none_or(|n| text(n) && !n.contains(['/', '\\']) && n != "." && n != "..");
            if !count(f.size_bytes) || !oshash || !name {
                return Err("file");
            }
        }
        let ext_name = |n: &str| {
            n.contains('.')
                && n.split('.').all(|l| {
                    !l.is_empty()
                        && l.bytes()
                            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
                })
        };
        if !self.ext.keys().all(|k| ext_name(k)) {
            return Err("ext");
        }
        if sending {
            if serde_json::to_string(self).map_or(true, |s| s.len() > 4096) {
                return Err("size");
            }
            if serde_json::to_string(&self.ext).map_or(true, |s| s.len() > 1024) {
                return Err("ext size");
            }
        }
        Ok(())
    }

    pub fn plaivo(&self) -> Option<PlaivoExt> {
        serde_json::from_value::<PlaivoExt>(self.ext.get(PLAIVO_EXT)?.clone())
            .ok()
            .filter(|h| safe_key(&h.item))
    }

    pub fn set_plaivo(&mut self, ext: &PlaivoExt) {
        self.ext.insert(
            PLAIVO_EXT.into(),
            serde_json::to_value(ext).unwrap_or_default(),
        );
    }
}

fn safe_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub fn ids_from_guids<'a>(guids: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, String> {
    guids
        .into_iter()
        .filter_map(|guid| {
            let (ns, id) = guid.split_once("://")?;
            let ns = match ns.to_ascii_lowercase() {
                ns if ns == "myanimelist" => "mal".to_owned(),
                ns => ns,
            };
            let value = if ns == "plex" { guid } else { id };
            (NAMESPACES.contains(&ns.as_str()) && !id.is_empty()).then(|| (ns, value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie() -> Media {
        Media {
            kind: MediaKind::Movie,
            title: "Arrival".into(),
            year: Some(2016),
            ids: ids_from_guids([
                "imdb://tt2543164",
                "tmdb://329865",
                "tvdb://123",
                "plex://movie/5d776",
                "local://9",
            ]),
            series: None,
            season: None,
            episode: None,
            duration_ms: 6_960_000,
            file: Some(MediaFile {
                size_bytes: Some(1 << 31),
                oshash: None,
                name: Some("Arrival.mkv".into()),
            }),
            ext: Map::new(),
        }
    }

    #[test]
    fn plex_guids_map_to_namespaces() {
        let ids = movie().ids;
        assert_eq!(ids["imdb"], "tt2543164");
        assert_eq!(ids["tmdb"], "329865");
        assert_eq!(ids["plex"], "plex://movie/5d776");
        assert!(!ids.contains_key("local"));
    }

    #[test]
    fn descriptor_round_trips_with_plaivo_hint() {
        let mut m = movie();
        m.set_plaivo(&PlaivoExt {
            server: "abc123".into(),
            item: "4711".into(),
        });
        let json = serde_json::to_string(&m).unwrap();
        assert_eq!(
            serde_json::to_value(&m).unwrap()["ext"],
            serde_json::json!({"app.plaivo": {"server": "abc123", "item": "4711"}})
        );
        assert!(json.contains(r#""durationMs":6960000"#));
        let back = Media::parse(serde_json::from_str(&json).unwrap()).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.plaivo().unwrap().item, "4711");
        assert_eq!(m.check(true), Ok(()));
    }

    #[test]
    fn peer_item_hints_must_be_plain_ids() {
        assert!(safe_key("4711") && safe_key("f137a2dd-21bb-c1b9-9aa5-c0f6bf02a805"));
        for bad in [
            "",
            "../users",
            "1?X-Plex-Token=x",
            "1/children",
            "1%2F",
            "1 2",
        ] {
            assert!(!safe_key(bad), "{bad}");
        }
        let mut m = movie();
        m.set_plaivo(&PlaivoExt {
            server: "abc123".into(),
            item: "1/../../accounts".into(),
        });
        assert_eq!(m.plaivo(), None);
    }

    #[test]
    fn rejects_bad_descriptors() {
        let mut m = movie();
        m.kind = MediaKind::Episode;
        assert_eq!(m.check(true), Err("series"));
        let mut m = movie();
        m.duration_ms = 0;
        assert_eq!(m.check(true), Err("durationMs"));
        let mut m = movie();
        m.file.as_mut().unwrap().name = Some("../x".into());
        assert_eq!(m.check(true), Err("file"));
        let mut m = movie();
        m.ext.insert("Plaivo".into(), Value::Null);
        assert_eq!(m.check(true), Err("ext"));
        let mut m = movie();
        m.ext
            .insert("app.big".into(), Value::String("x".repeat(1100)));
        assert_eq!(m.check(false), Ok(()));
        assert_eq!(m.check(true), Err("ext size"));
        assert!(
            Media::parse(serde_json::json!({"kind": "show", "title": "x", "durationMs": 1}))
                .is_none()
        );
        assert!(
            Media::parse(
                serde_json::json!({"kind": "video", "title": "x", "durationMs": 1.0, "extra": 1})
            )
            .is_some()
        );
    }
}
