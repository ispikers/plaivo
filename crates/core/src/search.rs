use crate::plex::{DISCOVER_ID, Item};
use std::collections::HashSet;

pub fn too_short(q: &str) -> bool {
    q.trim().chars().count() < 2
}

fn title_rank(title: &str, q: &str) -> u8 {
    let (t, q) = (title.to_lowercase(), q.to_lowercase());
    if t == q {
        0
    } else if t.starts_with(&q) {
        1
    } else if t
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| !w.is_empty() && w.starts_with(&q))
    {
        2
    } else if t.contains(&q) {
        3
    } else {
        4
    }
}

pub fn rank(
    q: &str,
    home: &str,
    local: Vec<(String, Vec<(Item, f64)>)>,
    discover: Vec<Item>,
) -> Vec<(String, Item)> {
    let mut seen = HashSet::new();
    let mut owned_guids = HashSet::new();
    let mut lib: Vec<(u8, bool, f64, String, Item)> = Vec::new();
    for (server, hits) in local {
        for (it, score) in hits {
            let it = it.as_show();
            if !matches!(it.kind.as_str(), "movie" | "show")
                || !seen.insert((server.clone(), it.rating_key.clone()))
            {
                continue;
            }
            if !it.guid.is_empty() && !owned_guids.insert(it.guid.clone()) {
                continue;
            }
            lib.push((
                title_rank(&it.title, q),
                server != home,
                -score,
                server.clone(),
                it,
            ));
        }
    }
    lib.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)));
    let owned_titles: HashSet<(String, Option<i64>)> = lib
        .iter()
        .map(|e| (e.4.title.to_lowercase(), e.4.year))
        .collect();
    let mut out: Vec<(String, Item)> = lib.into_iter().map(|e| (e.3, e.4)).collect();
    out.extend(
        discover
            .into_iter()
            .filter(|it| matches!(it.kind.as_str(), "movie" | "show"))
            .filter(|it| {
                !owned_guids.contains(&it.guid)
                    && !owned_titles.contains(&(it.title.to_lowercase(), it.year))
            })
            .map(|it| (DISCOVER_ID.to_owned(), it)),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, kind: &str, title: &str, guid: &str) -> Item {
        Item {
            rating_key: key.into(),
            kind: kind.into(),
            title: title.into(),
            guid: guid.into(),
            ..Default::default()
        }
    }

    #[test]
    fn title_matches_rank_above_incidental_hits() {
        let local = vec![(
            "home".to_owned(),
            vec![
                (item("1", "movie", "Sherlock Gnomes", "g1"), 0.99),
                (item("2", "show", "Dexter's Laboratory", "g2"), 0.5),
                (item("3", "show", "Dexter", "g3"), 0.4),
            ],
        )];
        let keys: Vec<_> = rank("dexter", "home", local, vec![])
            .into_iter()
            .map(|(_, i)| i.rating_key)
            .collect();
        assert_eq!(keys, ["3", "2", "1"]);
    }

    #[test]
    fn episodes_fold_into_one_show_and_owned_discover_hits_drop() {
        let mut ep = item("10", "episode", "Pilot", "");
        ep.grandparent_rating_key = "2".into();
        ep.grandparent_title = "Dexter's Laboratory".into();
        let local = vec![(
            "home".to_owned(),
            vec![
                (item("2", "show", "Dexter's Laboratory", "g2"), 0.5),
                (ep.clone(), 0.4),
                (ep, 0.3),
            ],
        )];
        let discover = vec![
            item("x", "show", "Dexter's Laboratory", "g2"),
            item("y", "show", "Dexter: New Blood", "g9"),
        ];
        let out = rank("dexter", "home", local, discover);
        let keys: Vec<_> = out
            .iter()
            .map(|(s, i)| format!("{s}/{}", i.rating_key))
            .collect();
        assert_eq!(keys, ["home/2", "discover/y"]);
    }

    #[test]
    fn home_server_breaks_title_ties_and_guids_dedupe_across_servers() {
        let local = vec![
            (
                "other".to_owned(),
                vec![(item("1", "movie", "Dexter", "g1"), 0.9)],
            ),
            (
                "home".to_owned(),
                vec![(item("2", "movie", "Dexter", "g2"), 0.1)],
            ),
            (
                "third".to_owned(),
                vec![(item("3", "movie", "Dexter", "g2"), 0.9)],
            ),
        ];
        let keys: Vec<_> = rank("dexter", "home", local, vec![])
            .into_iter()
            .map(|(s, i)| format!("{s}/{}", i.rating_key))
            .collect();
        assert_eq!(keys, ["home/2", "other/1"]);
    }

    #[test]
    fn word_prefix_beats_substring() {
        assert!(title_rank("The Dexter Files", "dex") < title_rank("Index", "dex"));
        assert_eq!(title_rank("Dexter", "DEXTER"), 0);
    }

    #[test]
    fn single_characters_are_too_short() {
        assert!(too_short(" a "));
        assert!(!too_short("ab"));
    }
}
