use crate::app::{Nav, Page, Plaivo};
use crate::plex::{DISCOVER_ID, Item};
use crate::settings::Theme;
use crate::ui::theme::{self, DARK, Icon, LIGHT, OLED, Palette};
use crate::ui::widgets::{backdrop, floating, icon, pill, round_button, row, shadow};
use gpui::{
    AnyElement, Context, Div, FontWeight, Hsla, Pixels, Point, SharedString, Window, div,
    prelude::*, px, rgb, rgba,
};
use plaivo_core::menu::{self, Action};

const MENU_W: f32 = 268.;
const MENU_ROW: f32 = 40.;

pub struct ItemMenu {
    at: Point<Pixels>,
    server: String,
    key: String,
    item: Option<Item>,
    watchlisted: Option<bool>,
}

pub enum Dialog {
    Rate {
        server: String,
        key: String,
        stars: u8,
    },
    Versions {
        server: String,
        key: String,
        labels: Vec<String>,
    },
    FileInfo {
        title: String,
        lines: Vec<(String, String)>,
    },
    Theme,
}

fn swatch(p: &Palette) -> Div {
    const S: f32 = 36.;
    let c = |v: u32| Hsla::from(rgb(v));
    let text = c(p.text);
    div()
        .absolute()
        .top_0()
        .left_0()
        .size(px(S))
        .rounded(px(S * 0.25))
        .bg(c(p.bg))
        .child(
            div()
                .absolute()
                .left(px(S * 0.16))
                .top(px(S * 0.44))
                .w(px(S * 0.68))
                .h(px(S * 0.40))
                .rounded(px(S * 0.08))
                .bg(c(p.surface))
                .border_1()
                .border_color(rgba(p.outline)),
        )
        .child(
            div()
                .absolute()
                .left(px(S * 0.26))
                .top(px(S * 0.54))
                .w(px(S * 0.40))
                .h(px(S * 0.07))
                .rounded(px(S * 0.04))
                .bg(text),
        )
        .child(
            div()
                .absolute()
                .left(px(S * 0.26))
                .top(px(S * 0.68))
                .w(px(S * 0.26))
                .h(px(S * 0.06))
                .rounded(px(S * 0.04))
                .bg(text.opacity(0.6)),
        )
        .child(
            div()
                .absolute()
                .left(px(S * 0.19))
                .top(px(S * 0.13))
                .size(px(S * 0.22))
                .rounded_full()
                .bg(text),
        )
}

fn theme_preview(t: Theme) -> Div {
    let frame = div().relative().size(px(36.)).flex_none();
    let outline = div()
        .absolute()
        .top_0()
        .left_0()
        .size(px(36.))
        .rounded(px(9.))
        .border_1()
        .border_color(theme::outline());
    match t {
        Theme::Light => frame.child(swatch(&LIGHT)),
        Theme::Dark => frame.child(swatch(&DARK)),
        Theme::Oled => frame.child(swatch(&OLED)),
        Theme::System => frame.child(swatch(&LIGHT)).child(
            div()
                .absolute()
                .top_0()
                .left(px(18.))
                .w(px(18.))
                .h_full()
                .overflow_hidden()
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px(-18.))
                        .size(px(36.))
                        .child(swatch(&DARK)),
                ),
        ),
    }
    .child(outline)
}

fn radio(on: bool) -> Div {
    div()
        .size(px(22.))
        .flex_none()
        .rounded_full()
        .border_2()
        .border_color(if on {
            theme::text().into()
        } else {
            theme::muted()
        })
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| {
            d.child(div().size(px(10.)).rounded_full().bg(theme::text()))
        })
}

