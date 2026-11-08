use crate::app::{Category, Nav, Plaivo, open_url};
use crate::plex;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, nav_row, pill, round_button, row, section};
use crate::{mpv, store};
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, div, prelude::*, px};

pub const NOTICES: &str = include_str!(concat!(env!("OUT_DIR"), "/THIRD-PARTY-NOTICES.txt"));
const CRATE_COUNT: &str = env!("PLAIVO_CRATE_COUNT");
const MPV_SOURCE: &str = "https://github.com/mpv-player/mpv";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const CATEGORIES: [Category; 4] = [Category::General, Category::Playback, Category::Connections, Category::About];

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

    fn open_notices(&mut self, cx: &mut Context<Self>) {
        let dir = store::cache_dir();
        let path = dir.join("THIRD-PARTY-NOTICES.txt");
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, NOTICES)) {
            Ok(()) => cx.open_with_system(&path),
            Err(e) => self.toast(format!("Could not open the licenses: {e}"), cx),
        }
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
            Category::Playback => format!("Default quality: {}", quality_label(self.settings.quality)),
            Category::Connections => match self.servers.len() {
                1 => "Plex account · 1 server".into(),
                n => format!("Plex account · {n} servers"),
            },
            Category::About => match &self.mpv_build {
                Some(Ok(b)) => format!("Version {VERSION} · libmpv {}", b.mpv),
                _ => format!("Version {VERSION}"),
            },
        }
    }

    pub fn render_settings_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let category = self.settings_page;
        let body = match category {
            None => div().flex().flex_col().gap(px(8.)).children(CATEGORIES.map(|c| {
                nav_row(c.title(), c.icon(), c.title(), self.summary(c)).on_click(cx.listener(move |this, _, _, cx| this.navigate(Nav::Settings(Some(c)), true, cx)))
            })),
            Some(Category::General) => section("Home server")
                .child(note("Home shows Continue Watching and recommendations from this server. Libraries from every server stay in the sidebar."))
                .children(self.server_rows(cx))
                .when(self.servers.is_empty(), |d| d.child(note("No servers found."))),
            Some(Category::Playback) => {
                let choices = self.quality_choices("Direct play, no transcoding".into());
                section("Default quality")
                    .child(note("Videos play at this quality unless the file is already smaller. Changing quality in the player updates it too."))
                    .children(self.choice_list("quality", None, choices, |this, id, cx| this.set_quality(id.parse().unwrap_or(0), cx), cx))
            }
            Some(Category::Connections) => section("Plex").child(
                row("plex-account", Some(Icon::Person), "Plex account", "Signed in").child(pill("sign-out", "Sign out", Some(Icon::Logout), false).on_click(cx.listener(|this, _, _, cx| this.sign_out(cx)))),
            ),
            Some(Category::About) => section("Plaivo")
                .child(row("version", Some(Icon::PlayCircle), "Plaivo", format!("Version {VERSION}")))
                .child(row("libmpv", Some(Icon::Movie), "libmpv", self.mpv_info()).child(pill("mpv-source", "Source", Some(Icon::OpenInNew), false).on_click(|_, _, _| open_url(MPV_SOURCE))))
                .child(
                    row("licenses", Some(Icon::Info), "Open-source licenses", format!("libmpv, FFmpeg and {CRATE_COUNT} Rust crates"))
                        .child(icon(Icon::OpenInNew, 20., theme::muted()))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::tint(0.12)))
                        .on_click(cx.listener(|this, _, _, cx| this.open_notices(cx))),
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
            Category::Playback => "Playback",
            Category::Connections => "Connections",
            Category::About => "About",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Category::General => Icon::Settings,
            Category::Playback => Icon::PlayCircle,
            Category::Connections => Icon::Link,
            Category::About => Icon::Info,
        }
    }
}

fn quality_label(kbps: u32) -> String {
    plex::Quality::find(kbps).map_or("Original".into(), |q| format!("{} · {}", q.label, plex::fmt_bitrate(q.kbps as u64)))
}

fn note(text: &'static str) -> Div {
    div().pb(px(4.)).text_size(px(13.)).text_color(theme::muted()).child(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_cover_libmpv_and_crates() {
        for needle in ["GNU LESSER GENERAL PUBLIC LICENSE", "Version 2.1, February 1999", "Version 3, 29 June 2007", "https://github.com/mpv-player/mpv", "\ngpui ", "\nserde ", "Apache License"] {
            assert!(NOTICES.contains(needle), "notices miss {needle:?}");
        }
        assert!(CRATE_COUNT.parse::<usize>().unwrap() > 100);
    }
}
