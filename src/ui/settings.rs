use crate::app::{Category, Nav, Plaivo, open_url};
use crate::settings::{
    COUNTDOWN_MAX, DENSITY_MAX, EpisodePoster, GridSpacing, Settings, SkipMode, ViewMode,
};
use crate::ui::licenses::LICENSES;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{
    chip, icon, nav_row, overlay_button, pill, round_button, row, scrollbar, section, stepper,
    toggle,
};
use crate::{logging, plex, shaders};
use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, MouseMoveEvent, SharedString, Stateful,
    Window, div, prelude::*, px,
};

const MPV_SOURCE: &str = "https://github.com/mpv-player/mpv";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MPV_VERSION: &str = env!("PLAIVO_MPV_VERSION");
const MPV_INFO: &str = concat!(
    "mpv ",
    env!("PLAIVO_MPV_VERSION"),
    " · FFmpeg ",
    env!("PLAIVO_FFMPEG_VERSION"),
    " · LGPL-2.1-or-later"
);
const CATEGORIES: [Category; 10] = [
    Category::General,
    Category::Appearance,
    Category::Playback,
    Category::Audio,
    Category::Services,
    Category::WatchTogether,
    Category::Downloads,
    Category::Connections,
    Category::Advanced,
    Category::About,
];
const DENSITY: [&str; DENSITY_MAX as usize] = ["Smallest", "Small", "Medium", "Large", "Largest"];

pub fn nudge(value: u32, step: i8, by: u32, lo: u32, hi: u32, default: u32) -> u32 {
    if step == 0 {
        default
    } else {
        (value as i64 + step as i64 * by as i64).clamp(lo as i64, hi as i64) as u32
    }
}

fn density_label(density: u32) -> &'static str {
    DENSITY[density.clamp(1, DENSITY_MAX) as usize - 1]
}

impl Plaivo {
    fn summary(&self, c: Category) -> String {
        match c {
            Category::General => self.home_server().map_or("No home server".into(), |s| {
                format!("Home server: {}", s.name)
            }),
            Category::Appearance => format!(
                "Theme: {} · Poster size: {}",
                self.settings.theme.label(),
                density_label(self.settings.density)
            ),
            Category::Playback => format!(
                "Default quality: {} · Autoplay {}",
                quality_label(self.settings.quality),
                if self.settings.autoplay_next {
                    "on"
                } else {
                    "off"
                }
            ),
            Category::Advanced => format!(
                "Image cache · Debug logging {}",
                if self.settings.debug_logging {
                    "on"
                } else {
                    "off"
                }
            ),
            Category::Audio => self.languages_summary(),
            Category::Downloads => self.download_summary(),
            Category::Services => self.services.summary(),
            Category::WatchTogether => self.watch_summary(),
            Category::Libraries
            | Category::Shaders
            | Category::AudioLanguage
            | Category::SubtitleLanguage
            | Category::LanguageOverride
            | Category::PlaybackOverride => String::new(),
            Category::Connections => {
                let count = |n: usize| {
                    if n == 1 {
                        "1 server".to_owned()
                    } else {
                        format!("{n} servers")
                    }
                };
                let plex = if self.account.token.is_some() {
                    count(self.account.servers.len())
                } else {
                    "Not signed in".into()
                };
                format!(
                    "Plex: {plex} · Jellyfin: {}",
                    count(self.account.jellyfin.len())
                )
            }
            Category::About => match self.updates.summary() {
                Some(update) => format!("Version {VERSION} · {update}"),
                None => format!("Version {VERSION} · libmpv {MPV_VERSION}"),
            },
        }
    }