fn action_icon(a: Action) -> Icon {
    match a {
        Action::FromStart => Icon::Replay,
        Action::MarkWatched => Icon::Check,
        Action::MarkUnwatched => Icon::Remove,
        Action::ClearProgress => Icon::Close,
        Action::Trailer => Icon::Movie,
        Action::Rate => Icon::StarBorder,
        Action::Versions => Icon::Layers,
        Action::FileInfo => Icon::HighQuality,
        Action::Download => Icon::Download,
        Action::AddToWatchlist => Icon::BookmarkAdd,
        Action::RemoveFromWatchlist => Icon::Bookmark,
        Action::Details => Icon::Info,
    }
}

impl Plaivo {
    pub fn open_item_menu(
        &mut self,
        server: String,
        key: String,
        at: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.page == Page::Downloads || key.is_empty() {
            return;
        }
        let known = self
            .detail
            .as_ref()
            .filter(|d| d.server.as_ref() == server && d.item.rating_key == key)
            .map(|d| (d.item.clone(), d.watchlisted));
        let (item, watchlisted) = known.map_or((None, None), |(it, w)| (Some(it), w));
        self.menu = Some(ItemMenu {
            at,
            server: server.clone(),
            key: key.clone(),
            item: item.clone(),
            watchlisted,
        });
        match item {
            Some(it) if watchlisted.is_none() => self.fetch_watchlisted(&it, cx),
            Some(_) => {}
            None => {
                let Some(srv) = self.server(&server) else {
                    return;
                };
                let k = key.clone();
                self.load(
                    cx,
                    move |c| srv.metadata(c, &k),
                    move |this, r, cx| {
                        let Some(m) = this
                            .menu
                            .as_mut()
                            .filter(|m| m.key == key && m.server == server)
                        else {
                            return;
                        };
                        match r {
                            Ok(it) => {
                                let first = m.item.is_none();
                                m.item = Some(it.clone());
                                if first {
                                    this.fetch_watchlisted(&it, cx);
                                }
                            }
                            Err(e) if m.item.is_none() => {
                                this.menu = None;
                                this.toast(format!("Could not load details: {e}"), cx);
                            }
                            Err(_) => {}
                        }
                    },
                );
            }
        }
        cx.notify();
    }

    pub fn fetch_watchlisted(&mut self, it: &Item, cx: &mut Context<Self>) {
        let (Some(token), Some(guid)) = (
            self.account.token.clone(),
            it.plex_guid()
                .map(str::to_owned)
                .filter(|_| matches!(it.kind.as_str(), "movie" | "show")),
        ) else {
            return;
        };
        let (c, key) = (self.client.clone(), it.rating_key.clone());
        Self::bg(
            cx,
            move || c.watchlisted(&token, &guid),
            move |this, r, _| match r {
                Ok(on) => this.set_watchlisted_state(&key, on),
                Err(e) => log::debug!("watchlist state for {key}: {e}"),
            },
        );
    }

    fn set_watchlisted_state(&mut self, key: &str, on: bool) {
        if let Some(d) = self.detail.as_mut().filter(|d| d.item.rating_key == key) {
            d.watchlisted = Some(on);
        }
        if let Some(m) = self.menu.as_mut().filter(|m| m.key == key) {
            m.watchlisted = Some(on);
        }
    }

    pub fn open_theme_picker(&mut self, cx: &mut Context<Self>) {
        self.dialog = Some(Dialog::Theme);
        cx.notify();
    }

    pub fn close_overlays(&mut self) -> bool {
        self.menu.take().is_some()
            | self.dialog.take().is_some()
            | std::mem::take(&mut self.sort_open)
            | self.services.cancel()
    }

    fn menu_entries(
        &self,
        server: &str,
        it: &Item,
        watchlisted: Option<bool>,
    ) -> Vec<(Icon, &'static str, Action)> {
        let local = server != DISCOVER_ID;
        let ctx = menu::Context {
            local,
            details: !(self.page == Page::Detail
                && self.detail_id == (server.to_owned(), it.rating_key.clone())),
            can_rate: self.server(server).is_some_and(|s| s.can_rate()),
            can_download: self.downloads.is_some()
                && !self.download_entries.iter().any(|e| {
                    e.server == server
                        && (e.key == it.rating_key || e.parents.contains(&it.rating_key))
                }),
            signed_in: self.account.token.is_some(),
            watchlisted,
        };
        menu::actions(it, &ctx)
            .into_iter()
            .map(|a| (action_icon(a), a.label(), a))
            .collect()
    }

