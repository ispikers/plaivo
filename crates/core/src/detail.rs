use crate::join;
use crate::plex::{self, Item, ItemKind};

pub const NOT_IN_LIBRARY: &str = "Not in your library";

pub struct Summary {
    pub play_key: String,
    pub play_label: String,
    pub progress: f32,
    pub eyebrow: String,
    pub chips: Vec<String>,
    pub info: Vec<(&'static str, String)>,
}

pub fn summary(it: &Item, in_library: bool) -> Summary {
    let media = it.media.first();
    let on_deck = it.on_deck.as_ref().and_then(|d| d.metadata.as_ref());
    let resume = |p: f32| if p > 0. { "Resume" } else { "Play" };
    let (mut play_key, mut play_label, progress) = match (it.kind, on_deck) {
        (ItemKind::Show, Some(ep)) => (
            ep.rating_key.clone(),
            format!(
                "{} S{}E{}",
                resume(ep.progress()),
                ep.parent_index.unwrap_or(0),
                ep.index.unwrap_or(0)
            ),
            ep.progress(),
        ),
        (ItemKind::Show, None) => (it.rating_key.clone(), "Play".into(), 0.),
        _ => (
            it.rating_key.clone(),
            resume(it.progress()).into(),
            it.progress(),
        ),
    };
    let mut chips = vec![
        if it.kind == ItemKind::Episode {
            it.originally_available_at.clone()
        } else {
            it.year.map(|y| y.to_string()).unwrap_or_default()
        },
        it.content_rating.clone(),
        it.duration
            .filter(|_| it.kind != ItemKind::Show)
            .map(plex::fmt_duration)
            .unwrap_or_default(),
        it.child_count
            .filter(|_| it.kind == ItemKind::Show)
            .map(|n| format!("{n} season{}", if n == 1 { "" } else { "s" }))
            .unwrap_or_default(),
        media
            .map(|m| plex::fmt_resolution(&m.video_resolution))
            .unwrap_or_default(),
        media
            .map(|m| m.video_dynamic_range.clone())
            .filter(|r| r != "SDR")
            .unwrap_or_default(),
        it.audio_label(),
    ];
    if !in_library {
        chips.insert(0, NOT_IN_LIBRARY.into());
        play_key.clear();
        play_label.clear();
    }
    chips.retain(|c| !c.is_empty());
    let names = |tags: &[plex::Tag]| {
        tags.iter()
            .take(3)
            .map(|t| t.tag.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let info = [
        ("Studio", it.studio.clone()),
        (
            if it.director.len() > 1 {
                "Directors"
            } else {
                "Director"
            },
            names(&it.director),
        ),
        (
            if it.writer.len() > 1 {
                "Writers"
            } else {
                "Writer"
            },
            names(&it.writer),
        ),
        (
            "Released",
            if it.kind == ItemKind::Episode {
                String::new()
            } else {
                it.originally_available_at.clone()
            },
        ),
        (
            "Status",
            match (
                it.is_continuing_series,
                it.next_episode_originally_available_at.is_empty(),
            ) {
                (_, false) => "Airing".into(),
                (Some(true), true) => "Continuing".into(),
                _ => String::new(),
            },
        ),
        (
            "Next episode",
            it.next_episode_originally_available_at.clone(),
        ),
    ];
    Summary {
        play_key,
        play_label,
        progress,
        eyebrow: if it.kind == ItemKind::Episode {
            join(&[it.grandparent_title.clone(), it.episode_label()])
        } else {
            String::new()
        },
        chips,
        info: info.into_iter().filter(|(_, v)| !v.is_empty()).collect(),
    }
}

pub fn merge_cast<T: Clone>(own: &[T], show: &[T], name: fn(&T) -> &str) -> Vec<T> {
    let mut seen: std::collections::HashSet<&str> = own.iter().map(name).collect();
    own.iter()
        .chain(show.iter().filter(|p| seen.insert(name(p))))
        .cloned()
        .collect()
}

pub fn pick_season(show: &Item, seasons: &[Item], wanted: Option<&str>) -> Option<String> {
    let valid = |k: &str| seasons.iter().any(|s| s.rating_key == k);
    let on_deck = show
        .on_deck
        .as_ref()
        .and_then(|d| d.metadata.as_ref())
        .map(|e| e.parent_rating_key.as_str());
    wanted
        .filter(|k| valid(k))
        .or(on_deck.filter(|k| valid(k)))
        .map(str::to_owned)
        .or_else(|| {
            seasons
                .iter()
                .find(|s| s.index != Some(0))
                .or(seasons.first())
                .map(|s| s.rating_key.clone())
        })
}

pub fn neighbours(episodes: &[Item], key: &str) -> [Option<String>; 2] {
    let Some(i) = episodes.iter().position(|e| e.rating_key == key) else {
        return [None, None];
    };
    let special = episodes[i].parent_index == Some(0);
    let same = |e: &&Item| (e.parent_index == Some(0)) == special;
    [
        episodes[..i].iter().rev().find(same),
        episodes[i + 1..].iter().find(same),
    ]
    .map(|e| e.map(|e| e.rating_key.clone()))
}

pub fn all_neighbours(episodes: &[Item]) -> Vec<(String, [Option<String>; 2])> {
    let mut out = Vec::with_capacity(episodes.len());
    for special in [false, true] {
        let keys: Vec<&str> = episodes
            .iter()
            .filter(|e| (e.parent_index == Some(0)) == special)
            .map(|e| e.rating_key.as_str())
            .collect();
        for (i, k) in keys.iter().enumerate() {
            let at = |j: Option<usize>| j.and_then(|j| keys.get(j)).map(|k| (*k).to_owned());
            out.push(((*k).to_owned(), [at(i.checked_sub(1)), at(Some(i + 1))]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::OnDeck;

    #[test]
    fn episode_cast_comes_first_and_the_show_fills_in_the_rest() {
        let merged = merge_cast(&["Guest", "Lead"], &["Lead", "Second", "Guest"], |s| *s);
        assert_eq!(merged, ["Guest", "Lead", "Second"]);
        assert_eq!(merge_cast(&[], &["Lead"], |s| *s), ["Lead"]);
    }

    fn season(key: &str, index: i64) -> Item {
        Item {
            rating_key: key.into(),
            kind: ItemKind::Season,
            index: Some(index),
            ..Default::default()
        }
    }

    #[test]
    fn shows_resume_their_on_deck_episode() {
        let ep = Item {
            rating_key: "e5".into(),
            parent_rating_key: "s2".into(),
            parent_index: Some(2),
            index: Some(5),
            view_offset: Some(60_000),
            duration: Some(120_000),
            ..Default::default()
        };
        let show = Item {
            rating_key: "show".into(),
            kind: ItemKind::Show,
            year: Some(2022),
            child_count: Some(1),
            on_deck: Some(Box::new(OnDeck { metadata: Some(ep) })),
            ..Default::default()
        };
        let s = summary(&show, true);
        assert_eq!(
            (s.play_key.as_str(), s.play_label.as_str()),
            ("e5", "Resume S2E5")
        );
        assert_eq!(s.chips, ["2022", "1 season"]);
        let seasons = [season("s0", 0), season("s1", 1), season("s2", 2)];
        assert_eq!(pick_season(&show, &seasons, None).as_deref(), Some("s2"));
        assert_eq!(
            pick_season(&show, &seasons, Some("s1")).as_deref(),
            Some("s1")
        );
        assert_eq!(
            pick_season(&show, &seasons, Some("gone")).as_deref(),
            Some("s2")
        );
        let fresh = Item {
            kind: ItemKind::Show,
            ..Default::default()
        };
        assert_eq!(pick_season(&fresh, &seasons, None).as_deref(), Some("s1"));
    }

    #[test]
    fn titles_outside_the_library_cannot_play() {
        let it = Item {
            rating_key: "m".into(),
            kind: ItemKind::Movie,
            studio: "A24".into(),
            ..Default::default()
        };
        let s = summary(&it, false);
        assert!(s.play_key.is_empty() && s.play_label.is_empty());
        assert_eq!(s.chips, [NOT_IN_LIBRARY]);
        assert_eq!(s.info, [("Studio", "A24".to_owned())]);
    }

    #[test]
    fn episodes_step_across_seasons_but_not_into_specials() {
        let ep = |key: &str, season| Item {
            rating_key: key.into(),
            parent_index: Some(season),
            ..Default::default()
        };
        let eps = [
            ep("sp1", 0),
            ep("a1", 1),
            ep("a2", 1),
            ep("b1", 2),
            ep("sp2", 0),
        ];
        let keys = |k| neighbours(&eps, k).map(|n| n.unwrap_or_default());
        assert_eq!(keys("a1"), ["", "a2"]);
        assert_eq!(keys("a2"), ["a1", "b1"]);
        assert_eq!(keys("b1"), ["a2", ""]);
        assert_eq!(keys("sp1"), ["", "sp2"]);
        assert_eq!(keys("gone"), ["", ""]);
        let all = all_neighbours(&eps);
        assert_eq!(all.len(), eps.len());
        for (key, pair) in all {
            assert_eq!(pair, neighbours(&eps, &key), "{key}");
        }
    }

    #[test]
    fn shows_say_whether_they_are_airing() {
        let status = |continuing, next: &str| {
            let it = Item {
                kind: ItemKind::Show,
                is_continuing_series: continuing,
                next_episode_originally_available_at: next.into(),
                ..Default::default()
            };
            summary(&it, true).info
        };
        assert_eq!(
            status(Some(true), "2026-11-22"),
            [
                ("Status", "Airing".to_owned()),
                ("Next episode", "2026-11-22".to_owned())
            ]
        );
        assert_eq!(
            status(Some(true), ""),
            [("Status", "Continuing".to_owned())]
        );
        assert!(status(None, "").is_empty());
        let it: Item = serde_json::from_str(
            r#"{"isContinuingSeries":true,"nextEpisodeOriginallyAvailableAt":"2026-11-22"}"#,
        )
        .unwrap();
        assert_eq!(
            (
                it.is_continuing_series,
                it.next_episode_originally_available_at.as_str()
            ),
            (Some(true), "2026-11-22")
        );
    }
}
