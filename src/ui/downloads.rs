use crate::app::{Card, Nav, Plaivo, card_for, download_root, episode_card, join};
use crate::downloads::{Entry, State};
use crate::plex::{DISCOVER_ID, Item};
use crate::ui::home::content_width;
use crate::ui::library::Grid;
use crate::ui::settings::{note, toggle_row};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{
    chip, icon, overlay_button, pill, progress_bar, round_button, row, section, tip,
};
use gpui::{
    AnyElement, Context, Div, FontWeight, PathPromptOptions, PromptLevel, SharedString, Stateful,
    Window, div, prelude::*, px,
};
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dl {
    None,
    Queued,
    Active(u8),
    Paused,
    Failed,
    Done,
}

impl Dl {
    pub fn label(self, idle: &'static str) -> SharedString {
        match self {
            Dl::None => idle.into(),
            Dl::Queued => "Queued".into(),
            Dl::Active(p) => format!("Downloading {p}%").into(),
            Dl::Paused => "Download paused".into(),
            Dl::Failed => "Download failed".into(),
            Dl::Done => "Downloaded".into(),
        }
    }

    pub fn icon(self) -> Icon {
        match self {
            Dl::Paused => Icon::Pause,
            Dl::Failed => Icon::Refresh,
            Dl::Done => Icon::DownloadDone,
            _ => Icon::Download,
        }
    }
}

pub fn download_state<'a>(entries: impl IntoIterator<Item = &'a Entry>, expected: usize) -> Dl {
    let (mut n, mut done, mut sum) = (0, 0, 0.0);
    let (mut downloading, mut queued, mut paused, mut failed) = (false, false, false, false);
    for e in entries {
        n += 1;
        match e.state {
            State::Done => {
                done += 1;
                sum += 1.0;
            }
            State::Downloading => {
                downloading = true;
                if e.total > 0 {
                    sum += e.bytes as f64 / e.total as f64;
                }
            }
            State::Queued => queued = true,
            State::Paused => paused = true,
            State::Failed => failed = true,
        }
    }
    match () {
        _ if downloading => Dl::Active((sum * 100.0 / n.max(expected) as f64).min(99.0) as u8),
        _ if queued => Dl::Queued,
        _ if failed => Dl::Failed,
        _ if paused => Dl::Paused,
        _ if n > 0 && done >= expected => Dl::Done,
        _ => Dl::None,
    }
}

pub fn fmt_size(bytes: u64) -> String {
    const GB: f64 = (1u64 << 30) as f64;
    if bytes as f64 >= GB {
        format!("{:.1} GB", bytes as f64 / GB)
    } else {
        format!("{} MB", bytes >> 20)
    }
}

pub fn offline_cards(items: &[(String, Item)]) -> Vec<Card> {
    let mut cards = Vec::new();
    let mut episodes: HashMap<(&str, &str), usize> = HashMap::new();
    for (server, it) in items {
        if it.kind != "episode" || it.grandparent_rating_key.is_empty() {
            cards.push(card_for(it, false, server));
            continue;
        }
        let n = episodes
            .entry((server, &it.grandparent_rating_key))
            .or_default();
        *n += 1;
        if *n == 1 {
            cards.push(card_for(&it.clone().as_show(), false, server));
        }
    }
    for c in &mut cards {
        if let Some(n) = episodes.get(&(c.server.as_ref(), c.key.as_ref())) {
            c.subtitle = format!("{n} episode{}", if *n == 1 { "" } else { "s" }).into();
        }
    }
    cards.sort_by_key(|c| c.title.to_lowercase());
    cards
}

pub fn offline_episodes<'a>(
    offline: &'a [(String, Item)],
    server: &str,
    key: &str,
) -> Vec<&'a Item> {
    let mut items: Vec<&Item> = offline
        .iter()
        .filter(|(s, it)| s == server && (it.rating_key == key || it.grandparent_rating_key == key))
        .map(|(_, it)| it)
        .collect();
    items.sort_by_key(|it| (it.parent_index, it.index));
    items
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Detail,
    Season,
    Episode(usize),
}