    pub fn detail_action_watched(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        self.run_act(
            d.server.to_string(),
            d.item.clone(),
            if on {
                Action::MarkWatched
            } else {
                Action::MarkUnwatched
            },
            cx,
        );
    }

    pub fn detail_action_watchlist(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        self.run_act(
            d.server.to_string(),
            d.item.clone(),
            if on {
                Action::AddToWatchlist
            } else {
                Action::RemoveFromWatchlist
            },
            cx,
        );
    }

    pub fn detail_action_rate(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.detail.as_ref() else {
            return;
        };
        self.run_act(d.server.to_string(), d.item.clone(), Action::Rate, cx);
    }

    fn act(&mut self, act: Action, cx: &mut Context<Self>) {
        let Some(m) = self.menu.take() else { return };
        if let Some(it) = m.item {
            self.run_act(m.server, it, act, cx);
        }
        cx.notify();
    }

    fn run_act(&mut self, server: String, it: Item, act: Action, cx: &mut Context<Self>) {
        let key = it.rating_key.clone();
        match act {
            Action::Details => self.navigate(Nav::Detail(server, key, None), true, cx),
            Action::FromStart => self.play_from_start(server, key, cx),
            Action::ClearProgress => {
                let (Some(srv), c) = (self.server(&server), self.client.clone()) else {
                    return;
                };
                Self::bg(
                    cx,
                    move || srv.remove_from_continue_watching(&c, &key),
                    move |this, r, cx| match r {
                        Ok(()) => {
                            this.toast("Removed from Continue Watching", cx);
                            this.load_home(cx);
                            this.refresh_page(cx);
                        }
                        Err(e) => this.toast(format!("Could not update: {e}"), cx),
                    },
                );
            }
            Action::Trailer => {
                if let Some(t) = it.trailer() {
                    self.play(server, t.rating_key.clone(), cx);
                }
            }
            Action::MarkWatched | Action::MarkUnwatched => {
                let on = act == Action::MarkWatched;
                let (Some(srv), c) = (self.server(&server), self.client.clone()) else {
                    return;
                };
                Self::bg(
                    cx,
                    move || srv.set_watched(&c, &key, on),
                    move |this, r, cx| match r {
                        Ok(()) => {
                            this.toast(
                                if on {
                                    "Marked as watched"
                                } else {
                                    "Marked as unwatched"
                                },
                                cx,
                            );
                            this.refresh_page(cx);
                        }
                        Err(e) => this.toast(format!("Could not update: {e}"), cx),
                    },
                );
            }
            Action::Rate => {
                self.dialog = Some(Dialog::Rate {
                    server,
                    key,
                    stars: menu::stars(&it),
                })
            }
            Action::Versions => {
                self.dialog = Some(Dialog::Versions {
                    server,
                    key,
                    labels: it.media.iter().map(menu::version_label).collect(),
                })
            }
            Action::FileInfo => {
                self.dialog = Some(Dialog::FileInfo {
                    title: it.title.clone(),
                    lines: menu::file_info(&it),
                })
            }
            Action::Download => {
                let Some(srv) = self.server(&server) else {
                    return;
                };
                self.with_downloads(cx, move |d| d.enqueue(&srv, &it));
                self.toast("Added to downloads", cx);
            }
            Action::AddToWatchlist | Action::RemoveFromWatchlist => {
                let on = act == Action::AddToWatchlist;
                let (Some(token), Some(guid)) = (
                    self.account.token.clone(),
                    it.plex_guid().map(str::to_owned),
                ) else {
                    return;
                };
                let c = self.client.clone();
                Self::bg(
                    cx,
                    move || c.set_watchlisted(&token, &guid, on),
                    move |this, r, cx| match r {
                        Ok(()) => {
                            this.set_watchlisted_state(&key, on);
                            this.toast(
                                if on {
                                    "Added to watchlist"
                                } else {
                                    "Removed from watchlist"
                                },
                                cx,
                            );
                        }
                        Err(e) => this.toast(format!("Could not update the watchlist: {e}"), cx),
                    },
                );
            }
        }
        cx.notify();
    }

