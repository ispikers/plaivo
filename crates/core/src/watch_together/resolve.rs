use super::media::{Media, MediaFile, MediaKind, NAMESPACES, PlaivoExt, Series, ids_from_guids};
use super::protocol::{Match, MatchState, forbidden};
use crate::plex::{Client, Ctx, Item, ItemKind, Server, Source};
use std::collections::{BTreeMap, HashSet};

pub const SAME_LENGTH_MS: i64 = 1500;
const CANDIDATES: usize = 5;
const PARALLEL: usize = 4;

pub fn clean(s: &str, max: usize) -> String {
    let words: Vec<String> = s
        .split_whitespace()
        .map(|w| w.chars().filter(|c| !forbidden(*c)).collect())
        .filter(|w: &String| !w.is_empty())
        .collect();
    words
        .join(" ")
        .chars()
        .take(max)
        .collect::<String>()
        .trim_end()
        .into()
}

pub fn display_name<'a>(accounts: impl IntoIterator<Item = &'a str>) -> String {
    accounts
        .into_iter()
        .map(|n| clean(n, super::NAME_MAX))
        .find(|n| !n.is_empty())
        .unwrap_or_else(|| "Plaivo".into())
}

pub fn item_ids(it: &Item) -> BTreeMap<String, String> {
    ids_from_guids(
        it.guids
            .iter()
            .map(|g| g.id.as_str())
            .chain([it.guid.as_str()]),
    )
}

fn id_match(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Option<bool> {
    let common: Vec<bool> = NAMESPACES
        .iter()
        .filter_map(|ns| Some((a.get(*ns)?, b.get(*ns)?)))
        .filter(|(x, y)| !x.is_empty() && !y.is_empty())
        .map(|(x, y)| x == y)
        .collect();
    (!common.is_empty()).then(|| common.contains(&true))
}

pub fn shares_id(ids: &BTreeMap<String, String>, it: &Item) -> Option<bool> {
    id_match(ids, &item_ids(it))
}

pub fn title_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn same_name(a: &str, b: &str) -> bool {
    let a = title_key(a);
    !a.is_empty() && a == title_key(b)
}

pub fn describe(server: &str, it: &Item, show: Option<&Item>) -> Result<Media, &'static str> {
    let kind = match it.kind {
        ItemKind::Movie => MediaKind::Movie,
        ItemKind::Episode => MediaKind::Episode,
        _ => MediaKind::Video,
    };
    let episode = kind == MediaKind::Episode;
    let index = |i: Option<i64>| i.filter(|i| *i >= 0 && episode);
    let title = Some(clean(&it.title, usize::MAX))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| index(it.index).map_or("Untitled".into(), |n| format!("Episode {n}")));
    let series = episode.then(|| Series {
        title: Some(clean(
            show.map_or(&it.grandparent_title, |s| &s.title),
            usize::MAX,
        ))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| clean(&it.grandparent_title, usize::MAX)),
        year: show.and_then(|s| s.year),
        ids: show.map(item_ids).unwrap_or_default(),
    });
    let name = it
        .media
        .first()
        .and_then(|m| m.part.first())
        .and_then(|p| p.file.rsplit(['/', '\\']).next())
        .map(|n| clean(n, usize::MAX))
        .filter(|n| !n.is_empty());
    let mut media = Media {
        kind,
        title,
        year: it.year.filter(|_| !episode),
        ids: if kind == MediaKind::Video {
            BTreeMap::new()
        } else {
            item_ids(it)
        },
        series,
        season: index(it.parent_index),
        episode: index(it.index),
        duration_ms: it.duration.unwrap_or(0) as i64,
        file: name.map(|name| MediaFile {
            name: Some(name),
            ..Default::default()
        }),
        ext: Default::default(),
    };
    media.set_plaivo(&PlaivoExt {
        server: server.into(),
        item: it.rating_key.clone(),
    });
    if media.check(true).is_err() {
        media.file = None;
    }
    media.check(true).map(|_| media)
}

pub fn match_for(ours_ms: Option<i64>, theirs_ms: i64) -> Match {
    match ours_ms.filter(|d| *d > 0) {
        Some(d) => {
            let delta = d - theirs_ms;
            Match {
                state: if delta.abs() <= SAME_LENGTH_MS {
                    MatchState::Matched
                } else {
                    MatchState::Mismatched
                },
                delta_ms: Some(delta),
            }
        }
        None => Match {
            state: MatchState::Matched,
            delta_ms: None,
        },
    }
}