impl Plaivo {
    fn target_item(&self, t: Target) -> Option<&Item> {
        match t {
            Target::Detail => self
                .detail
                .as_ref()
                .map(|d| &d.item)
                .filter(|it| matches!(it.kind.as_str(), "movie" | "episode" | "show")),
            Target::Season => self
                .season
                .as_ref()
                .and_then(|k| self.season_items.iter().find(|s| &s.rating_key == k)),
            Target::Episode(i) => self.episode_items.get(i),
        }
        .filter(|_| self.detail_id.0 != DISCOVER_ID)
    }

    fn target_entries(&self, t: Target) -> Option<(Vec<&Entry>, usize)> {
        let it = self.target_item(t)?;
        let (server, key) = (&self.detail_id.0, &it.rating_key);
        let in_season = |k: &str| {
            t == Target::Season
                && self
                    .episode_items
                    .iter()
                    .any(|e| e.rating_key == k && &e.parent_rating_key == key)
        };
        let entries = self
            .download_entries
            .iter()
            .filter(|e| {
                &e.server == server
                    && (&e.key == key || e.parents.contains(key) || in_season(&e.key))
            })
            .collect();
        Some((entries, it.leaf_count.unwrap_or(1).max(1) as usize))
    }

    pub fn target_state(&self, t: Target) -> Option<Dl> {
        self.target_entries(t)
            .map(|(entries, expected)| download_state(entries, expected))
    }

    pub fn detail_download_states(&self) -> Vec<Option<Dl>> {
        [Target::Detail, Target::Season]
            .into_iter()
            .chain((0..self.episode_items.len()).map(Target::Episode))
            .map(|t| self.target_state(t))
            .collect()
    }

    fn download(&mut self, t: Target, cx: &mut Context<Self>) {
        let Some((entries, expected)) = self.target_entries(t) else {
            return;
        };
        let server = self.detail_id.0.clone();
        match download_state(entries.iter().copied(), expected) {
            Dl::None => {
                let (Some(srv), Some(it)) = (self.server(&server), self.target_item(t).cloned())
                else {
                    return;
                };
                self.with_downloads(cx, move |d| d.enqueue(&srv, &it));
            }
            Dl::Paused | Dl::Failed => {
                let keys: Vec<String> = entries
                    .iter()
                    .filter(|e| matches!(e.state, State::Paused | State::Failed))
                    .map(|e| e.key.clone())
                    .collect();
                self.with_downloads(cx, move |d| keys.iter().for_each(|k| d.resume(&server, k)));
            }
            _ => self.navigate(Nav::Downloads(None), true, cx),
        }
    }

    pub fn detail_download_button(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let st = self.target_state(Target::Detail)?;
        let inner = match st {
            Dl::Active(p) => div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{p}%"))
                .into_any_element(),
            _ => icon(
                st.icon(),
                24.,
                if st == Dl::Done {
                    theme::accent()
                } else {
                    theme::text()
                },
            )
            .into_any_element(),
        };
        Some(
            div()
                .id("download")
                .size(px(48.))
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .bg(theme::tint(0.08))
                .hover(|s| s.bg(theme::tint(0.16)))
                .tooltip(tip(st.label("Download")))
                .child(inner)
                .on_click(cx.listener(|this, _, _, cx| this.download(Target::Detail, cx))),
        )
    }

    pub fn season_download_chip(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let st = self.target_state(Target::Season)?;
        Some(
            chip("season-download", st.label("Download season"), false)
                .gap(px(6.))
                .child(icon(st.icon(), 18., theme::text()))
                .on_click(cx.listener(|this, _, _, cx| this.download(Target::Season, cx))),
        )
    }

