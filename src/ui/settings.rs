use crate::app::{Category, Nav, Plaivo, open_url};
use crate::settings::{COUNTDOWN_MAX, DENSITY_MAX, SkipMode, library_key};
use crate::ui::licenses::LICENSES;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{chip, nav_row, pill, round_button, row, section, stepper, toggle};
use crate::{logging, mpv, plex, shaders};
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, Stateful, div, prelude::*, px};

const MPV_SOURCE: &str = "https://github.com/mpv-player/mpv";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const CATEGORIES: [Category; 8] = [Category::General, Category::Appearance, Category::Playback, Category::Audio, Category::Downloads, Category::Connections, Category::Advanced, Category::About];
const DENSITY: [&str; DENSITY_MAX as usize] = ["Smallest", "Small", "Medium", "Large", "Largest"];

pub fn nudge(value: u32, step: i8, by: u32, lo: u32, hi: u32, default: u32) -> u32 {
    if step == 0 { default } else { (value as i64 + step as i64 * by as i64).clamp(lo as i64, hi as i64) as u32 }
}

fn density_label(density: u32) -> &'static str {
    DENSITY[density.clamp(1, DENSITY_MAX) as usize - 1]
}

impl Plaivo {
    pub fn load_mpv_build(&mut self, cx: &mut Context<Self>) {
        if self.mpv_build.is_some() || self.mpv_build_task.is_some() {
            return;
        }
        let job = cx.background_executor().spawn(async { mpv::build_info() });
        self.mpv_build_task = Some(cx.spawn(async move |this, cx| {
            let build = job.await;
            let _ = this.update(cx, |this, cx| {
                this.mpv_build = Some(build);
                this.mpv_build_task = None;
                cx.notify();
            });
        }));
    }

    fn mpv_info(&self) -> SharedString {
        match &self.mpv_build {
            None => "Checking…".into(),
            Some(Ok(b)) => format!("{} · FFmpeg {} · {}", b.mpv, b.ffmpeg, if b.gpl { "GPL-2.0-or-later" } else { "LGPL-2.1-or-later" }).into(),
            Some(Err(e)) => e.clone().into(),
        }
    }

    fn summary(&self, c: Category) -> String {
        match c {
            Category::General => self.home_server().map_or("No home server".into(), |s| format!("Home server: {}", s.name)),
            Category::Appearance => format!("Poster size: {}", density_label(self.settings.density)),
            Category::Playback => format!("Default quality: {} · Autoplay {}", quality_label(self.settings.quality), if self.settings.autoplay_next { "on" } else { "off" }),
            Category::Advanced => format!("Image cache · Debug logging {}", if self.settings.debug_logging { "on" } else { "off" }),
            Category::Audio => self.languages_summary(),
            Category::Downloads => self.download_summary(),
            Category::Libraries | Category::Shaders | Category::AudioLanguage | Category::SubtitleLanguage => String::new(),
            Category::Connections => {
                let count = |n: usize| if n == 1 { "1 server".to_owned() } else { format!("{n} servers") };
                let plex = if self.account.token.is_some() { count(self.account.servers.len()) } else { "Not signed in".into() };
                format!("Plex: {plex} · Jellyfin: {}", count(self.account.jellyfin.len()))
            }
            Category::About => match &self.mpv_build {
                Some(Ok(b)) => format!("Version {VERSION} · libmpv {}", b.mpv),
                _ => format!("Version {VERSION}"),
            },
        }
    }