    fn render_playback_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let s = &self.settings;
        let quality = self.quality_choices(s.quality, "Direct play, no transcoding".into());
        let skip_row = |id: &'static str,
                        title: &'static str,
                        detail: &'static str,
                        current: SkipMode,
                        credits: bool,
                        cx: &mut Context<Self>| {
            row(id, None, title, detail).child(div().flex().gap(px(6.)).children(
                SkipMode::ALL.map(|m| {
                    chip((id, m as usize), m.label(), m == current).on_click(cx.listener(
                        move |this, _, _, cx| {
                            if credits {
                                this.settings.skip_credits = m
                            } else {
                                this.settings.skip_intro = m
                            }
                            this.save_settings_soon(cx);
                            cx.notify();
                        },
                    ))
                }),
            ))
        };
        let seek_row = |id: &'static str,
                        title: &'static str,
                        detail: &'static str,
                        value: u32,
                        large: bool,
                        cx: &mut Context<Self>| {
            row(id, None, title, detail).child(stepper(
                id,
                format!("{value} s"),
                cx.listener(move |this, step: &i8, _, cx| {
                    let s = &mut this.settings;
                    if large {
                        s.seek_large = nudge(s.seek_large, *step, 10, 10, 600, 30)
                    } else {
                        s.seek_small = nudge(s.seek_small, *step, 5, 5, 60, 10)
                    }
                    this.save_settings_soon(cx);
                    cx.notify();
                }),
            ))
        };
        let up_next = section("Up next")
            .child(
                toggle_row(
                    "autoplay",
                    "Autoplay next episode",
                    "Plays the next episode in the queue when one ends",
                    s.autoplay_next,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.autoplay_next = !this.settings.autoplay_next;
                    this.save_settings_soon(cx);
                    cx.notify();
                })),
            )
            .when(s.autoplay_next, |d| {
                d.child(
                    row(
                        "countdown",
                        None,
                        "Countdown",
                        "Seconds before the next video plays",
                    )
                    .child(stepper(
                        "countdown",
                        format!("{} s", s.autoplay_countdown),
                        cx.listener(|this, step: &i8, _, cx| {
                            this.settings.autoplay_countdown = nudge(
                                this.settings.autoplay_countdown,
                                *step,
                                1,
                                0,
                                COUNTDOWN_MAX,
                                5,
                            );
                            this.save_settings_soon(cx);
                            cx.notify();
                        }),
                    )),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(up_next)
            .child(
                section("Skip markers")
                    .child(skip_row(
                        "skip-intro",
                        "Intro",
                        "Opening titles and recaps",
                        s.skip_intro,
                        false,
                        cx,
                    ))
                    .child(skip_row(
                        "skip-credits",
                        "Credits",
                        "End credits",
                        s.skip_credits,
                        true,
                        cx,
                    )),
            )
            .child(
                section("Seeking")
                    .child(seek_row(
                        "seek-small",
                        "Small step",
                        "← and → keys, and the seek buttons",
                        s.seek_small,
                        false,
                        cx,
                    ))
                    .child(seek_row(
                        "seek-large",
                        "Large step",
                        "Shift + ← and Shift + →",
                        s.seek_large,
                        true,
                        cx,
                    )),
            )
            .child(
                section("Player").child(
                    toggle_row(
                        "stop-on-close",
                        "Close stops playback",
                        "Close and Esc stop playback instead of minimizing",
                        s.stop_on_close,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.stop_on_close = !this.settings.stop_on_close;
                        this.save_settings_soon(cx);
                        cx.notify();
                    })),
                ),
            )
            .child(
                section("Video").child(
                    nav_row(
                        "shader",
                        Icon::Sparkle,
                        "Default shader",
                        shaders::name(&s.shader),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.navigate(Nav::Settings(Some(Category::Shaders)), true, cx)
                    })),
                ),
            )
            .child(
                section("Default quality")
                    .children(self.choice_list(
                        "quality",
                        None,
                        quality,
                        |this, id, cx| this.set_default_quality(id.parse().unwrap_or(0), cx),
                        cx,
                    )),
            )
            .child(self.render_playback_rules(cx))
    }

    fn render_library_manager(&mut self, cx: &mut Context<Self>) -> Div {
        let groups = self.sections_in_order(true);
        let mut body = div().flex().flex_col().gap(px(28.));
        for (server, rows) in groups {
            let last = rows.len().saturating_sub(1);
            body = body.child(section(server.to_owned()).children(
                rows.into_iter().enumerate().map(|(i, x)| {
                    let id = x.id.clone();
                    let hidden = self.settings.library_hidden(&id);
                    let mover = |n: usize, i_: Icon, step: isize, enabled: bool| {
                        let id = id.clone();
                        overlay_button(
                            (SharedString::from(format!("move-{id}")), n),
                            i_,
                            36.,
                            22.,
                            theme::black(0.),
                            theme::tint(0.08),
                            theme::text(),
                        )
                        .when(!enabled, |d| d.opacity(0.3).cursor_default())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if enabled {
                                this.move_library(&id, step, cx);
                            }
                        }))
                    };
                    let up = mover(0, Icon::ExpandLess, -1, i > 0);
                    let down = mover(1, Icon::ExpandMore, 1, i < last);
                    row(
                        SharedString::from(format!("lib-{id}")),
                        Some(if x.kind == "movie" {
                            Icon::Movie
                        } else {
                            Icon::Tv
                        }),
                        x.title.clone(),
                        if hidden { "Hidden" } else { "" },
                    )
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.12)))
                    .child(up)
                    .child(down)
                    .child(toggle(!hidden))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.set_library_hidden(&id, !hidden);
                        this.save_settings_soon(cx);
                        cx.notify();
                    }))
                }),
            ));
        }
        body
    }

    fn move_library(&mut self, id: &str, step: isize, cx: &mut Context<Self>) {
        let arranged: Vec<String> = self
            .sections_in_order(true)
            .into_iter()
            .flat_map(|(_, rows)| rows.into_iter().map(|r| r.id.clone()))
            .collect();
        self.settings.move_library(&arranged, id, step);
        self.save_settings_soon(cx);
        cx.notify();
    }

    fn render_advanced_settings(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                section("Storage")
                    .child(
                        row(
                            "clear-cache",
                            Some(Icon::Refresh),
                            "Clear image cache",
                            "Removes cached artwork and library data",
                        )
                        .child(
                            pill("clear-cache-button", "Clear", None, false)
                                .on_click(cx.listener(|this, _, _, cx| this.clear_image_cache(cx))),
                        ),
                    )
                    .child(
                        row(
                            "reset-data",
                            Some(Icon::Delete),
                            "Delete all data",
                            "Signs you out and deletes settings, cache, logs and downloads",
                        )
                        .child(
                            pill("reset-data-button", "Delete", None, false).on_click(
                                cx.listener(|this, _, window, cx| this.confirm_reset(window, cx)),
                            ),
                        ),
                    ),
            )
            .child(
                section("Logging")
                    .child(
                        toggle_row(
                            "debug-logging",
                            "Debug logging",
                            "Useful when reporting a problem",
                            self.settings.debug_logging,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.debug_logging = !this.settings.debug_logging;
                            this.save_settings_soon(cx);
                            logging::set_debug(this.settings.debug_logging);
                            if let Some(m) = &this.player.mpv {
                                m.sync_log_level();
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        nav_row(
                            "logs",
                            Icon::Info,
                            "Logs",
                            "Kept in memory until Plaivo closes; export to share them",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Logs, true, cx))),
                    ),
            )
    }

    pub fn render_settings_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let category = self.settings_page;
        let body = match category {
            None => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .children(CATEGORIES.map(|c| {
                    let entry = if c == Category::About && self.updates.attention() {
                        row(c.title(), Some(c.icon()), c.title(), self.summary(c))
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::tint(0.12)))
                            .child(
                                pill("about-restart-to-update", "Restart to update", None, true)
                                    .occlude()
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.restart_to_update(cx)),
                                    ),
                            )
                            .child(icon(Icon::ChevronRight, 22., theme::muted()))
                    } else {
                        nav_row(c.title(), c.icon(), c.title(), self.summary(c))
                    };
                    entry.on_click(cx.listener(move |this, _, _, cx| {
                        if c == Category::Audio {
                            this.sync_languages(cx);
                        }
                        this.navigate(Nav::Settings(Some(c)), true, cx)
                    }))
                })),
            Some(Category::General) => div()
                .flex()
                .flex_col()
                .gap(px(28.))
                .child(
                    section("Home server")
                        .children(self.server_rows(cx))
                        .when(self.servers.is_empty(), |d| {
                            d.child(note("No servers found."))
                        }),
                )
                .child(
                    section("Sidebar").child(
                        nav_row(
                            "libraries",
                            Icon::Sort,
                            "Manage libraries",
                            "Reorder and hide the libraries in the sidebar",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.navigate(Nav::Settings(Some(Category::Libraries)), true, cx)
                        })),
                    ),
                ),
            Some(Category::Libraries) => self.render_library_manager(cx),
            Some(Category::Appearance) => self.render_appearance(cx),
            Some(Category::Playback) => self.render_playback_settings(cx),
            Some(Category::Shaders) => {
                let choices = self.shader_choices(&self.settings.shader);
                section("Default shader")
                    .children(self.choice_list(
                        "shader",
                        None,
                        choices,
                        |this, id, cx| {
                            this.settings.shader = id.to_owned();
                            this.save_settings_soon(cx);
                        },
                        cx,
                    ))
            }
            Some(Category::Audio) => self.render_audio_settings(cx),
            Some(Category::AudioLanguage) => self.render_language_list(false, cx),
            Some(Category::SubtitleLanguage) => self.render_language_list(true, cx),
            Some(Category::LanguageOverride) => self.render_language_override(cx),
            Some(Category::PlaybackOverride) => self.render_playback_override(cx),
            Some(Category::Downloads) => self.render_download_settings(cx),
            Some(Category::Services) => self.render_services(cx),
            Some(Category::WatchTogether) => self.render_watch_settings(window, cx),
            Some(Category::Advanced) => self.render_advanced_settings(cx),
            Some(Category::Connections) => self.render_connections(cx),
            Some(Category::About) => div()
                .flex()
                .flex_col()
                .gap(px(28.))
                .child(self.render_updates(cx))
                .child(
                    section("Plaivo")
                        .child(row(
                            "version",
                            Some(Icon::PlayCircle),
                            "Plaivo",
                            format!("Version {VERSION}"),
                        ))
                        .child(
                            row("libmpv", Some(Icon::Movie), "libmpv", MPV_INFO).child(
                                pill("mpv-source", "Source", Some(Icon::OpenInNew), false)
                                    .on_click(|_, _, _| open_url(MPV_SOURCE)),
                            ),
                        )
                        .child(row(
                            "trakt",
                            Some(Icon::Check),
                            "Trakt",
                            "Plaivo uses Trakt but is not endorsed or certified by Trakt",
                        ))
                        .child(
                            nav_row(
                                "licenses",
                                Icon::Info,
                                "Open-source licenses",
                                format!("libmpv, FFmpeg and {} more", LICENSES.len() - 2),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.navigate(Nav::Licenses(None), true, cx)
                            })),
                        ),
                ),
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .when(category.is_some(), |d| {
                d.child(
                    round_button(
                        "settings-back",
                        Icon::Back,
                        40.,
                        22.,
                        theme::black(0.),
                        theme::tint(0.08),
                        theme::text(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
                )
            })
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .child(category.map_or("Settings", Category::title)),
            )
            .when(category == Some(Category::LanguageOverride), |d| {
                d.child(div().flex_1()).children(self.rule_actions(cx))
            })
            .when(category == Some(Category::PlaybackOverride), |d| {
                d.child(div().flex_1())
                    .children(self.playback_rule_actions(cx))
            });
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id(("settings", category.map_or(0, |c| c as usize + 1)))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.settings_scroll)
                    .child(
                        div()
                            .w_full()
                            .px(px(32.))
                            .py(px(32.))
                            .flex()
                            .flex_col()
                            .gap(px(28.))
                            .child(header)
                            .child(body),
                    ),
            )
            .child(scrollbar(&self.settings_scroll))
            .into_any_element()
    }
}

