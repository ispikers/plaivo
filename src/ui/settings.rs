use crate::app::{Plaivo, open_url};
use crate::mpv;
use crate::store;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, pill};
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, div, prelude::*, px};

pub const NOTICES: &str = include_str!(concat!(env!("OUT_DIR"), "/THIRD-PARTY-NOTICES.txt"));
const CRATE_COUNT: &str = env!("PLAIVO_CRATE_COUNT");
const MPV_SOURCE: &str = "https://github.com/mpv-player/mpv";

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

    pub fn render_settings_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mpv_info: SharedString = match &self.mpv_build {
            None => "Checking…".into(),
            Some(Ok(b)) => format!("{} · FFmpeg {} · {}", b.mpv, b.ffmpeg, if b.gpl { "GPL-2.0-or-later" } else { "LGPL-2.1-or-later" }).into(),
            Some(Err(e)) => e.clone().into(),
        };
        let servers = self.server_rows(cx);
        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .justify_center()
            .child(
                div()
                    .w_full()
                    .max_w(px(560.))
                    .px(px(24.))
                    .py(px(48.))
                    .flex()
                    .flex_col()
                    .gap(px(36.))
                    .child(div().text_size(px(28.)).font_weight(FontWeight::BOLD).child("Settings"))
                    .child(
                        section("Home server")
                            .child(div().text_size(px(13.)).text_color(theme::muted()).child("Home shows Continue Watching and recommendations from this server. Libraries from every server stay in the sidebar."))
                            .children(servers)
                            .when(self.servers.is_empty(), |d| d.child(div().text_size(px(13.)).text_color(theme::muted()).child("No servers found."))),
                    )
                    .child(
                        section("Account").child(
                            row(Icon::Person, "Plex account", "Signed in").child(pill("sign-out", "Sign out", Some(Icon::Logout), false).on_click(cx.listener(|this, _, _, cx| this.sign_out(cx)))),
                        ),
                    )
                    .child(
                        section("About")
                            .child(row(Icon::PlayCircle, "Plaivo", format!("Version {}", env!("CARGO_PKG_VERSION"))))
                            .child(row(Icon::Movie, "libmpv", mpv_info).child(pill("mpv-source", "Source", Some(Icon::OpenInNew), false).on_click(|_, _, _| open_url(MPV_SOURCE))))
                            .child(
                                row(Icon::Info, "Open-source licenses", format!("libmpv, FFmpeg and {CRATE_COUNT} Rust crates"))
                                    .child(icon(Icon::OpenInNew, 20., theme::muted()))
                                    .id("licenses")
                                    .cursor_pointer()
                                    .hover(|s| s.bg(theme::tint(0.12)))
                                    .on_click(cx.listener(|this, _, _, cx| this.open_notices(cx))),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn section(title: &'static str) -> Div {
    div().flex().flex_col().gap(px(12.)).child(div().text_size(px(18.)).font_weight(FontWeight::SEMIBOLD).child(title))
}

fn row(i: Icon, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Div {
    div()
        .min_h(px(64.))
        .px(px(16.))
        .py(px(10.))
        .rounded(px(12.))
        .bg(theme::surface())
        .flex()
        .items_center()
        .gap(px(14.))
        .child(icon(i, 24., theme::muted()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
                .child(div().text_size(px(12.)).text_color(theme::muted()).child(detail.into())),
        )
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