    fn rate(&mut self, stars: Option<u8>, cx: &mut Context<Self>) {
        let Some(Dialog::Rate { server, key, .. }) = self.dialog.take() else {
            return;
        };
        let (Some(srv), c) = (self.server(&server), self.client.clone()) else {
            return;
        };
        Self::bg(
            cx,
            move || srv.rate(&c, &key, stars),
            move |this, r, cx| match r {
                Ok(()) => {
                    this.toast(
                        stars.map_or("Rating cleared".to_owned(), |s| format!("Rated {s} of 5")),
                        cx,
                    );
                    this.refresh_page(cx);
                }
                Err(e) => this.toast(format!("Could not rate: {e}"), cx),
            },
        );
        cx.notify();
    }

    fn refresh_page(&mut self, cx: &mut Context<Self>) {
        match self.nav.last().cloned() {
            Some(Nav::Detail(server, key, _)) => {
                self.navigate(Nav::Detail(server, key, self.season.clone()), false, cx)
            }
            Some(
                nav @ (Nav::Library(..) | Nav::Collection(..) | Nav::Hub(..) | Nav::Watchlist),
            ) => self.navigate(nav, false, cx),
            Some(Nav::Home) => self.load_home(cx),
            _ => {}
        }
    }

    pub fn render_overlays(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut out = Vec::new();
        if let Some(m) = &self.menu {
            out.push(self.render_menu(m, window, cx));
        }
        if self.dialog.is_some() {
            out.push(self.render_dialog(window, cx));
        }
        out.extend(self.render_sign_in(cx));
        out
    }