impl Category {
    fn title(self) -> &'static str {
        match self {
            Category::General => "General",
            Category::Libraries => "Manage libraries",
            Category::Appearance => "Appearance",
            Category::Playback => "Playback",
            Category::Shaders => "Default shader",
            Category::Audio => "Audio & Subtitles",
            Category::AudioLanguage => "Audio language",
            Category::SubtitleLanguage => "Subtitle language",
            Category::LanguageOverride => "Language override",
            Category::PlaybackOverride => "Playback override",
            Category::Downloads => "Downloads",
            Category::Services => "Services",
            Category::WatchTogether => "Watch Together",
            Category::Connections => "Connections",
            Category::Advanced => "Advanced",
            Category::About => "About",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Category::General | Category::Libraries => Icon::Settings,
            Category::Appearance => Icon::Palette,
            Category::Playback | Category::Shaders | Category::PlaybackOverride => Icon::PlayCircle,
            Category::Audio
            | Category::AudioLanguage
            | Category::SubtitleLanguage
            | Category::LanguageOverride => Icon::Subtitles,
            Category::Downloads => Icon::Download,
            Category::Services => Icon::Check,
            Category::WatchTogether => Icon::Group,
            Category::Connections => Icon::Link,
            Category::Advanced => Icon::Build,
            Category::About => Icon::Info,
        }
    }
}