struct Target<'a> {
    kind: ItemKind,
    title: &'a str,
    year: Option<i64>,
    ids: &'a BTreeMap<String, String>,
}

impl<'a> Target<'a> {
    fn of(m: &'a Media) -> Option<Self> {
        match (m.kind, &m.series) {
            (MediaKind::Movie, _) => Some(Self {
                kind: ItemKind::Movie,
                title: &m.title,
                year: m.year,
                ids: &m.ids,
            }),
            (MediaKind::Episode, Some(s)) => Some(Self {
                kind: ItemKind::Show,
                title: &s.title,
                year: s.year,
                ids: &s.ids,
            }),
            _ => None,
        }
    }

    fn matches(&self, it: &Item) -> bool {
        it.kind == self.kind
            && shares_id(self.ids, it).unwrap_or_else(|| {
                self.year
                    .zip(it.year)
                    .is_some_and(|(a, b)| (a - b).abs() <= 1)
                    && same_name(self.title, &it.title)
            })
    }
}

pub fn pick_episode<'a>(episodes: &'a [Item], m: &Media) -> Option<&'a Item> {
    episodes
        .iter()
        .find(|e| shares_id(&m.ids, e) == Some(true))
        .or_else(|| {
            let (season, episode) = (m.season?, m.episode?);
            episodes.iter().find(|e| {
                e.parent_index == Some(season)
                    && e.index == Some(episode)
                    && shares_id(&m.ids, e) != Some(false)
            })
        })
}

fn find_title(srv: &Server, cx: &Ctx, target: &Target) -> Option<Item> {
    let hits = srv
        .search(cx, target.title)
        .inspect_err(|e| log::warn!("search on {}: {e}", srv.name))
        .ok()?;
    let mut seen = HashSet::new();
    let candidates: Vec<Item> = hits
        .into_iter()
        .map(|(it, _)| it.into_show())
        .filter(|it| it.kind == target.kind && seen.insert(it.rating_key.clone()))
        .take(CANDIDATES)
        .collect();
    std::thread::scope(|s| {
        let full: Vec<_> = candidates
            .iter()
            .map(|it| s.spawn(|| srv.metadata(cx, &it.rating_key)))
            .collect();
        full.into_iter().find_map(|h| {
            h.join()
                .ok()?
                .inspect_err(|e| log::warn!("metadata on {}: {e}", srv.name))
                .ok()
                .filter(|full| target.matches(full))
        })
    })
}

fn find_on(srv: &Server, cx: &Ctx, target: &Target, media: &Media) -> Option<Item> {
    let found = find_title(srv, cx, target)?;
    match media.kind {
        MediaKind::Episode => {
            let episodes = srv
                .all_episodes(cx, &found.rating_key)
                .inspect_err(|e| log::warn!("episodes on {}: {e}", srv.name))
                .ok()?;
            pick_episode(&episodes, media).cloned()
        }
        _ => Some(found),
    }
}

pub fn find_copy(media: &Media, servers: &[Server], client: &Client) -> Option<(String, Item)> {
    let cx = Ctx::new(client.clone(), Source::Live);
    if let Some(hint) = media.plaivo()
        && let Some(srv) = servers.iter().find(|s| s.id == hint.server)
        && let Ok(it) = srv
            .metadata(&cx, &hint.item)
            .inspect_err(|e| log::warn!("metadata on {}: {e}", srv.name))
        && shares_id(&media.ids, &it) != Some(false)
    {
        return Some((srv.id.clone(), it));
    }
    let target = Target::of(media)?;
    first_on(servers, |srv| find_on(srv, &cx, &target, media))
}

pub fn find_owned(it: &Item, servers: &[Server], client: &Client) -> Option<(String, Item)> {
    let kind = match it.kind {
        ItemKind::Movie | ItemKind::Show => it.kind,
        _ => return None,
    };
    let ids = item_ids(it);
    let target = Target {
        kind,
        title: &it.title,
        year: it.year,
        ids: &ids,
    };
    let cx = Ctx::new(client.clone(), Source::Live);
    first_on(servers, |srv| find_title(srv, &cx, &target))
}