    fn render_playback_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let s = &self.settings;
        let quality = self.quality_choices("Direct play, no transcoding".into());
        let skip_row = |id: &'static str, title: &'static str, detail: &'static str, current: SkipMode, credits: bool, cx: &mut Context<Self>| {
            row(id, None, title, detail).child(div().flex().gap(px(6.)).children(SkipMode::ALL.map(|m| {
                chip((id, m as usize), m.label(), m == current).on_click(cx.listener(move |this, _, _, cx| {
                    if credits { this.settings.skip_credits = m } else { this.settings.skip_intro = m }
                    this.settings.save();
                    cx.notify();
                }))
            })))
        };
        let seek_row = |id: &'static str, title: &'static str, detail: &'static str, value: u32, large: bool, cx: &mut Context<Self>| {
            row(id, None, title, detail).child(stepper(
                id,
                format!("{value} s"),
                cx.listener(move |this, step: &i8, _, cx| {
                    let s = &mut this.settings;
                    if large { s.seek_large = nudge(s.seek_large, *step, 10, 10, 600, 30) } else { s.seek_small = nudge(s.seek_small, *step, 5, 5, 60, 10) }
                    s.save();
                    cx.notify();
                }),
            ))
        };
        let up_next = section("Up next")
            .child(toggle_row("autoplay", "Autoplay next episode", "Plays the next episode in the queue when one ends", s.autoplay_next).on_click(cx.listener(|this, _, _, cx| {
                this.settings.autoplay_next = !this.settings.autoplay_next;
                this.settings.save();
                cx.notify();
            })))
            .when(s.autoplay_next, |d| {
                d.child(row("countdown", None, "Countdown", "Seconds the Up next card waits before playing. 0 plays at once.").child(stepper(
                    "countdown",
                    format!("{} s", s.autoplay_countdown),
                    cx.listener(|this, step: &i8, _, cx| {
                        this.settings.autoplay_countdown = nudge(this.settings.autoplay_countdown, *step, 1, 0, COUNTDOWN_MAX, 5);
                        this.settings.save();
                        cx.notify();
                    }),
                )))
            });
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(up_next)
            .child(
                section("Skip markers")
                    .child(note("Button shows a skip button. Auto skips on its own."))
                    .child(skip_row("skip-intro", "Intro", "Opening titles and recaps", s.skip_intro, false, cx))
                    .child(skip_row("skip-credits", "Credits", "End credits", s.skip_credits, true, cx)),
            )
            .child(
                section("Seeking")
                    .child(seek_row("seek-small", "Small step", "← and → keys, and the seek buttons", s.seek_small, false, cx))
                    .child(seek_row("seek-large", "Large step", "Shift + ← and Shift + →", s.seek_large, true, cx)),
            )
            .child(section("Video").child(
                nav_row("shader", Icon::Sparkle, "Default shader", shaders::name(&s.shader)).on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Settings(Some(Category::Shaders)), true, cx))),
            ))
            .child(
                section("Default quality")
                    .child(note("Videos play at this quality unless the file is already smaller. Changing quality in the player updates it too."))
                    .children(self.choice_list("quality", None, quality, |this, id, cx| this.set_quality(id.parse().unwrap_or(0), cx), cx)),
            )
    }

    fn render_library_manager(&mut self, cx: &mut Context<Self>) -> Div {
        let groups = self.sections_in_order(true);
        let mut body = div().flex().flex_col().gap(px(28.)).child(note("Order and visibility are saved for each library on each server."));
        for (server, rows) in groups {
            let last = rows.len().saturating_sub(1);
            body = body.child(section(server).children(rows.into_iter().enumerate().map(|(i, (srv, key, title, movie))| {
                let id = library_key(&srv, &key);
                let hidden = self.settings.library_hidden(&id);
                let mover = |n: usize, i_: Icon, step: isize, enabled: bool| {
                    let id = id.clone();
                    round_button((SharedString::from(format!("move-{id}")), n), i_, 36., 22., theme::black(0.), theme::tint(0.08), theme::text())
                        .when(!enabled, |d| d.opacity(0.3).cursor_default())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if enabled {
                                this.move_library(&id, step, cx);
                            }
                        }))
                };
                let up = mover(0, Icon::ExpandLess, -1, i > 0);
                let down = mover(1, Icon::ExpandMore, 1, i < last);
                row(SharedString::from(format!("lib-{id}")), Some(if movie { Icon::Movie } else { Icon::Tv }), title, if hidden { "Hidden" } else { "" })
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.12)))
                    .child(up)
                    .child(down)
                    .child(toggle(!hidden))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings.set_library_hidden(&id, !hidden);
                        this.settings.save();
                        cx.notify();
                    }))
            })));
        }
        body
    }

    fn move_library(&mut self, id: &str, step: isize, cx: &mut Context<Self>) {
        let arranged: Vec<String> = self.sections_in_order(true).into_iter().flat_map(|(_, rows)| rows.into_iter().map(|r| library_key(&r.0, &r.1))).collect();
        self.settings.move_library(&arranged, id, step);
        self.settings.save();
        cx.notify();
    }

    fn render_advanced_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let logs = logging::log_path().parent().map(|p| p.to_path_buf()).unwrap_or_default();
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(section("Storage").child(
                row("clear-cache", Some(Icon::Refresh), "Clear image cache", "Removes cached artwork and library data. It downloads again when needed.")
                    .child(pill("clear-cache-button", "Clear", None, false).on_click(cx.listener(|this, _, _, cx| this.clear_image_cache(cx)))),
            ))
            .child(
                section("Logging")
                    .child(toggle_row("debug-logging", "Debug logging", "Writes more detail to the log. Turn it on when reporting a problem.", self.settings.debug_logging).on_click(cx.listener(|this, _, _, cx| {
                        this.settings.debug_logging = !this.settings.debug_logging;
                        this.settings.save();
                        logging::set_debug(this.settings.debug_logging);
                        if let Some(m) = &this.player.mpv {
                            m.sync_log_level();
                        }
                        cx.notify();
                    })))
                    .child(row("log-folder", Some(Icon::Info), "Log folder", logs.display().to_string()).child(pill("open-logs", "Open", Some(Icon::OpenInNew), false).on_click(cx.listener(move |_, _, _, cx| cx.open_with_system(&logs))))),
            )
    }

    pub fn render_settings_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let category = self.settings_page;
        let body = match category {
            None => div().flex().flex_col().gap(px(8.)).children(CATEGORIES.map(|c| {
                nav_row(c.title(), c.icon(), c.title(), self.summary(c)).on_click(cx.listener(move |this, _, _, cx| {
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
                        .child(note("Home shows Continue Watching and recommendations from this server. Libraries from every server stay in the sidebar."))
                        .children(self.server_rows(cx))
                        .when(self.servers.is_empty(), |d| d.child(note("No servers found."))),
                )
                .child(section("Sidebar").child(
                    nav_row("libraries", Icon::Sort, "Manage libraries", "Reorder and hide the libraries in the sidebar").on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Settings(Some(Category::Libraries)), true, cx))),
                )),
            Some(Category::Libraries) => self.render_library_manager(cx),
            Some(Category::Appearance) => section("Library").child(row("density", Some(Icon::Movie), "Poster size", "Size of posters on Home, in libraries and in search").child(stepper(
                "density",
                density_label(self.settings.density),
                cx.listener(|this, step: &i8, _, cx| {
                    this.settings.density = nudge(this.settings.density, *step, 1, 1, DENSITY_MAX, 3);
                    this.settings.save();
                    cx.notify();
                }),
            ))),
            Some(Category::Playback) => self.render_playback_settings(cx),
            Some(Category::Shaders) => {
                let choices = self.shader_choices(&self.settings.shader);
                section("Default shader")
                    .child(note("Applied to every video. Change it for one video from the player's settings. NVScaler turns itself off for HDR video."))
                    .children(self.choice_list("shader", None, choices, |this, id, _| {
                        this.settings.shader = id.to_owned();
                        this.settings.save();
                    }, cx))
            }
            Some(Category::Audio) => self.render_audio_settings(cx),
            Some(Category::AudioLanguage) => self.render_language_list(false, cx),
            Some(Category::SubtitleLanguage) => self.render_language_list(true, cx),
            Some(Category::Downloads) => self.render_download_settings(cx),
            Some(Category::Advanced) => self.render_advanced_settings(cx),
            Some(Category::Connections) => self.render_connections(cx),
            Some(Category::About) => section("Plaivo")
                .child(row("version", Some(Icon::PlayCircle), "Plaivo", format!("Version {VERSION}")))
                .child(row("libmpv", Some(Icon::Movie), "libmpv", self.mpv_info()).child(pill("mpv-source", "Source", Some(Icon::OpenInNew), false).on_click(|_, _, _| open_url(MPV_SOURCE))))
                .child(
                    nav_row("licenses", Icon::Info, "Open-source licenses", format!("libmpv, FFmpeg and {} more", LICENSES.len() - 2))
                        .on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Licenses(None), true, cx))),
                ),
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .when(category.is_some(), |d| {
                d.child(round_button("settings-back", Icon::Back, 40., 22., theme::black(0.), theme::tint(0.08), theme::text()).on_click(cx.listener(|this, _, _, cx| this.back(cx))))
            })
            .child(div().text_size(px(28.)).font_weight(FontWeight::BOLD).child(category.map_or("Settings", Category::title)));
        div()
            .id(("settings", category.map_or(0, |c| c as usize + 1)))
            .size_full()
            .overflow_y_scroll()
            .child(div().w_full().px(px(32.)).py(px(32.)).flex().flex_col().gap(px(28.)).child(header).child(body))
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
            Category::Downloads => "Downloads",
            Category::Connections => "Connections",
            Category::Advanced => "Advanced",
            Category::About => "About",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Category::General | Category::Libraries => Icon::Settings,
            Category::Appearance => Icon::Palette,
            Category::Playback | Category::Shaders => Icon::PlayCircle,
            Category::Audio | Category::AudioLanguage | Category::SubtitleLanguage => Icon::Subtitles,
            Category::Downloads => Icon::Download,
            Category::Connections => Icon::Link,
            Category::Advanced => Icon::Build,
            Category::About => Icon::Info,
        }
    }
}

fn quality_label(kbps: u32) -> String {
    plex::Quality::find(kbps).map_or("Original".into(), |q| format!("{} · {}", q.label, plex::fmt_bitrate(q.kbps as u64)))
}

pub fn note(text: &'static str) -> Div {
    div().pb(px(4.)).text_size(px(13.)).text_color(theme::muted()).child(text)
}

pub fn toggle_row(id: impl Into<gpui::ElementId>, title: impl Into<SharedString>, detail: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    row(id, None, title, detail).cursor_pointer().hover(|s| s.bg(theme::tint(0.12))).child(toggle(on))
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
        assert_eq!((density_label(0), density_label(3), density_label(9)), ("Smallest", "Medium", "Largest"));
    }
}