struct CardToggle {
    id: &'static str,
    title: &'static str,
    detail: &'static str,
    on: bool,
    field: fn(&mut Settings) -> &mut bool,
    rebuild: bool,
}

fn option_block(i: Icon, title: &'static str, body: impl IntoElement) -> Div {
    div()
        .w_full()
        .p(px(16.))
        .rounded(px(12.))
        .bg(theme::surface())
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(16.))
                .child(icon(i, 24., theme::muted()))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                ),
        )
        .child(body)
}

impl Plaivo {
    fn segmented<T: Copy + PartialEq + 'static>(
        &self,
        id: &'static str,
        options: &[(T, &'static str)],
        current: T,
        set: fn(&mut Self, T, &mut Context<Self>),
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .flex()
            .gap(px(4.))
            .p(px(4.))
            .rounded_full()
            .bg(theme::tint(0.06))
            .children(options.iter().enumerate().map(|(i, &(value, label))| {
                let selected = value == current;
                div()
                    .id((id, i))
                    .flex_1()
                    .h(px(36.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(px(14.))
                    .cursor_pointer()
                    .when(selected, |d| {
                        d.bg(theme::text())
                            .text_color(theme::bg())
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!selected, |d| d.hover(|s| s.bg(theme::tint(0.08))))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| set(this, value, cx)))
            }))
    }

    fn set_density(&mut self, d: u32, cx: &mut Context<Self>) {
        if self.settings.density != d {
            self.settings.density = d;
            self.save_settings_soon(cx);
            cx.notify();
        }
    }

    fn density_slider(&self, cx: &mut Context<Self>) -> Div {
        let at = self.settings.density.clamp(1, DENSITY_MAX);
        let frac = (at - 1) as f32 / (DENSITY_MAX - 1) as f32;
        let stops = (0..DENSITY_MAX).map(|i| {
            let filled = i < at - 1;
            div()
                .absolute()
                .top(px(10.))
                .left(gpui::relative(i as f32 / (DENSITY_MAX - 1) as f32))
                .ml(px(-2.))
                .size(px(4.))
                .rounded_full()
                .bg(if filled {
                    theme::bg().into()
                } else {
                    theme::muted()
                })
        });
        let zones = (1..=DENSITY_MAX).map(|d| {
            div()
                .id(("density-stop", d as usize))
                .flex_1()
                .h_full()
                .cursor_pointer()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.set_density(d, cx)),
                )
                .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                    if ev.dragging() {
                        this.set_density(d, cx);
                    }
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .relative()
                    .h(px(24.))
                    .mx(px(4.))
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(9.))
                            .h(px(6.))
                            .rounded_full()
                            .bg(theme::tint(0.12)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(9.))
                            .h(px(6.))
                            .w(gpui::relative(frac))
                            .rounded_full()
                            .bg(theme::text()),
                    )
                    .children(stops)
                    .child(
                        div()
                            .absolute()
                            .top(px(2.))
                            .left(gpui::relative(frac))
                            .ml(px(-2.))
                            .w(px(4.))
                            .h(px(20.))
                            .rounded(px(2.))
                            .bg(theme::text()),
                    )
                    .child(div().absolute().size_full().flex().children(zones)),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_size(px(12.))
                    .text_color(theme::muted())
                    .child("Compact")
                    .child("Comfortable"),
            )
    }

    fn render_appearance(&mut self, cx: &mut Context<Self>) -> Div {
        let s = &self.settings;
        let toggles = [
            CardToggle {
                id: "episode-numbers",
                title: "Show episode numbers on cards",
                detail: "Show season and episode number on episode cards",
                on: s.episode_numbers,
                field: |s| &mut s.episode_numbers,
                rebuild: true,
            },
            CardToggle {
                id: "season-posters",
                title: "Show season posters on tabs",
                detail: "Show each season's poster above its tab",
                on: s.season_posters,
                field: |s| &mut s.season_posters,
                rebuild: false,
            },
            CardToggle {
                id: "hide-spoilers",
                title: "Hide spoilers for unwatched episodes",
                detail: "Blur thumbnails and hide descriptions of unwatched episodes",
                on: s.hide_spoilers,
                field: |s| &mut s.hide_spoilers,
                rebuild: true,
            },
            CardToggle {
                id: "watched-indicators",
                title: "Show watched indicators",
                detail: "Show a checkmark on watched movies, shows and episodes",
                on: s.watched_indicators,
                field: |s| &mut s.watched_indicators,
                rebuild: false,
            },
        ];
        let toggles: Vec<_> = toggles
            .into_iter()
            .map(
                |CardToggle {
                     id,
                     title,
                     detail,
                     on,
                     field,
                     rebuild,
                 }| {
                    toggle_row(id, title, detail, on).on_click(cx.listener(
                        move |this, _, _, cx| {
                            let v = field(&mut this.settings);
                            *v = !*v;
                            if rebuild {
                                this.cards_changed(cx);
                            } else {
                                this.save_settings_soon(cx);
                                cx.notify();
                            }
                        },
                    ))
                },
            )
            .collect();
        let view = self.segmented(
            "view-mode",
            &ViewMode::ALL.map(|m| (m, m.label())),
            s.view_mode,
            |this, m, cx| {
                this.settings.view_mode = m;
                this.cards_changed(cx);
            },
            cx,
        );
        let spacing = self.segmented(
            "grid-spacing",
            &GridSpacing::ALL.map(|g| (g, g.label())),
            s.grid_spacing,
            |this, g, cx| {
                this.settings.grid_spacing = g;
                this.save_settings_soon(cx);
                cx.notify();
            },
            cx,
        );
        let poster = self.segmented(
            "episode-poster",
            &EpisodePoster::ALL.map(|p| (p, p.label())),
            s.episode_poster,
            |this, p, cx| {
                this.settings.episode_poster = p;
                this.cards_changed(cx);
            },
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                section("Display").child(
                    row(
                        "theme",
                        Some(Icon::Contrast),
                        "Theme",
                        self.settings.theme.label(),
                    )
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.12)))
                    .child(icon(Icon::ChevronRight, 22., theme::muted()))
                    .on_click(cx.listener(|this, _, _, cx| this.open_theme_picker(cx))),
                ),
            )
            .child(
                section("Library & cards")
                    .child(option_block(Icon::ViewList, "View mode", view))
                    .child(option_block(
                        Icon::GridView,
                        "Library density",
                        self.density_slider(cx),
                    ))
                    .child(option_block(Icon::ScaleFill, "Grid spacing", spacing))
                    .child(option_block(Icon::Image, "Episode poster style", poster))
                    .children(toggles),
            )
    }
}

pub fn quality_label(kbps: u32) -> String {
    plex::Quality::find(kbps).map_or("Original".into(), |q| {
        format!("{} · {}", q.label, plex::fmt_bitrate(q.kbps as u64))
    })
}

pub fn note(text: &'static str) -> Div {
    div()
        .pb(px(4.))
        .text_size(px(13.))
        .text_color(theme::muted())
        .child(text)
}

pub fn toggle_row(
    id: impl Into<gpui::ElementId>,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    on: bool,
) -> Stateful<Div> {
    row(id, None, title, detail)
        .cursor_pointer()
        .hover(|s| s.bg(theme::tint(0.12)))
        .child(toggle(on))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steppers_clamp_and_reset() {
        assert_eq!(nudge(10, 1, 5, 5, 60, 10), 15);
        assert_eq!(nudge(60, 1, 5, 5, 60, 10), 60);
        assert_eq!(nudge(5, -1, 5, 5, 60, 10), 5);
        assert_eq!(nudge(0, -1, 1, 0, 30, 5), 0);
        assert_eq!(nudge(25, 0, 5, 5, 60, 10), 10);
        assert_eq!(
            (density_label(0), density_label(3), density_label(9)),
            ("Smallest", "Medium", "Largest")
        );
    }
}