fn first_on(
    servers: &[Server],
    find: impl Fn(&Server) -> Option<Item> + Sync,
) -> Option<(String, Item)> {
    servers.chunks(PARALLEL).find_map(|chunk| {
        std::thread::scope(|s| {
            let jobs: Vec<_> = chunk
                .iter()
                .map(|srv| s.spawn(|| find(srv).map(|it| (srv.id.clone(), it))))
                .collect();
            jobs.into_iter()
                .filter_map(|job| job.join().ok().flatten())
                .next()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::{Guid, Media as PlexMedia, Part};
    use crate::watch_together::protocol::tests::{assert_none_failed, vectors};

    #[test]
    fn display_name_takes_the_first_usable_account() {
        assert_eq!(display_name(["  Alex  ", "jf"]), "Alex");
        assert_eq!(display_name(["\u{200E}", "jf"]), "jf");
        assert_eq!(display_name([]), "Plaivo");
        assert_eq!(
            display_name([&*"x".repeat(40)]).len(),
            super::super::NAME_MAX
        );
    }

    fn guids(ids: &[&str]) -> Vec<Guid> {
        ids.iter().map(|id| Guid { id: (*id).into() }).collect()
    }

    fn movie() -> Item {
        Item {
            rating_key: "42".into(),
            kind: ItemKind::Movie,
            title: "Arrival".into(),
            guid: "plex://movie/5d776".into(),
            year: Some(2016),
            duration: Some(6_960_000),
            guids: guids(&["imdb://tt2543164", "tmdb://329865"]),
            media: vec![PlexMedia {
                part: vec![Part {
                    file: "/data/movies/Arrival (2016).mkv".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn episode(season: i64, index: i64, key: &str) -> Item {
        Item {
            rating_key: key.into(),
            kind: ItemKind::Episode,
            title: format!("Episode {index}"),
            grandparent_title: "Severance".into(),
            parent_index: Some(season),
            index: Some(index),
            duration: Some(3_000_000),
            ..Default::default()
        }
    }

    fn ids(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn movies_describe_with_ids_primary_guid_file_name_and_hint() {
        let m = describe("srv", &movie(), None).unwrap();
        assert_eq!(
            (m.kind, m.title.as_str(), m.year, m.duration_ms),
            (MediaKind::Movie, "Arrival", Some(2016), 6_960_000)
        );
        assert_eq!(
            (m.ids["imdb"].as_str(), m.ids["plex"].as_str()),
            ("tt2543164", "plex://movie/5d776")
        );
        assert_eq!(
            m.file.as_ref().and_then(|f| f.name.as_deref()),
            Some("Arrival (2016).mkv")
        );
        assert_eq!(
            m.plaivo(),
            Some(PlaivoExt {
                server: "srv".into(),
                item: "42".into()
            })
        );
        assert!(
            describe(
                "srv",
                &Item {
                    duration: None,
                    ..movie()
                },
                None
            )
            .is_err()
        );
    }

    #[test]
    fn episodes_carry_their_show_numbers_and_title_fallbacks() {
        let mut ep = episode(1, 3, "7");
        ep.title = " \u{200E} ".into();
        let show = Item {
            title: "Severance".into(),
            year: Some(2022),
            guids: guids(&["tvdb://371980"]),
            ..Default::default()
        };
        let m = describe("srv", &ep, Some(&show)).unwrap();
        assert_eq!(
            (m.title.as_str(), m.year, m.season, m.episode),
            ("Episode 3", None, Some(1), Some(3))
        );
        let series = m.series.unwrap();
        assert_eq!(
            (
                series.title.as_str(),
                series.year,
                series.ids["tvdb"].as_str()
            ),
            ("Severance", Some(2022), "371980")
        );
        let bare = describe("srv", &ep, Some(&Item::default()))
            .unwrap()
            .series
            .unwrap();
        assert_eq!((bare.title.as_str(), bare.ids.len()), ("Severance", 0));
    }

    #[test]
    fn candidates_match_by_ids_and_by_title_and_year_only_without_overlap() {
        let m = describe("srv", &movie(), None).unwrap();
        let t = Target::of(&m).unwrap();
        assert!(t.matches(&movie()));
        assert!(t.matches(&Item {
            guids: guids(&["tmdb://329865"]),
            guid: String::new(),
            title: "Other".into(),
            ..movie()
        }));
        assert!(!t.matches(&Item {
            guids: guids(&["imdb://tt0000001"]),
            guid: String::new(),
            ..movie()
        }));
        assert!(t.matches(&Item {
            guids: vec![],
            guid: String::new(),
            title: "ARRIVAL".into(),
            ..movie()
        }));
        assert!(t.matches(&Item {
            guids: vec![],
            guid: String::new(),
            year: Some(2017),
            ..movie()
        }));
        assert!(!t.matches(&Item {
            guids: vec![],
            guid: String::new(),
            year: Some(2018),
            ..movie()
        }));
        assert!(!t.matches(&Item {
            kind: ItemKind::Show,
            ..movie()
        }));
        assert_eq!(shares_id(&ids(&[]), &movie()), None);
        assert_eq!(
            shares_id(&ids(&[("plex", "plex://movie/5d776")]), &movie()),
            Some(true)
        );
    }

    #[test]
    fn episodes_are_picked_by_ids_then_numbers() {
        let mut tagged = episode(2, 1, "b");
        tagged.guids = guids(&["tvdb://999"]);
        let episodes = [episode(1, 3, "a"), tagged];
        let mut m = describe("srv", &episode(1, 3, "x"), None).unwrap();
        assert_eq!(
            pick_episode(&episodes, &m).map(|e| e.rating_key.as_str()),
            Some("a")
        );
        m.ids = ids(&[("tvdb", "999")]);
        assert_eq!(
            pick_episode(&episodes, &m).map(|e| e.rating_key.as_str()),
            Some("b")
        );
        let mut numbered = episode(2, 1, "c");
        numbered.guids = guids(&["tvdb://111"]);
        m.ids = ids(&[("tvdb", "222")]);
        m.season = Some(2);
        m.episode = Some(1);
        assert!(pick_episode(&[numbered], &m).is_none());
        m.ids.clear();
        m.episode = Some(9);
        assert!(pick_episode(&episodes, &m).is_none());
    }

    #[test]
    fn lengths_within_a_second_and_a_half_count_as_the_same_file() {
        assert_eq!(
            match_for(Some(6_961_000), 6_960_000),
            Match {
                state: MatchState::Matched,
                delta_ms: Some(1000)
            }
        );
        assert_eq!(
            match_for(Some(6_900_000), 6_960_000).state,
            MatchState::Mismatched
        );
        assert_eq!(
            match_for(None, 6_960_000),
            Match {
                state: MatchState::Matched,
                delta_ms: None
            }
        );
    }

    fn same_title(a: &Media, b: &Media) -> bool {
        if a.kind != b.kind {
            return false;
        }
        match a.kind {
            MediaKind::Movie => id_match(&a.ids, &b.ids) == Some(true),
            MediaKind::Episode => id_match(&a.ids, &b.ids).unwrap_or_else(|| {
                matches!((&a.series, &b.series), (Some(x), Some(y)) if id_match(&x.ids, &y.ids) == Some(true))
                    && a.season.is_some()
                    && a.season == b.season
                    && a.episode.is_some()
                    && a.episode == b.episode
            }),
            MediaKind::Video => match (&a.file, &b.file) {
                (Some(x), Some(y)) => same_file(x, y),
                _ => false,
            },
        }
    }

    fn same_file(a: &MediaFile, b: &MediaFile) -> bool {
        let eq = |x: &Option<String>, y: &Option<String>| {
            x.as_ref()
                .zip(y.as_ref())
                .is_some_and(|(x, y)| x.eq_ignore_ascii_case(y))
        };
        eq(&a.oshash, &b.oshash)
            || (a.size_bytes.is_some() && a.size_bytes == b.size_bytes && eq(&a.name, &b.name))
    }

    #[test]
    fn same_title_vectors() {
        let cases = vectors("same-title");
        let mut failures = Vec::new();
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            let a = Media::parse(case["input"]["a"].clone()).unwrap();
            let b = Media::parse(case["input"]["b"].clone()).unwrap();
            let want = case["expect"]["same"].as_bool().unwrap();
            for (label, x, y) in [("", &a, &b), (" (swapped)", &b, &a)] {
                let got = same_title(x, y);
                if got != want {
                    failures.push(format!("{name}{label}: same want {want}, got {got}"));
                }
            }
        }
        assert_none_failed("same-title", cases.len(), &failures);
    }

    #[test]
    fn text_is_cleaned_to_protocol_text() {
        assert_eq!(clean("a\tb\n\u{202E}c", 99), "a b c");
        assert_eq!(clean("abcdef", 3), "abc");
        assert_eq!(clean("\u{200E} \u{200F}", 99), "");
        assert_eq!(clean("\u{200E} Alex", 99), "Alex");
        assert_eq!(clean("ab cd", 3), "ab");
        assert!(same_name(
            "Spider-Man: No Way Home",
            "spider man no way home"
        ));
        assert!(!same_name("", ""));
    }
}
