use crate::join;
use crate::plex::{Item, Media, fmt_bitrate, fmt_resolution};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    FromStart,
    MarkWatched,
    MarkUnwatched,
    ClearProgress,
    Trailer,
    Rate,
    Versions,
    FileInfo,
    Download,
    AddToWatchlist,
    RemoveFromWatchlist,
    Details,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::FromStart => "Play from beginning",
            Action::MarkWatched => "Mark as watched",
            Action::MarkUnwatched => "Mark as unwatched",
            Action::ClearProgress => "Remove from Continue Watching",
            Action::Trailer => "Play trailer",
            Action::Rate => "Rate",
            Action::Versions => "Play version…",
            Action::FileInfo => "File info",
            Action::Download => "Download",
            Action::AddToWatchlist => "Add to watchlist",
            Action::RemoveFromWatchlist => "Remove from watchlist",
            Action::Details => "View details",
        }
    }
}

pub struct Context {
    pub local: bool,
    pub details: bool,
    pub can_rate: bool,
    pub can_download: bool,
    pub signed_in: bool,
    pub watchlisted: Option<bool>,
}

pub fn actions(it: &Item, c: &Context) -> Vec<Action> {
    let library = c.local && matches!(it.kind.as_str(), "movie" | "episode" | "show" | "season");
    let in_progress = c.local && it.view_offset.is_some_and(|o| o > 0) && matches!(it.kind.as_str(), "movie" | "episode");
    let mut v = Vec::new();
    if in_progress {
        v.extend([Action::FromStart, Action::MarkWatched, Action::MarkUnwatched, Action::ClearProgress]);
        if c.details {
            v.push(Action::Details);
        }
        return v;
    }
    if it.trailer().is_some() {
        v.push(Action::Trailer);
    }
    if library {
        v.push(if it.watched() { Action::MarkUnwatched } else { Action::MarkWatched });
    }
    if library && c.can_rate {
        v.push(Action::Rate);
    }
    if c.local && it.media.len() > 1 {
        v.push(Action::Versions);
    }
    if c.local && !it.media.is_empty() {
        v.push(Action::FileInfo);
    }
    if library && c.can_download {
        v.push(Action::Download);
    }
    if c.signed_in && it.plex_guid().is_some() && matches!(it.kind.as_str(), "movie" | "show") {
        v.push(if c.watchlisted == Some(true) { Action::RemoveFromWatchlist } else { Action::AddToWatchlist });
    }
    if c.details {
        v.push(Action::Details);
    }
    v
}

pub fn version_label(m: &Media) -> String {
    join(&[fmt_resolution(&m.video_resolution), m.video_codec.to_ascii_uppercase(), m.video_dynamic_range.clone(), m.bitrate.map(fmt_bitrate).unwrap_or_default(), m.container.to_ascii_uppercase()])
}

pub fn stars(it: &Item) -> u8 {
    it.user_rating.map_or(0, |r| (r / 2.).round().clamp(0., 5.) as u8)
}

pub fn file_info(it: &Item) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    for (i, m) in it.media.iter().enumerate() {
        if it.media.len() > 1 {
            lines.push((format!("Version {}", i + 1), version_label(m)));
        }
        let part = m.part.first();
        let streams = |t: u8| part.map(|p| p.stream.iter().filter(|s| s.stream_type == t).map(|s| s.display_title.clone()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        let fields = [
            ("File", part.map(|p| p.file.clone()).unwrap_or_default()),
            ("Container", m.container.to_ascii_uppercase()),
            ("Video", join(&[fmt_resolution(&m.video_resolution), m.video_codec.to_ascii_uppercase(), m.video_dynamic_range.clone()])),
            ("Bitrate", m.bitrate.map(fmt_bitrate).unwrap_or_default()),
            ("Audio", streams(2)),
            ("Subtitles", streams(3)),
        ];
        lines.extend(fields.into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.to_owned(), v)));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::{Extras, Part, Stream};

    fn ctx() -> Context {
        Context { local: true, details: true, can_rate: true, can_download: true, signed_in: true, watchlisted: None }
    }

    #[test]
    fn in_progress_items_get_the_continue_watching_menu() {
        let ep = Item { kind: "episode".into(), view_offset: Some(60_000), duration: Some(1_200_000), ..Default::default() };
        assert_eq!(actions(&ep, &ctx()), [Action::FromStart, Action::MarkWatched, Action::MarkUnwatched, Action::ClearProgress, Action::Details]);
        assert_eq!(actions(&ep, &Context { details: false, ..ctx() }).last(), Some(&Action::ClearProgress));
    }

    #[test]
    fn library_items_get_the_full_menu() {
        let movie = Item {
            kind: "movie".into(),
            guid: "plex://movie/5d77".into(),
            view_count: Some(1),
            extras: Some(Extras { metadata: vec![Item { rating_key: "9".into(), subtype: "trailer".into(), ..Default::default() }] }),
            media: vec![Media::default(), Media::default()],
            ..Default::default()
        };
        let all = [Action::Trailer, Action::MarkUnwatched, Action::Rate, Action::Versions, Action::FileInfo, Action::Download, Action::RemoveFromWatchlist, Action::Details];
        assert_eq!(actions(&movie, &Context { watchlisted: Some(true), ..ctx() }), all);
        let discover = actions(&movie, &Context { local: false, can_rate: false, can_download: false, ..ctx() });
        assert_eq!(discover, [Action::Trailer, Action::AddToWatchlist, Action::Details]);
    }

    #[test]
    fn file_info_lists_each_version() {
        let m = |res: &str| Media { video_resolution: res.into(), container: "mkv".into(), part: vec![Part { file: "/m.mkv".into(), stream: vec![Stream { stream_type: 2, display_title: "English (AC3 5.1)".into(), ..Default::default() }], ..Default::default() }], ..Default::default() };
        let it = Item { media: vec![m("1080"), m("4k")], user_rating: Some(7.0), ..Default::default() };
        let info = file_info(&it);
        assert_eq!(info[0], ("Version 1".into(), "1080p · MKV".into()));
        assert!(info.contains(&("Audio".into(), "English (AC3 5.1)".into())));
        assert_eq!((stars(&it), version_label(&it.media[1])), (4, "4K · MKV".into()));
    }
}