    fn render_menu(&self, m: &ItemMenu, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let entries = m
            .item
            .as_ref()
            .map(|it| self.menu_entries(&m.server, it, m.watchlisted));
        let rows = entries.as_ref().map_or(1, |e| e.len().max(1));
        let vp = window.viewport_size();
        let (vw, vh) = (f32::from(vp.width), f32::from(vp.height));
        let h = rows as f32 * MENU_ROW + 12.;
        let (x, y) = (f32::from(m.at.x), f32::from(m.at.y));
        let left = x.min(vw - MENU_W - 8.).max(8.);
        let top = if y + h > vh - 8. { (y - h).max(8.) } else { y };
        let items: Vec<AnyElement> = match entries {
            None => vec![
                div()
                    .h(px(MENU_ROW))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .text_color(theme::muted())
                    .child("Loading…")
                    .into_any_element(),
            ],
            Some(e) if e.is_empty() => vec![
                div()
                    .h(px(MENU_ROW))
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .text_color(theme::muted())
                    .child("No actions")
                    .into_any_element(),
            ],
            Some(e) => e
                .into_iter()
                .enumerate()
                .map(|(i, (ic, label, act))| {
                    div()
                        .id(("menu-item", i))
                        .h(px(MENU_ROW))
                        .px(px(16.))
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::tint(0.08)))
                        .child(icon(ic, 20., theme::muted()))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| this.act(act, cx)))
                        .into_any_element()
                })
                .collect(),
        };
        backdrop(
            "menu-backdrop",
            cx.listener(|this, _, _, cx| {
                this.menu = None;
                cx.notify();
            }),
        )
        .child(
            floating("menu")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(MENU_W))
                .py(px(6.))
                .rounded(px(14.))
                .bg(theme::surface())
                .border_1()
                .border_color(theme::outline())
                .shadow(shadow(24., 0.5))
                .children(items),
        )
        .into_any_element()
    }

    fn render_dialog(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let close = cx.listener(|this, _, _, cx| {
            this.dialog = None;
            cx.notify();
        });
        let (title, body): (SharedString, Vec<AnyElement>) = match self.dialog.as_ref() {
            Some(Dialog::Rate { stars, .. }) => {
                let stars = *stars;
                let row_ = div().flex().gap(px(4.)).children((1..=5u8).map(|n| {
                    round_button(
                        ("rate-star", n as usize),
                        if n <= stars {
                            Icon::Star
                        } else {
                            Icon::StarBorder
                        },
                        48.,
                        34.,
                        theme::black(0.),
                        theme::tint(0.08),
                        theme::accent(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.rate(Some(n), cx)))
                }));
                let actions = div().flex().gap(px(12.)).pt(px(8.)).when(stars > 0, |d| {
                    d.child(
                        pill("rate-clear", "Clear rating", None, false)
                            .on_click(cx.listener(|this, _, _, cx| this.rate(None, cx))),
                    )
                });
                (
                    "Rate".into(),
                    vec![row_.into_any_element(), actions.into_any_element()],
                )
            }
            Some(Dialog::Versions {
                server,
                key,
                labels,
            }) => {
                let rows = labels
                    .iter()
                    .enumerate()
                    .map(|(i, label)| {
                        let (s, k) = (server.clone(), key.clone());
                        row(
                            ("version", i),
                            Some(Icon::Layers),
                            format!("Version {}", i + 1),
                            label.clone(),
                        )
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::tint(0.12)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.dialog = None;
                            this.play_version(s.clone(), k.clone(), i, cx);
                        }))
                        .into_any_element()
                    })
                    .collect();
                ("Play version".into(), rows)
            }
            Some(Dialog::FileInfo { title, lines }) => {
                let rows = lines
                    .iter()
                    .map(|(k, v)| {
                        let header = k.starts_with("Version ");
                        div()
                            .flex()
                            .gap(px(16.))
                            .when(header, |d| d.pt(px(8.)))
                            .child(
                                div()
                                    .w(px(96.))
                                    .flex_none()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(if header {
                                        theme::text().into()
                                    } else {
                                        theme::muted()
                                    })
                                    .child(k.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(13.))
                                    .flex()
                                    .flex_col()
                                    .children(v.lines().map(|l| div().child(l.to_owned()))),
                            )
                            .into_any_element()
                    })
                    .collect();
                (title.clone().into(), rows)
            }
            Some(Dialog::Theme) => {
                let current = self.settings.theme;
                let rows = Theme::ALL
                    .into_iter()
                    .enumerate()
                    .map(|(i, t)| {
                        div()
                            .id(("theme", i))
                            .h(px(56.))
                            .px(px(12.))
                            .rounded(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .cursor_pointer()
                            .when(t == current, |d| d.bg(theme::tint(0.06)))
                            .hover(|s| s.bg(theme::tint(0.1)))
                            .child(theme_preview(t))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(t.label()),
                            )
                            .child(radio(t == current))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.settings.theme = t;
                                this.save_settings_soon(cx);
                                this.dialog = None;
                                cx.notify();
                            }))
                            .into_any_element()
                    })
                    .collect();
                ("Theme".into(), rows)
            }
            None => return div().into_any_element(),
        };
        let max_h = f32::from(window.viewport_size().height) * 0.8;
        backdrop("dialog-backdrop", close)
            .bg(theme::black(0.55))
            .flex()
            .items_center()
            .justify_center()
            .child(
                floating("dialog")
                    .w(px(460.))
                    .max_h(px(max_h))
                    .overflow_y_scroll()
                    .p(px(24.))
                    .rounded(px(16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::outline())
                    .shadow(shadow(32., 0.6))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(20.))
                            .font_weight(FontWeight::BOLD)
                            .child(title),
                    )
                    .children(body),
            )
            .into_any_element()
    }
}
