use crate::app::{Choice, Nav, Page, Plaivo, Status};
use crate::menu;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, pill, shadow};
use gpui::{
    AnyElement, Context, ElementId, FontWeight, Hsla, KeyDownEvent, KeyUpEvent, SharedString,
    Window, div, prelude::*, px, rgb,
};
use std::time::{Duration, Instant};

const SIDEBAR_ANIM: Duration = Duration::from_millis(180);

pub fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(4)
}

pub fn progress(start: Option<Instant>, dur: Duration, window: &mut Window) -> f32 {
    match start {
        Some(t0) => {
            let p = (t0.elapsed().as_secs_f32() / dur.as_secs_f32()).min(1.);
            if p < 1. {
                window.request_animation_frame();
            }
            p
        }
        None => 1.,
    }
}

impl Render for Plaivo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        theme::apply(self.settings.theme, window.appearance());
        self.scale = window.scale_factor();
        self.viewport_w = (f32::from(window.viewport_size().width) * self.scale) as u32;
        self.lru.start_frame();
        self.arrivals
            .retain(|_, t| t.elapsed() < crate::app::IMAGE_FADE);
        if !self.arrivals.is_empty() {
            window.request_animation_frame();
        }
        let standalone = self.add_server_standalone();
        let chrome = !matches!(self.page, Page::Splash | Page::Auth | Page::Player) && !standalone;
        let mini = chrome && self.player.mini && self.playing();
        let content = match self.page {
            Page::Splash => self.render_splash(),
            Page::Auth => self.render_auth(cx),
            Page::Player => self.render_player(window, cx),
            Page::AddServer if standalone => self.render_add_server(window, cx),
            _ => self.render_chrome(mini, window, cx),
        };
        let overlays = if self.page == Page::Player {
            Vec::new()
        } else {
            self.render_overlays(window, cx)
        };
        let background: Option<Hsla> = match self.page {
            Page::Player if self.player.video_ready => None,
            Page::Player | Page::Splash => Some(rgb(0x000000).into()),
            _ if mini => None,
            _ => Some(theme::bg().into()),
        };
        div()
            .id("root")
            .size_full()
            .relative()
            .font_family(".SystemUIFont")
            .text_color(theme::text())
            .text_size(px(14.))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_action(cx.listener(|this, _: &menu::OpenSettings, _, cx| {
                if !matches!(this.page, Page::Splash | Page::Auth) {
                    this.open_from_player(Nav::Settings(None), cx);
                }
            }))
            .on_action(|_: &menu::ToggleFullScreen, window, _| window.toggle_fullscreen())
            .on_action(|_: &menu::Minimize, window, _| window.minimize_window())
            .on_action(|_: &menu::Zoom, window, _| window.zoom_window())
            .when_some(background, |d, bg| d.bg(bg))
            .child(content)
            .children(overlays)
            .when_some(self.toast.clone(), |d, msg| {
                d.child(self.render_toast(msg, if mini { theme::MINI_BAR } else { 0. }))
            })
    }
}

