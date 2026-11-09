use crate::join;
use crate::plex::{self, Item};

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
    let (mut play_key, mut play_label, progress) = match (it.kind.as_str(), on_deck) {
        ("show", Some(ep)) => (ep.rating_key.clone(), format!("{} S{}E{}", resume(ep.progress()), ep.parent_index.unwrap_or(0), ep.index.unwrap_or(0)), ep.progress()),
        ("show", None) => (it.rating_key.clone(), "Play".into(), 0.),
        _ => (it.rating_key.clone(), resume(it.progress()).into(), it.progress()),
    };
    let mut chips = vec![
        if it.kind == "episode" { it.originally_available_at.clone() } else { it.year.map(|y| y.to_string()).unwrap_or_default() },
        it.content_rating.clone(),
        it.duration.filter(|_| it.kind != "show").map(plex::fmt_duration).unwrap_or_default(),
        it.child_count.filter(|_| it.kind == "show").map(|n| format!("{n} season{}", if n == 1 { "" } else { "s" })).unwrap_or_default(),
        media.map(|m| plex::fmt_resolution(&m.video_resolution)).unwrap_or_default(),
        media.map(|m| m.video_dynamic_range.clone()).filter(|r| r != "SDR").unwrap_or_default(),
        it.audio_label(),
    ];
    if !in_library {
        chips.insert(0, "Not in your library".into());
        play_key.clear();
        play_label.clear();
    }
    chips.retain(|c| !c.is_empty());
    let names = |tags: &[plex::Tag]| tags.iter().take(3).map(|t| t.tag.clone()).collect::<Vec<_>>().join(", ");
    let info = [
        ("Studio", it.studio.clone()),
        (if it.director.len() > 1 { "Directors" } else { "Director" }, names(&it.director)),
        (if it.writer.len() > 1 { "Writers" } else { "Writer" }, names(&it.writer)),
        ("Released", if it.kind == "episode" { String::new() } else { it.originally_available_at.clone() }),
    ];
    Summary {
        play_key,
        play_label,
        progress,
        eyebrow: if it.kind == "episode" { join(&[it.grandparent_title.clone(), it.episode_label()]) } else { String::new() },
        chips,
        info: info.into_iter().filter(|(_, v)| !v.is_empty()).collect(),
    }
}

pub fn pick_season(show: &Item, seasons: &[Item], wanted: Option<&str>) -> Option<String> {
    let valid = |k: &str| seasons.iter().any(|s| s.rating_key == k);
    let on_deck = show.on_deck.as_ref().and_then(|d| d.metadata.as_ref()).map(|e| e.parent_rating_key.as_str());
    wanted
        .filter(|k| valid(k))
        .or(on_deck.filter(|k| valid(k)))
        .map(str::to_owned)
        .or_else(|| seasons.iter().find(|s| s.index != Some(0)).or(seasons.first()).map(|s| s.rating_key.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::OnDeck;

    fn season(key: &str, index: i64) -> Item {
        Item { rating_key: key.into(), kind: "season".into(), index: Some(index), ..Default::default() }
    }

    #[test]
    fn shows_resume_their_on_deck_episode() {
        let ep = Item { rating_key: "e5".into(), parent_rating_key: "s2".into(), parent_index: Some(2), index: Some(5), view_offset: Some(60_000), duration: Some(120_000), ..Default::default() };
        let show = Item { rating_key: "show".into(), kind: "show".into(), year: Some(2022), child_count: Some(1), on_deck: Some(Box::new(OnDeck { metadata: Some(ep) })), ..Default::default() };
        let s = summary(&show, true);
        assert_eq!((s.play_key.as_str(), s.play_label.as_str()), ("e5", "Resume S2E5"));
        assert_eq!(s.chips, ["2022", "1 season"]);
        let seasons = [season("s0", 0), season("s1", 1), season("s2", 2)];
        assert_eq!(pick_season(&show, &seasons, None).as_deref(), Some("s2"));
        assert_eq!(pick_season(&show, &seasons, Some("s1")).as_deref(), Some("s1"));
        assert_eq!(pick_season(&show, &seasons, Some("gone")).as_deref(), Some("s2"));
        let fresh = Item { kind: "show".into(), ..Default::default() };
        assert_eq!(pick_season(&fresh, &seasons, None).as_deref(), Some("s1"));
    }

    #[test]
    fn titles_outside_the_library_cannot_play() {
        let it = Item { rating_key: "m".into(), kind: "movie".into(), studio: "A24".into(), ..Default::default() };
        let s = summary(&it, false);
        assert!(s.play_key.is_empty() && s.play_label.is_empty());
        assert_eq!(s.chips, ["Not in your library"]);
        assert_eq!(s.info, [("Studio", "A24".to_owned())]);
    }
}