    pub fn episode_download_button(&self, i: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let st = self.target_state(Target::Episode(i))?;
        let inner = match st {
            Dl::Active(p) => div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{p}%"))
                .into_any_element(),
            _ => icon(
                st.icon(),
                22.,
                if st == Dl::Done {
                    theme::accent().into()
                } else {
                    theme::muted()
                },
            )
            .into_any_element(),
        };
        Some(
            div()
                .id(("episode-download", i))
                .size(px(40.))
                .my_auto()
                .flex_none()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(theme::tint(0.08)))
                .occlude()
                .tooltip(tip(st.label("Download")))
                .child(inner)
                .on_click(cx.listener(move |this, _, _, cx| this.download(Target::Episode(i), cx)))
                .into_any_element(),
        )
    }

    pub fn pick_download_dir(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Use this folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picked.await
                && let Some(dir) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    this.set_download_dir(dir.to_string_lossy().into_owned(), cx)
                })
                .ok();
            }
        })
        .detach();
    }

    pub fn download_summary(&self) -> String {
        format!(
            "{} · Remove watched {}",
            download_root(&self.settings.download_dir).display(),
            if self.settings.auto_remove_watched {
                "on"
            } else {
                "off"
            }
        )
    }

    pub fn render_download_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let custom = !self.settings.download_dir.is_empty();
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                section("Location")
                    .child(
                        row(
                            "download-dir",
                            Some(Icon::Folder),
                            "Download folder",
                            download_root(&self.settings.download_dir)
                                .display()
                                .to_string(),
                        )
                        .when(custom, |d| {
                            d.child(
                                pill("download-dir-default", "Default", None, false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.set_download_dir(String::new(), cx)
                                    }),
                                ),
                            )
                        })
                        .child(
                            pill("download-dir-change", "Change", None, false)
                                .on_click(cx.listener(|this, _, _, cx| this.pick_download_dir(cx))),
                        ),
                    ),
            )
            .child(
                section("Storage").child(
                    toggle_row(
                        "auto-remove",
                        "Remove watched downloads",
                        "Deletes a download after you finish watching it",
                        self.settings.auto_remove_watched,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.auto_remove_watched = !this.settings.auto_remove_watched;
                        this.save_settings_soon(cx);
                        let on = this.settings.auto_remove_watched;
                        this.with_downloads(cx, move |d| d.set_auto_remove_watched(on));
                        cx.notify();
                    })),
                ),
            )
    }

    fn confirm_remove(
        &mut self,
        server: String,
        keys: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let answer = window.prompt(
            PromptLevel::Warning,
            "Remove this download?",
            Some("The downloaded files are deleted from this computer."),
            &["Remove", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                this.update(cx, |this, cx| {
                    this.with_downloads(cx, move |d| {
                        keys.iter().for_each(|k| d.remove(&server, k))
                    });
                    this.back(cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn queue_row(&self, e: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let status = match e.state {
            State::Downloading if e.total > 0 => {
                format!("{} of {}", fmt_size(e.bytes), fmt_size(e.total))
            }
            State::Downloading => "Starting…".into(),
            State::Queued if !self.servers.iter().any(|s| s.id == e.server) => {
                "Server unavailable".into()
            }
            State::Queued => "Queued".into(),
            State::Paused => "Paused".into(),
            State::Failed => join(&["Failed".into(), e.error.clone()]),
            State::Done => String::new(),
        };
        let frac = if e.total > 0 {
            e.bytes as f32 / e.total as f32
        } else {
            0.
        };
        let stopped = matches!(e.state, State::Paused | State::Failed);
        let id = SharedString::from(format!("{}/{}", e.server, e.key));
        let button = |n: usize, i_: Icon, label: &'static str| {
            round_button(
                (id.clone(), n),
                i_,
                36.,
                22.,
                theme::black(0.),
                theme::tint(0.08),
                theme::text(),
            )
            .tooltip(tip(label))
        };
        let (server, key) = (e.server.clone(), e.key.clone());
        let toggle = button(
            0,
            if stopped { Icon::Play } else { Icon::Pause },
            if stopped { "Resume" } else { "Pause" },
        )
        .on_click(cx.listener({
            let (server, key) = (server.clone(), key.clone());
            move |this, _, _, cx| {
                let (server, key) = (server.clone(), key.clone());
                this.with_downloads(cx, move |d| {
                    if stopped {
                        d.resume(&server, &key)
                    } else {
                        d.pause(&server, &key)
                    }
                });
            }
        }));
        let cancel =
            button(1, Icon::Close, "Cancel").on_click(cx.listener(move |this, _, _, cx| {
                let (server, key) = (server.clone(), key.clone());
                this.with_downloads(cx, move |d| d.remove(&server, &key));
            }));
        div()
            .id((id.clone(), 2))
            .w_full()
            .px(px(20.))
            .py(px(10.))
            .rounded(px(12.))
            .bg(theme::surface())
            .flex()
            .items_center()
            .gap(px(16.))
            .child(icon(
                if e.kind == "movie" {
                    Icon::Movie
                } else {
                    Icon::Tv
                },
                24.,
                theme::muted(),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(e.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .truncate()
                            .child(join(&[e.subtitle.clone(), status])),
                    )
                    .child(progress_bar(frac, 4., theme::outline())),
            )
            .child(toggle)
            .child(cancel)
            .into_any_element()
    }

    pub fn render_downloads(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some((server, key)) = self.offline_view.clone() {
            return self.render_offline_item(server, key, cx);
        }
        let entries = std::mem::take(&mut self.download_entries);
        let queue: Vec<&Entry> = entries.iter().filter(|e| e.state != State::Done).collect();
        let active = queue
            .iter()
            .any(|e| matches!(e.state, State::Queued | State::Downloading));
        let stopped = queue
            .iter()
            .any(|e| matches!(e.state, State::Paused | State::Failed));
        let rows: Vec<AnyElement> = queue.iter().map(|e| self.queue_row(e, cx)).collect();
        let queue_empty = queue.is_empty();
        self.download_entries = entries;
        let empty = self.offline_cards.is_empty();
        let scroll = self.offline_scroll.clone();
        let grid =
            (!empty).then(|| self.grid(Grid::Offline, content_width(self, window), scroll, cx));
        let header = div()
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .text_size(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .child("Downloads"),
            )
            .when(active, |d| {
                d.child(
                    pill("pause-all", "Pause all", Some(Icon::Pause), false).on_click(
                        cx.listener(|this, _, _, cx| this.with_downloads(cx, |d| d.pause_all())),
                    ),
                )
            })
            .when(stopped, |d| {
                d.child(
                    pill("resume-all", "Resume all", Some(Icon::Play), false).on_click(
                        cx.listener(|this, _, _, cx| this.with_downloads(cx, |d| d.resume_all())),
                    ),
                )
            });
        div()
            .size_full()
            .pt(px(24.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(header)
            .when(!rows.is_empty(), |d| {
                d.child(
                    div()
                        .id("download-queue")
                        .flex_none()
                        .max_h(gpui::relative(0.4))
                        .overflow_y_scroll()
                        .px(px(16.))
                        .child(section("Queue").children(rows)),
                )
            })
            .when_some(grid, |d, grid| {
                d.child(
                    div()
                        .flex_none()
                        .px(px(16.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::muted())
                        .child("Downloaded"),
                )
                .child(grid)
            })
            .when(queue_empty && empty, |d| {
                d.child(div().px(px(16.)).child(note("Nothing downloaded yet.")))
            })
            .into_any_element()
    }

    fn render_offline_item(
        &mut self,
        server: String,
        key: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let offline = std::mem::take(&mut self.offline);
        let items = offline_episodes(&offline, &server, &key);
        let title = items.first().map_or(String::new(), |it| {
            if it.kind == "episode" {
                it.grandparent_title.clone()
            } else {
                it.title.clone()
            }
        });
        let keys: Vec<String> = items.iter().map(|it| it.rating_key.clone()).collect();
        let mut list = div().flex().flex_col().gap(px(8.));
        let mut season = None;
        for (i, it) in items.iter().enumerate() {
            if it.kind == "episode" && season != Some(it.parent_index) {
                season = Some(it.parent_index);
                let label = if it.parent_title.is_empty() {
                    it.parent_index
                        .map_or("Episodes".into(), |n| format!("Season {n}"))
                } else {
                    it.parent_title.clone()
                };
                list = list.child(
                    div()
                        .pt(px(12.))
                        .text_size(px(18.))
                        .font_weight(FontWeight::BOLD)
                        .child(label),
                );
            }
            let (s, k) = (server.clone(), it.rating_key.clone());
            let remove = overlay_button(
                ("offline-remove", i),
                Icon::Delete,
                40.,
                22.,
                theme::black(0.),
                theme::tint(0.08),
                theme::muted(),
            )
            .my_auto()
            .tooltip(tip("Remove download"))
            .on_click(cx.listener(move |this, _, _, cx| {
                let (s, k) = (s.clone(), k.clone());
                this.with_downloads(cx, move |d| d.remove(&s, &k));
            }));
            let card = episode_card(it, &server);
            list = list.child(self.episode_row(i, &card, Some(remove.into_any_element()), cx));
        }
        let nothing = items.is_empty();
        self.offline = offline;
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                round_button(
                    "offline-back",
                    Icon::Back,
                    40.,
                    22.,
                    theme::black(0.),
                    theme::tint(0.08),
                    theme::text(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .child(title),
            )
            .when(!keys.is_empty(), |d| {
                d.child(
                    pill("offline-remove-all", "Remove", Some(Icon::Delete), false).on_click(
                        cx.listener(move |this, _, window, cx| {
                            this.confirm_remove(server.clone(), keys.clone(), window, cx)
                        }),
                    ),
                )
            });
        div()
            .id("offline-item")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .px(px(24.))
                    .pt(px(24.))
                    .pb(px(32.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(header)
                    .when(nothing, |d| d.child(note("Nothing downloaded here.")))
                    .child(list),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: &str, bytes: u64, total: u64) -> Entry {
        serde_json::from_value(serde_json::json!({"server": "s", "key": "k", "kind": "episode", "title": "", "subtitle": "", "state": state, "bytes": bytes, "total": total})).unwrap()
    }

    #[test]
    fn download_state_summarises_entries() {
        let (done, active, queued, paused, failed) = (
            entry("Done", 10, 10),
            entry("Downloading", 5, 10),
            entry("Queued", 0, 0),
            entry("Paused", 2, 10),
            entry("Failed", 0, 0),
        );
        assert_eq!(download_state([], 1), Dl::None);
        assert_eq!(download_state([&done], 1), Dl::Done);
        assert_eq!(download_state([&active], 1), Dl::Active(50));
        assert_eq!(
            download_state([&done, &active, &queued, &queued], 4),
            Dl::Active(37)
        );
        assert_eq!(download_state([&done, &queued], 2), Dl::Queued);
        assert_eq!(download_state([&done, &paused, &failed], 3), Dl::Failed);
        assert_eq!(download_state([&done, &paused], 2), Dl::Paused);
        assert_eq!(download_state([&done, &done], 5), Dl::None);
        assert_eq!(
            download_state([&entry("Downloading", 10, 10)], 1),
            Dl::Active(99)
        );
    }

    #[test]
    fn sizes_read_like_a_file_manager() {
        assert_eq!(fmt_size(0), "0 MB");
        assert_eq!(fmt_size(700 << 20), "700 MB");
        assert_eq!(fmt_size(3 << 29), "1.5 GB");
    }

    #[test]
    fn episodes_fold_into_one_show_card() {
        let ep = |key: &str, show: &str, title: &str| Item {
            rating_key: key.into(),
            kind: "episode".into(),
            grandparent_rating_key: show.into(),
            grandparent_title: title.into(),
            ..Default::default()
        };
        let movie = Item {
            rating_key: "m".into(),
            kind: "movie".into(),
            title: "alien".into(),
            ..Default::default()
        };
        let items = vec![
            ("a".into(), ep("1", "s1", "Zed")),
            ("a".into(), movie),
            ("a".into(), ep("2", "s1", "Zed")),
            ("b".into(), ep("3", "s1", "Zed")),
        ];
        let cards: Vec<_> = offline_cards(&items)
            .into_iter()
            .map(|c| {
                (
                    c.server.to_string(),
                    c.key.to_string(),
                    c.kind.to_string(),
                    c.subtitle.to_string(),
                )
            })
            .collect();
        assert_eq!(
            cards,
            [
                ("a".into(), "m".into(), "movie".into(), String::new()),
                ("a".into(), "s1".into(), "show".into(), "2 episodes".into()),
                ("b".into(), "s1".into(), "show".into(), "1 episode".into()),
            ]
        );
    }
}