impl Plaivo {
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key == "escape" && self.close_overlays() {
            cx.stop_propagation();
            return cx.notify();
        }
        if self.page != Page::Player {
            let sub_page = match self.page {
                Page::Detail | Page::HubList | Page::Licenses => true,
                Page::Settings => self.settings_page.is_some(),
                Page::Downloads => self.offline_view.is_some(),
                _ => false,
            };
            if ev.keystroke.key == "escape" && sub_page {
                self.back(cx);
            }
            return;
        }
        self.wake_controls(cx);
        match ev.keystroke.key.as_str() {
            "space" if ev.is_held => {
                if self.view.space_down {
                    self.start_boost();
                }
            }
            "space" => self.view.space_down = true,
            "k" => self.toggle_pause(),
            "left" | "right" => {
                let step = if ev.keystroke.modifiers.shift {
                    self.settings.seek_large
                } else {
                    self.settings.seek_small
                } as f64;
                self.seek_by(if ev.keystroke.key == "left" {
                    -step
                } else {
                    step
                });
            }
            "up" | "down" => {
                self.set_volume(
                    self.player.volume + if ev.keystroke.key == "up" { 5. } else { -5. },
                );
                self.hint_volume(cx);
            }
            "m" => self.toggle_mute(cx),
            "f" => window.toggle_fullscreen(),
            "s" => self.skip_marker(),
            "n" => self.play_neighbor(1, cx),
            "p" => self.play_neighbor(-1, cx),
            "escape" => {
                if self.player.panel.is_some() {
                    self.player.panel = None;
                } else if window.is_fullscreen() {
                    window.toggle_fullscreen();
                } else if self.settings.stop_on_close {
                    self.stop_playback(cx);
                } else {
                    self.minimize_player(cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn on_key_up(&mut self, ev: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.key != "space" || !std::mem::take(&mut self.view.space_down) {
            return;
        }
        if !self.end_boost() && self.page == Page::Player {
            self.toggle_pause();
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn render_toast(&self, msg: SharedString, lift: f32) -> impl IntoElement {
        div()
            .absolute()
            .bottom(px(24. + lift))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .max_w(px(560.))
                    .px(px(16.))
                    .py(px(12.))
                    .rounded(px(12.))
                    .bg(theme::surface())
                    .shadow(shadow(16., 0.5))
                    .text_color(theme::text())
                    .child(msg),
            )
    }

    fn render_chrome(
        &mut self,
        mini: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = match self.page {
            Page::Servers => self.render_servers(cx),
            Page::Home => self.render_home(window, cx),
            Page::Library => self.render_library(window, cx),
            Page::HubList => self.render_hub_list(window, cx),
            Page::Search => self.render_search(window, cx),
            Page::Detail => self.render_detail(window, cx),
            Page::Settings => self.render_settings_page(window, cx),
            Page::Licenses => self.render_licenses(cx),
            Page::AddServer => self.render_add_server(window, cx),
            Page::Downloads => self.render_downloads(window, cx),
            Page::Watch => self.render_watch(window, cx),
            _ => div().into_any_element(),
        };
        let sidebar = (!matches!(self.page, Page::Detail | Page::HubList))
            .then(|| self.render_sidebar(window, cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .bg(theme::bg())
                    .children(sidebar)
                    .child(div().flex_1().min_w_0().h_full().relative().child(page)),
            )
            .when(mini, |d| d.child(self.render_mini(cx)))
            .into_any_element()
    }

    fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = self.settings.sidebar_compact;
        let p = ease_out(progress(self.view.sidebar_anim, SIDEBAR_ANIM, window));
        let (from, to) = if compact {
            (theme::SIDEBAR, theme::SIDEBAR_COMPACT)
        } else {
            (theme::SIDEBAR_COMPACT, theme::SIDEBAR)
        };
        let width = from + (to - from) * p;
        let label_alpha = if compact { 1. - p } else { p };
        let item = |id: ElementId, i: Icon, label: SharedString, selected: bool| {
            div()
                .id(id)
                .h(px(40.))
                .w_full()
                .flex_none()
                .rounded(px(10.))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(px(12.))
                .pl(px(14.))
                .cursor_pointer()
                .when(selected, |d| d.bg(theme::tint(0.10)))
                .when(!selected, |d| d.hover(|s| s.bg(theme::tint(0.05))))
                .child(icon(
                    i,
                    20.,
                    if selected {
                        Hsla::from(theme::accent())
                    } else {
                        theme::muted()
                    },
                ))
                .child(
                    div()
                        .flex_none()
                        .w(px(150.))
                        .opacity(label_alpha)
                        .text_size(px(14.))
                        .whitespace_nowrap()
                        .truncate()
                        .font_weight(if selected {
                            FontWeight::SEMIBOLD
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(if selected {
                            Hsla::from(theme::text())
                        } else {
                            theme::muted()
                        })
                        .child(label),
                )
        };
        let groups = self.sections_in_order(false);
        let multi = groups.len() > 1;
        let active = if self.page == Page::Library {
            self.nav
                .iter()
                .rev()
                .take_while(|n| matches!(n, Nav::Library(..) | Nav::Collection(..)))
                .find_map(|n| match n {
                    Nav::Library(server, section) => Some((server.as_str(), section.as_str())),
                    _ => None,
                })
        } else {
            None
        };
        let mut libs = div()
            .id("libs")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.));
        let mut n = 0usize;
        for (server_name, rows) in groups {
            if multi {
                libs = libs.child(
                    div()
                        .h(px(28.))
                        .flex_none()
                        .flex()
                        .items_end()
                        .pb(px(4.))
                        .pl(px(14.))
                        .opacity(label_alpha)
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::muted())
                        .whitespace_nowrap()
                        .child(server_name.to_owned()),
                );
            }
            for x in rows {
                let selected = active == Some((x.server.as_str(), x.key.as_str()));
                let (server, key) = (x.server.clone(), x.key.clone());
                libs = libs.child(
                    item(
                        ("lib", n).into(),
                        if x.kind == "movie" {
                            Icon::Movie
                        } else {
                            Icon::Tv
                        },
                        x.title.clone(),
                        selected,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigate(Nav::Library(server.clone(), key.clone()), true, cx)
                    })),
                );
                n += 1;
            }
        }
        let fullscreen = window.is_fullscreen();
        div()
            .w(px(width))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .border_r_1()
            .border_color(theme::outline())
            .bg(theme::bg())
            .flex()
            .flex_col()
            .gap(px(2.))
            .p(px(12.))
            .pt(px(16.))
            .child(
                item("menu".into(), Icon::Menu, "Plaivo".into(), false)
                    .child(div())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .child(div().h(px(8.)).flex_none())
            .child(
                item(
                    "home".into(),
                    Icon::Home,
                    "Home".into(),
                    self.page == Page::Home,
                )
                .on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Home, true, cx))),
            )
            .child(
                item(
                    "search".into(),
                    Icon::Search,
                    "Search".into(),
                    self.page == Page::Search,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.navigate(Nav::Search, true, cx);
                    let focus = this.search.read(cx).focus.clone();
                    window.focus(&focus, cx);
                })),
            )
            .when(self.account.token.is_some(), |d| {
                let on = self.page == Page::Library && self.nav.last() == Some(&Nav::Watchlist);
                d.child(
                    item("watchlist".into(), Icon::Bookmark, "Watchlist".into(), on).on_click(
                        cx.listener(|this, _, _, cx| this.navigate(Nav::Watchlist, true, cx)),
                    ),
                )
            })
            .child(
                item(
                    "downloads".into(),
                    Icon::Download,
                    "Downloads".into(),
                    self.page == Page::Downloads,
                )
                .on_click(
                    cx.listener(|this, _, _, cx| this.navigate(Nav::Downloads(None), true, cx)),
                ),
            )
            .child(
                item(
                    "watch".into(),
                    Icon::Group,
                    "Watch Together".into(),
                    self.page == Page::Watch,
                )
                .on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Watch, true, cx))),
            )
            .child(
                div()
                    .h(px(36.))
                    .flex_none()
                    .relative()
                    .child(
                        div()
                            .absolute()
                            .bottom(px(6.))
                            .left(px(12.))
                            .opacity(label_alpha)
                            .text_size(px(11.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::muted())
                            .whitespace_nowrap()
                            .child("LIBRARIES"),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom(px(12.))
                            .left(px(12.))
                            .w(px(24.))
                            .h(px(1.))
                            .opacity(1. - label_alpha)
                            .bg(theme::outline()),
                    ),
            )
            .child(libs)
            .child(
                item(
                    "fullscreen".into(),
                    if fullscreen {
                        Icon::FullscreenExit
                    } else {
                        Icon::Fullscreen
                    },
                    if fullscreen {
                        "Exit fullscreen"
                    } else {
                        "Fullscreen"
                    }
                    .into(),
                    false,
                )
                .on_click(|_, window, _| window.toggle_fullscreen()),
            )
            .child(
                item(
                    "settings".into(),
                    Icon::Settings,
                    "Settings".into(),
                    matches!(self.page, Page::Settings | Page::Licenses),
                )
                .on_click(
                    cx.listener(|this, _, _, cx| this.navigate(Nav::Settings(None), true, cx)),
                ),
            )
    }

    fn render_splash(&self) -> AnyElement {
        let rows = self
            .splash
            .as_ref()
            .map(|s| s.servers.clone())
            .unwrap_or_default();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(12.))
            .child(logo())
            .child(div().h(px(36.)))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme::muted())
                    .child("Connecting to servers…"),
            )
            .children(rows.into_iter().map(|(_, name, status)| {
                let dot = match status {
                    Status::Pending => div().size(px(8.)).rounded_full().bg(theme::muted()),
                    Status::Ok => div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(rgb(0x2E9E5B))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(Icon::Check, 10., rgb(0xFFFFFF))),
                    Status::Error => div()
                        .size(px(14.))
                        .rounded_full()
                        .bg(rgb(0xD9453B))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon(Icon::Close, 10., rgb(0xFFFFFF))),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(14.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(dot),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(if status == Status::Error {
                                theme::muted()
                            } else {
                                theme::text().into()
                            })
                            .child(name),
                    )
            }))
            .into_any_element()
    }

    fn render_auth(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let right = if self.auth_waiting {
            div()
                .flex()
                .flex_col()
                .gap(px(16.))
                .items_center()
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme::muted())
                        .text_center()
                        .child("Waiting for authentication…"),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(theme::muted())
                        .text_center()
                        .child("Finish signing in in your browser."),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(
                            pill("retry", "Retry", None, false)
                                .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                        )
                        .child(
                            pill("cancel", "Cancel", None, false)
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_sign_in(cx))),
                        ),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    pill("signin", "Sign in with Plex", None, true)
                        .h(px(52.))
                        .on_click(cx.listener(|this, _, _, cx| this.sign_in(cx))),
                )
                .child(
                    pill(
                        "add-jellyfin",
                        "Add a Jellyfin server",
                        Some(Icon::Server),
                        false,
                    )
                    .h(px(52.))
                    .on_click(
                        cx.listener(|this, _, _, cx| this.navigate(Nav::AddServer, true, cx)),
                    ),
                )
                .when(!self.servers.is_empty(), |d| {
                    d.child(
                        pill("auth-back", "Back", None, false)
                            .h(px(52.))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel_sign_in(cx))),
                    )
                })
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(48.))
                    .p(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap(px(24.))
                            .child(logo())
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(FontWeight::BOLD)
                                    .child("Plaivo"),
                            ),
                    )
                    .child(
                        div()
                            .w(px(340.))
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .child(right)
                            .when_some(self.auth_error.clone(), |d, e| {
                                d.child(
                                    div().text_size(px(13.)).text_color(theme::error()).child(e),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_servers(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("servers")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(520.))
                    .py(px(48.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .child("Home server"),
                    )
                    .child(div().text_size(px(13.)).text_color(theme::muted()).child(
                        "Home shows Continue Watching and recommendations from this server.",
                    ))
                    .when_some(self.servers_error.clone(), |d, e| {
                        d.child(div().text_color(theme::error()).child(e))
                    })
                    .when(self.servers_loading && self.servers.is_empty(), |d| {
                        d.child(
                            div()
                                .text_color(theme::muted())
                                .child("Looking for servers…"),
                        )
                    })
                    .children(self.server_rows(cx)),
            )
            .into_any_element()
    }

    pub fn server_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let home = self.home_server().map(|s| s.id).unwrap_or_default();
        let choices = self
            .servers
            .iter()
            .map(|s| {
                let sub = if s.is_jellyfin() {
                    "Jellyfin"
                } else if s.owned {
                    "Owned"
                } else {
                    "Shared with you"
                };
                Choice {
                    id: s.id.clone().into(),
                    label: s.name.clone().into(),
                    sub: sub.into(),
                    selected: s.id == home,
                }
            })
            .collect();
        self.choice_list(
            "srv",
            Some(Icon::Server),
            choices,
            |this, id, cx| this.pick_home_server(id, cx),
            cx,
        )
    }
}

pub fn logo() -> impl IntoElement {
    div()
        .size(px(112.))
        .rounded(px(28.))
        .bg(theme::surface())
        .flex()
        .items_center()
        .justify_center()
        .child(icon(Icon::Play, 68., theme::accent()))
}
