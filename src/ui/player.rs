use crate::app::{Card, Choice, Nav, Plaivo, Shape, fmt_resolution};
use crate::clock;
use crate::player::{Panel, SPEEDS};
use crate::plex::{self, QUALITIES};
use crate::settings::Scale;
use crate::shaders;
use crate::ui::settings::quality_label;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{
    bounds_probe, fmt_time, icon, pill, progress_bar, round_button, shadow, stepper, tip,
};
use gpui::{
    AnyElement, Context, ElementId, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ObjectFit, PathBuilder, SharedString, Window, canvas, div, linear_color_stop,
    linear_gradient, point, prelude::*, px, rgb, uniform_list,
};
use std::f32::consts::TAU;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const QUEUE_ROW: f32 = 75.;
const SEEK_PREVIEW: Duration = Duration::from_millis(500);
const SPIN_PERIOD_MS: u128 = 900;

fn control(
    id: impl Into<ElementId>,
    i: Icon,
    size: f32,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    round_button(
        id,
        i,
        40.,
        size,
        theme::black(0.),
        theme::white().opacity(0.12),
        theme::white(),
    )
    .tooltip(tip(label))
}

fn seek_button(id: &'static str, i: Icon, secs: u32, back: bool) -> gpui::Stateful<gpui::Div> {
    let label = format!("{} {secs} seconds", if back { "Back" } else { "Forward" });
    control(id, i, 24., label).relative().child(
        div()
            .absolute()
            .top(px(16.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .text_size(px(7.))
            .font_weight(FontWeight::BOLD)
            .child(secs.to_string()),
    )
}

fn fmt_speed(s: f64) -> String {
    if s == 1.0 {
        "Normal".into()
    } else {
        format!("{s}×")
    }
}

pub fn spinner(size: f32, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            const SEGMENTS: usize = 40;
            let turn = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0., |d| {
                    (d.as_millis() % SPIN_PERIOD_MS) as f32 / SPIN_PERIOD_MS as f32
                });
            let start = turn * TAU;
            let center = bounds.center();
            let r = size / 2. - 3.;
            let points: Vec<_> = (0..=SEGMENTS)
                .map(|i| {
                    let a = start + i as f32 / SEGMENTS as f32 * TAU * 0.75;
                    point(center.x + px(r * a.cos()), center.y + px(r * a.sin()))
                })
                .collect();
            let mut path = PathBuilder::stroke(px(4.));
            path.add_polygon(&points, false);
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
            window.request_animation_frame();
        },
    )
    .size(px(size))
}

fn scale_icon(s: Scale) -> Icon {
    match s {
        Scale::Fit => Icon::ScaleFit,
        Scale::Fill => Icon::ScaleFill,
        Scale::Stretch => Icon::ScaleStretch,
    }
}

fn panel_title(p: Panel) -> &'static str {
    match p {
        Panel::Tracks => "Audio & Subtitles",
        Panel::SubtitleSearch => "Search subtitles",
        Panel::Settings => "Playback Settings",
        Panel::Quality => "Quality",
        Panel::Speed => "Playback Speed",
        Panel::Output => "Audio Output",
        Panel::Shader => "Shader",
        Panel::Info => "File Info",
        Panel::Queue => "Queue",
    }
}

impl Plaivo {
    fn frac_in(&self, x: f32, bounds: gpui::Bounds<gpui::Pixels>) -> f32 {
        let w = f32::from(bounds.size.width).max(1.);
        ((x - f32::from(bounds.origin.x)) / w).clamp(0., 1.)
    }

    fn link(
        &self,
        id: impl Into<ElementId>,
        text: SharedString,
        nav: Option<Nav>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let base = div().min_w_0().truncate().child(text);
        match nav {
            Some(nav) => base
                .id(id)
                .cursor_pointer()
                .hover(|s| s.underline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.open_from_player(nav.clone(), cx);
                }))
                .into_any_element(),
            None => base.into_any_element(),
        }
    }

    fn crumbs(&self, prefix: &'static str, cx: &mut Context<Self>) -> gpui::Div {
        let mut row = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .overflow_hidden();
        for (i, (text, nav)) in self.player.crumbs.iter().enumerate() {
            if i > 0 {
                row = row.child(div().flex_none().child("·"));
            }
            row = row.child(self.link(
                SharedString::from(format!("{prefix}-crumb-{i}")),
                text.clone(),
                nav.clone(),
                cx,
            ));
        }
        row
    }

    fn end_seek_drag(&mut self, x: f32, cx: &mut Context<Self>) {
        self.view.seek_preview = None;
        if self.view.seek_drag.take().is_some() {
            let t = self.frac_in(x, self.view.timeline) as f64 * self.player.duration;
            self.seek_to(t);
            self.player.time = t;
            cx.notify();
        }
    }

    fn end_volume_drag(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.view.volume_drag) {
            self.hint_volume(cx);
        }
    }

    fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        let group = |p: Panel| match p {
            Panel::Tracks | Panel::SubtitleSearch => Panel::Tracks,
            Panel::Queue => p,
            _ => Panel::Settings,
        };
        let open = self.player.panel.is_some_and(|p| group(p) == group(panel));
        if open {
            self.player.panel = None;
            cx.notify();
        } else {
            self.open_panel(panel, cx);
        }
    }

    pub fn render_player(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let hide = !self.controls_shown();
        if hide != self.view.cursor_hidden {
            self.view.cursor_hidden = hide;
            if let Some(host) = &self.player.host {
                host.set_cursor_hidden(hide);
            }
        }
        if !hide {
            self.tick_clock(cx);
        }
        let p = &self.player;
        let (pos, dur) = (p.time, p.duration);
        let frac = self.view.seek_drag.unwrap_or(if dur > 0. {
            (pos / dur).clamp(0., 1.) as f32
        } else {
            0.
        });
        let pos = if self.view.seek_drag.is_some() {
            frac as f64 * dur
        } else {
            pos
        };
        let cached = if dur > 0. {
            (p.cached / dur).clamp(0., 1.) as f32
        } else {
            0.
        };
        let hover = self.view.timeline_hover;
        let fullscreen = window.is_fullscreen();
        let entity = cx.entity();
        let wake = cx.listener(|this, _: &MouseMoveEvent, _, cx| this.wake_controls(cx));
        let background = div()
            .id("video")
            .absolute()
            .size_full()
            .on_click(cx.listener(|this, ev: &gpui::ClickEvent, window, cx| {
                if ev.click_count() >= 2 {
                    window.toggle_fullscreen();
                } else if this.player.panel.is_some() {
                    this.player.panel = None;
                } else {
                    this.toggle_pause();
                }
                cx.notify();
            }));
        let spinner = self.player.buffering.then(|| {
            div()
                .absolute()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(spinner(44., theme::white()))
        });
        let up_next = self.up_next();
        let skip = self.active_skip().filter(|_| up_next.is_none()).map(|s| {
            div()
                .id("skip")
                .occlude()
                .absolute()
                .right(px(24.))
                .bottom(px(if hide { 32. } else { 116. }))
                .h(px(44.))
                .px(px(18.))
                .rounded(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(theme::white().opacity(0.92))
                .hover(|s| s.bg(theme::white()))
                .shadow(shadow(12., 0.4))
                .cursor_pointer()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::bg())
                .child(s.label)
                .child(icon(Icon::FastForward, 20., theme::bg()))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.skip_marker();
                    cx.notify();
                }))
        });
        let timeline = {
            let e1 = entity.clone();
            let bar = if hover.is_some() { 6. } else { 4. };
            div()
                .id("timeline")
                .relative()
                .flex_1()
                .h(px(20.))
                .cursor_pointer()
                .child(bounds_probe(move |b, _, cx| {
                    e1.update(cx, |this, _| this.view.timeline = b)
                }))
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                    let f = this.frac_in(f32::from(ev.position.x), this.view.timeline);
                    this.view.timeline_hover = Some(f);
                    if this.view.seek_drag.is_some() && ev.dragging() {
                        this.view.seek_drag = Some(f);
                        if this
                            .view
                            .seek_preview
                            .is_none_or(|t| t.elapsed() >= SEEK_PREVIEW)
                        {
                            this.view.seek_preview = Some(Instant::now());
                            this.preview_seek(f as f64 * this.player.duration);
                        }
                    }
                    cx.notify();
                }))
                .on_hover(cx.listener(|this, h: &bool, _, cx| {
                    if !h {
                        this.view.timeline_hover = None;
                        cx.notify();
                    }
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.view.seek_drag =
                            Some(this.frac_in(f32::from(ev.position.x), this.view.timeline));
                        cx.stop_propagation();
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                        this.end_seek_drag(f32::from(ev.position.x), cx)
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                        this.end_seek_drag(f32::from(ev.position.x), cx)
                    }),
                )
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px((20. - bar) / 2.))
                        .h(px(bar))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(theme::white().opacity(0.2))
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .h_full()
                                .w(gpui::relative(cached.max(frac)))
                                .bg(theme::white().opacity(0.35)),
                        )
                        .child(
                            div()
                                .absolute()
                                .left_0()
                                .h_full()
                                .w(gpui::relative(frac))
                                .bg(theme::white()),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(3.))
                        .left(gpui::relative(frac))
                        .ml(px(-1.5))
                        .w(px(3.))
                        .h(px(14.))
                        .rounded(px(1.5))
                        .bg(theme::white()),
                )
                .when_some(hover.filter(|_| dur > 0.), |d, h| {
                    d.child(
                        div()
                            .absolute()
                            .top(px(-30.))
                            .left(gpui::relative(h))
                            .ml(px(-28.))
                            .w(px(56.))
                            .h(px(24.))
                            .rounded(px(4.))
                            .bg(theme::black(0.7))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(12.))
                            .text_color(theme::white())
                            .child(fmt_time(h as f64 * dur)),
                    )
                })
        };
        let volume = {
            let e2 = entity.clone();
            let level = if self.player.muted {
                0.
            } else {
                (self.player.volume / 100.) as f32
            };
            div()
                .id("volume")
                .relative()
                .w(px(100.))
                .h(px(20.))
                .cursor_pointer()
                .child(bounds_probe(move |b, _, cx| {
                    e2.update(cx, |this, _| this.view.volume = b)
                }))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top(px(8.))
                        .h(px(4.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(theme::white().opacity(0.2))
                        .child(div().h_full().w(gpui::relative(level)).bg(theme::white())),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(3.))
                        .left(gpui::relative(level))
                        .ml(px(-1.5))
                        .w(px(3.))
                        .h(px(14.))
                        .rounded(px(1.5))
                        .bg(theme::white()),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                        this.view.volume_drag = true;
                        let f = this.frac_in(f32::from(ev.position.x), this.view.volume);
                        this.set_volume(f as f64 * 100.);
                        cx.stop_propagation();
                    }),
                )
                .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, _| {
                    if this.view.volume_drag && ev.dragging() {
                        let f = this.frac_in(f32::from(ev.position.x), this.view.volume);
                        this.set_volume(f as f64 * 100.);
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.end_volume_drag(cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.end_volume_drag(cx)),
                )
        };
        let up_next = up_next.map(|(card, left)| self.render_up_next(card, left, hide, cx));
        let small = self.settings.seek_small;
        let stop_on_close = self.settings.stop_on_close;
        let paused = self.player.paused;
        let episodic = !self.player.queue.is_empty();
        let (prev, next) = (self.neighbor(-1), self.neighbor(1));
        let skip_button = |id: &'static str,
                           i: Icon,
                           step: isize,
                           target: Option<crate::app::Card>,
                           cx: &mut Context<Self>| {
            let label: SharedString = target.as_ref().map_or_else(
                || {
                    if step < 0 {
                        "No previous episode"
                    } else {
                        "No next episode"
                    }
                    .into()
                },
                |c| {
                    format!(
                        "{} · {}",
                        if step < 0 { "Previous" } else { "Next" },
                        c.subtitle
                    )
                    .into()
                },
            );
            let b = control(id, i, 26., label);
            match target {
                Some(_) => {
                    b.on_click(cx.listener(move |this, _, _, cx| this.play_neighbor(step, cx)))
                }
                None => b.opacity(0.35).cursor_default(),
            }
        };
        let muted = self.player.muted || self.player.volume == 0.;
        let scale = self.settings.scale;
        let title_max = (f32::from(window.viewport_size().width) / 2. - 180.).max(160.);
        let ends = (dur > 0.).then(|| {
            format!(
                "Ends at {}",
                clock::local_time_in((dur - pos).max(0.) / self.player.speed.max(0.1))
            )
        });
        let controls = div()
            .absolute()
            .size_full()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(160.))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(theme::black(0.7), 0.),
                        linear_color_stop(theme::black(0.), 1.),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .h(px(200.))
                    .bg(linear_gradient(
                        0.,
                        linear_color_stop(theme::black(0.75), 0.),
                        linear_color_stop(theme::black(0.), 1.),
                    )),
            )
            .child(
                div()
                    .id("top-bar")
                    .occlude()
                    .on_mouse_move(
                        cx.listener(|this, _: &MouseMoveEvent, _, cx| this.wake_controls(cx)),
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(72.))
                    .pl(px(8.))
                    .pr(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(if stop_on_close {
                        control("stop", Icon::Close, 26., "Stop")
                            .on_click(cx.listener(|this, _, _, cx| this.stop_playback(cx)))
                    } else {
                        control("minimize", Icon::ExpandMore, 28., "Minimize")
                            .on_click(cx.listener(|this, _, _, cx| this.minimize_player(cx)))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w(px(title_max))
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme::white())
                                    .child(self.link(
                                        "title",
                                        self.player.title.clone(),
                                        self.player.title_link.clone(),
                                        cx,
                                    )),
                            )
                            .when(!self.player.crumbs.is_empty(), |d| {
                                d.child(
                                    self.crumbs("player", cx)
                                        .text_size(px(14.))
                                        .text_color(theme::white().opacity(0.7)),
                                )
                            }),
                    )
                    .child(div().flex_1())
                    .children(self.render_watch_pill(cx))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::white())
                            .child(clock::local_time_in(0.)),
                    ),
            )
            .child(
                div()
                    .id("bottom-bar")
                    .occlude()
                    .on_mouse_move(
                        cx.listener(|this, _: &MouseMoveEvent, _, cx| this.wake_controls(cx)),
                    )
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .px(px(20.))
                    .pb(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .px(px(4.))
                            .child(
                                div()
                                    .min_w(px(44.))
                                    .text_size(px(13.))
                                    .text_color(theme::white())
                                    .child(fmt_time(pos)),
                            )
                            .child(timeline)
                            .child(
                                div()
                                    .min_w(px(52.))
                                    .flex()
                                    .justify_end()
                                    .text_size(px(13.))
                                    .text_color(theme::white())
                                    .child(format!("-{}", fmt_time(dur - pos))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .when(episodic, |d| {
                                d.child(skip_button("prev", Icon::SkipPrevious, -1, prev, cx))
                            })
                            .child(seek_button("back", Icon::Replay, small, true).on_click(
                                cx.listener(move |this, _, _, _| this.seek_by(-(small as f64))),
                            ))
                            .child(
                                control(
                                    "pause",
                                    if paused { Icon::Play } else { Icon::Pause },
                                    30.,
                                    if paused { "Play" } else { "Pause" },
                                )
                                .on_click(cx.listener(|this, _, _, _| this.toggle_pause())),
                            )
                            .child(seek_button("fwd", Icon::Forward, small, false).on_click(
                                cx.listener(move |this, _, _, _| this.seek_by(small as f64)),
                            ))
                            .when(episodic, |d| {
                                d.child(skip_button("next", Icon::SkipNext, 1, next, cx))
                            })
                            .child(if stop_on_close {
                                control("minimize", Icon::ExpandMore, 28., "Minimize").on_click(
                                    cx.listener(|this, _, _, cx| this.minimize_player(cx)),
                                )
                            } else {
                                control("stop", Icon::Stop, 24., "Stop")
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_playback(cx)))
                            })
                            .when_some(ends, |d, e| {
                                d.child(
                                    div()
                                        .ml(px(12.))
                                        .text_size(px(13.))
                                        .text_color(theme::white().opacity(0.7))
                                        .child(e),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                control(
                                    "mute",
                                    if muted { Icon::VolumeOff } else { Icon::Volume },
                                    24.,
                                    if muted { "Unmute" } else { "Mute" },
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_mute(cx))),
                            )
                            .child(volume)
                            .child(div().w(px(12.)))
                            .child(
                                control("settings", Icon::Tune, 24., "Playback settings").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.toggle_panel(Panel::Settings, cx)
                                    }),
                                ),
                            )
                            .child(
                                control("tracks", Icon::Subtitles, 24., "Audio & subtitles")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_panel(Panel::Tracks, cx)
                                    })),
                            )
                            .when(episodic, |d| {
                                d.child(control("queue", Icon::Queue, 24., "Queue").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.toggle_panel(Panel::Queue, cx)
                                    }),
                                ))
                            })
                            .child(
                                control(
                                    SharedString::from(format!("scale-{scale:?}")),
                                    scale_icon(scale),
                                    24.,
                                    scale.label(),
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.set_scale(scale.next());
                                        this.hint(
                                            scale_icon(scale.next()),
                                            scale.next().label(),
                                            cx,
                                        );
                                    },
                                )),
                            )
                            .child(
                                control(
                                    "fullscreen",
                                    if fullscreen {
                                        Icon::FullscreenExit
                                    } else {
                                        Icon::Fullscreen
                                    },
                                    24.,
                                    if fullscreen {
                                        "Exit fullscreen"
                                    } else {
                                        "Fullscreen"
                                    },
                                )
                                .on_click(|_, window, _| window.toggle_fullscreen()),
                            ),
                    ),
            );
        let panel = match self.player.panel {
            Some(Panel::Tracks) => Some(self.render_tracks(window, cx).into_any_element()),
            Some(Panel::SubtitleSearch) => {
                Some(self.render_subtitle_search(window, cx).into_any_element())
            }
            Some(Panel::Queue) => Some(self.render_queue(window, cx).into_any_element()),
            Some(p) => Some(self.render_settings(p, window, cx).into_any_element()),
            None => None,
        };
        let hint = self.player.hint.clone().map(|(i, text)| {
            div()
                .absolute()
                .top(px(18.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .h(px(36.))
                        .px(px(14.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .bg(theme::black(0.65))
                        .shadow(shadow(12., 0.3))
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::white())
                        .child(icon(i, 20., theme::white()))
                        .child(text),
                )
        });
        self.view.player_bare = hide
            && self.player.video_ready
            && !self.player.buffering
            && hint.is_none()
            && skip.is_none()
            && up_next.is_none()
            && panel.is_none();
        div()
            .id("player")
            .size_full()
            .relative()
            .on_mouse_move(wake)
            .child(background)
            .children(spinner)
            .when(!hide, |d| d.child(controls))
            .children(hint)
            .children(skip)
            .children(up_next)
            .children(panel)
            .into_any_element()
    }

    fn render_up_next(
        &mut self,
        card: crate::app::Card,
        left: f64,
        hide: bool,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let image = self.picture(&card.thumb, Shape::Wide);
        div()
            .id("up-next")
            .occlude()
            .absolute()
            .right(px(24.))
            .bottom(px(if hide { 32. } else { 116. }))
            .w(px(420.))
            .p(px(12.))
            .rounded(px(12.))
            .bg(theme::surface())
            .shadow(shadow(24., 0.5))
            .flex()
            .gap(px(12.))
            .child(
                div()
                    .w(px(128.))
                    .h(px(72.))
                    .flex_none()
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::bg())
                    .when_some(image, |d, im| {
                        d.child(im.size_full().object_fit(ObjectFit::Cover))
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::muted())
                                    .child(format!("Up next in {}s", left.max(0.).ceil())),
                            )
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(card.subtitle),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(
                                pill("up-next-play", "Play now", Some(Icon::Play), true)
                                    .h(px(36.))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.play_up_next(cx);
                                    })),
                            )
                            .child(
                                pill("up-next-cancel", "Cancel", None, false)
                                    .h(px(36.))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.cancel_up_next();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
    }

    pub(crate) fn panel_shell(
        &self,
        id: &'static str,
        panel: Panel,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let back = !matches!(panel, Panel::Settings | Panel::Tracks | Panel::Queue);
        let remaining = (panel == Panel::Queue).then(|| {
            self.queue_index()
                .map_or(self.player.queue.len(), |i| self.player.queue.len() - i - 1)
        });
        let header = div()
            .h(px(52.))
            .flex_none()
            .pl(px(if back { 6. } else { 16. }))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(theme::outline())
            .when(back, |d| {
                d.child(
                    round_button(
                        "panel-back",
                        Icon::ChevronLeft,
                        36.,
                        22.,
                        theme::black(0.),
                        theme::tint(0.08),
                        theme::text(),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_panel(
                            if panel == Panel::SubtitleSearch {
                                Panel::Tracks
                            } else {
                                Panel::Settings
                            },
                            cx,
                        )
                    })),
                )
            })
            .when(!back, |d| {
                d.child(icon(
                    match panel {
                        Panel::Tracks => Icon::Subtitles,
                        Panel::Queue => Icon::Queue,
                        _ => Icon::Tune,
                    },
                    20.,
                    theme::text(),
                ))
            })
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::BOLD)
                            .child(panel_title(panel)),
                    )
                    .when_some(remaining, |d, n| {
                        d.child(div().text_size(px(13.)).text_color(theme::muted()).child(
                            if n == 1 {
                                "1 remaining".to_owned()
                            } else {
                                format!("{n} remaining")
                            },
                        ))
                    }),
            )
            .child(
                round_button(
                    "panel-close",
                    Icon::Close,
                    40.,
                    20.,
                    theme::black(0.),
                    theme::tint(0.08),
                    theme::text(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.player.panel = None;
                    cx.notify();
                })),
            );
        div()
            .id(id)
            .occlude()
            .absolute()
            .right(px(24.))
            .bottom(px(84.))
            .rounded(px(16.))
            .overflow_hidden()
            .bg(theme::surface())
            .shadow(shadow(24., 0.5))
            .flex()
            .flex_col()
            .child(header)
    }

    fn setting_row(
        &self,
        i: Icon,
        label: &'static str,
        value: String,
        target: Panel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .id(label)
            .h(px(48.))
            .flex_none()
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(14.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::tint(0.06)))
            .child(icon(i, 20., theme::muted()))
            .child(div().flex_1().text_size(px(14.)).child(label))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme::muted())
                    .truncate()
                    .max_w(px(180.))
                    .child(value),
            )
            .child(icon(Icon::ChevronRight, 20., theme::muted()))
            .on_click(cx.listener(move |this, _, _, cx| this.open_panel(target, cx)))
    }

    fn choice_row(
        &self,
        panel: Panel,
        i: usize,
        c: &Choice,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let id = c.id.to_string();
        div()
            .id(SharedString::from(format!("{panel:?}-{i}")))
            .min_h(px(44.))
            .flex_none()
            .px(px(16.))
            .py(px(6.))
            .flex()
            .items_center()
            .gap(px(10.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::tint(0.06)))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(if c.selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .truncate()
                    .child(c.label.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .truncate()
                    .child(c.sub.clone()),
            )
            .when(c.selected, |d| {
                d.child(icon(Icon::Check, 20., theme::text()))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                match panel {
                    Panel::Quality => this.set_quality(id.parse().unwrap_or(0), cx),
                    Panel::Speed => this.set_speed(id.parse().unwrap_or(1.)),
                    Panel::Output => this.set_audio_device(&id),
                    Panel::Shader => this.set_shader(&id, cx),
                    _ => {}
                }
                this.open_panel(Panel::Settings, cx);
            }))
    }

    fn shader_label(&self) -> String {
        let name = shaders::name(&self.player.shader);
        if self.player.shader == shaders::NVSCALER && self.player.shader_applied.is_empty() {
            format!("{name} · off for HDR")
        } else {
            name.into()
        }
    }

    pub fn shader_choices(&self, current: &str) -> Vec<Choice> {
        std::iter::once(("", "Off"))
            .chain(shaders::PRESETS.iter().map(|p| (p.id, p.name)))
            .map(|(id, name)| Choice {
                id: id.into(),
                label: name.into(),
                sub: "".into(),
                selected: id == current,
            })
            .collect()
    }

    pub fn quality_choices(&self, original: SharedString) -> Vec<Choice> {
        let q = self.settings.quality;
        let original = Choice {
            id: "0".into(),
            label: "Original".into(),
            sub: original,
            selected: plex::Quality::find(q).is_none(),
        };
        std::iter::once(original)
            .chain(QUALITIES.iter().map(|x| Choice {
                id: x.kbps.to_string().into(),
                label: x.label.into(),
                sub: plex::fmt_bitrate(x.kbps as u64).into(),
                selected: x.kbps == q,
            }))
            .collect()
    }

    fn render_settings(
        &mut self,
        panel: Panel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let max_h = f32::from(window.viewport_size().height) - 120.;
        let body = div()
            .id("panel-body")
            .min_h_0()
            .overflow_y_scroll()
            .py(px(6.))
            .flex()
            .flex_col();
        let body = match panel {
            Panel::Settings => {
                let quality = match (
                    plex::Quality::find(self.settings.quality),
                    self.player.media.bitrate,
                ) {
                    (None, Some(b)) => format!("Original · {}", plex::fmt_bitrate(b)),
                    _ => quality_label(self.settings.quality),
                };
                let device = self.settings.audio_device.clone();
                let output = if device.is_empty() {
                    "Auto".into()
                } else {
                    self.player
                        .devices
                        .iter()
                        .find(|d| d.id.as_ref() == device)
                        .map_or(device, |d| d.label.to_string())
                };
                body.child(self.setting_row(
                    Icon::HighQuality,
                    "Quality",
                    quality,
                    Panel::Quality,
                    cx,
                ))
                .child(self.setting_row(
                    Icon::Speed,
                    "Playback Speed",
                    fmt_speed(self.player.speed),
                    Panel::Speed,
                    cx,
                ))
                .child(self.setting_row(Icon::Speaker, "Audio Output", output, Panel::Output, cx))
                .child(self.setting_row(
                    Icon::Sparkle,
                    "Shader",
                    self.shader_label(),
                    Panel::Shader,
                    cx,
                ))
                .child(self.setting_row(
                    Icon::Info,
                    "File Info",
                    String::new(),
                    Panel::Info,
                    cx,
                ))
            }
            Panel::Info => {
                body.px(px(16.))
                    .gap(px(10.))
                    .py(px(14.))
                    .children(self.player.info.iter().map(|(k, v)| {
                        div()
                            .flex()
                            .gap(px(12.))
                            .text_size(px(13.))
                            .child(
                                div()
                                    .w(px(84.))
                                    .flex_none()
                                    .text_color(theme::muted())
                                    .child(k.clone()),
                            )
                            .child(div().flex_1().min_w_0().child(v.clone()))
                    }))
            }
            _ => {
                let choices = match panel {
                    Panel::Quality => {
                        let m = &self.player.media;
                        self.quality_choices(
                            crate::app::join(&[
                                m.bitrate.map(plex::fmt_bitrate).unwrap_or_default(),
                                fmt_resolution(&m.video_resolution),
                            ])
                            .into(),
                        )
                    }
                    Panel::Speed => SPEEDS
                        .iter()
                        .map(|&s| Choice {
                            id: s.to_string().into(),
                            label: fmt_speed(s).into(),
                            sub: "".into(),
                            selected: s == self.player.speed,
                        })
                        .collect(),
                    Panel::Shader => self.shader_choices(&self.player.shader),
                    _ => self.player.devices.clone(),
                };
                body.children(
                    choices
                        .iter()
                        .enumerate()
                        .map(|(i, c)| self.choice_row(panel, i, c, cx)),
                )
            }
        };
        self.panel_shell("settings-panel", panel, cx)
            .w(px(if panel == Panel::Info { 440. } else { 380. }))
            .max_h(px(max_h))
            .child(body)
    }

    fn render_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let max_h = (f32::from(window.viewport_size().height) - 120.) * 0.75;
        let count = self.player.queue.len();
        let list = uniform_list(
            "queue-body",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let current = this.queue_index();
                range
                    .filter_map(|i| {
                        Some(this.queue_entry(
                            i,
                            current == Some(i),
                            this.player.queue.get(i)?.clone(),
                            cx,
                        ))
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.player.queue_scroll)
        .py(px(6.))
        .h(px((count as f32 * QUEUE_ROW + 12.).min(max_h - 52.)));
        self.panel_shell("queue-panel", Panel::Queue, cx)
            .w(px(440.))
            .child(list)
    }

    fn queue_entry(&mut self, i: usize, now: bool, c: Card, cx: &mut Context<Self>) -> AnyElement {
        let image = self.picture(&c.thumb, Shape::Wide);
        div()
            .id(("queue", i))
            .h(px(QUEUE_ROW))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .when(now, |d| d.bg(theme::tint(0.08)))
            .when(!now, |d| {
                d.cursor_pointer().hover(|s| s.bg(theme::tint(0.05)))
            })
            .child(
                div()
                    .relative()
                    .w(px(112.))
                    .h(px(63.))
                    .flex_none()
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::bg())
                    .when_some(image, |d, im| {
                        d.child(im.size_full().rounded(px(6.)).object_fit(ObjectFit::Cover))
                    })
                    .when(now, |d| {
                        d.child(
                            div()
                                .absolute()
                                .size_full()
                                .rounded(px(6.))
                                .bg(theme::black(0.5))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(Icon::Play, 24., theme::white())),
                        )
                    })
                    .when(c.watched && !now, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top(px(3.))
                                .right(px(3.))
                                .size(px(18.))
                                .rounded_full()
                                .bg(theme::text())
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon(Icon::Check, 12., theme::bg())),
                        )
                    })
                    .when(c.progress > 0. && !c.watched && !now, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(px(6.))
                                .left(px(6.))
                                .right(px(6.))
                                .child(progress_bar(c.progress, 3., theme::surface())),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(if now {
                                FontWeight::BOLD
                            } else {
                                FontWeight::SEMIBOLD
                            })
                            .truncate()
                            .child(c.subtitle.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .child(if now {
                                "Now playing".into()
                            } else {
                                c.meta.clone()
                            }),
                    ),
            )
            .when(!now, |d| {
                d.on_click(cx.listener(move |this, _, _, cx| this.play_card(c.clone(), cx)))
            })
            .into_any_element()
    }

    fn delay_stepper(&self, audio: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let (v, label, id) = if audio {
            (self.player.audio_delay, "Audio delay", "audio-delay")
        } else {
            (self.player.sub_delay, "Subtitle delay", "sub-delay")
        };
        div()
            .h(px(52.))
            .flex_none()
            .px(px(12.))
            .flex()
            .items_center()
            .border_t_1()
            .border_color(theme::outline())
            .child(
                div()
                    .flex_1()
                    .pl(px(4.))
                    .text_size(px(13.))
                    .text_color(theme::muted())
                    .child(label),
            )
            .child(stepper(
                id,
                format!("{:+} ms", v as i64),
                cx.listener(move |this, step: &i8, _, cx| {
                    this.nudge_delay(audio, *step as f64 * 50.);
                    cx.notify();
                }),
            ))
    }

    fn track_column(
        &self,
        title: &'static str,
        kind: &'static str,
        tracks: &[Choice],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let list = div()
            .id(title)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .h(px(32.))
                    .flex()
                    .items_center()
                    .px(px(4.))
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::muted())
                    .child(title),
            )
            .children(tracks.iter().enumerate().map(|(i, t)| {
                let id = t.id.to_string();
                div()
                    .id(SharedString::from(format!("{kind}-{i}")))
                    .min_h(px(36.))
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.06)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(if t.selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .truncate()
                                    .child(t.label.clone()),
                            )
                            .when(!t.sub.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(theme::muted())
                                        .truncate()
                                        .child(t.sub.clone()),
                                )
                            }),
                    )
                    .when(t.selected, |d| {
                        d.child(icon(Icon::Check, 20., theme::text()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_track(kind, &id);
                        cx.notify();
                    }))
            }))
            .when(kind == "sid" && self.can_search_subtitles(), |d| {
                d.child(
                    div()
                        .id("sub-search-open")
                        .min_h(px(36.))
                        .px(px(12.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::tint(0.06)))
                        .child(icon(Icon::Search, 18., theme::muted()))
                        .child(div().text_size(px(14.)).child("Search subtitles…"))
                        .on_click(cx.listener(|this, _, _, cx| this.open_subtitle_search(cx))),
                )
            });
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(list)
            .child(self.delay_stepper(kind == "aid", cx))
    }

    fn render_tracks(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vh = f32::from(window.viewport_size().height);
        let rows = self
            .player
            .audio_tracks
            .len()
            .max(self.player.sub_tracks.len()) as f32;
        let list_h = (32. + 16. + 52. + rows * 50.).min(vh - 220.).max(172.);
        let (audio, subs) = (
            self.player.audio_tracks.clone(),
            self.player.sub_tracks.clone(),
        );
        self.panel_shell("tracks", Panel::Tracks, cx)
            .w(px(640.))
            .h(px(53. + list_h))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.track_column("Audio", "aid", &audio, cx))
                    .child(div().w(px(1.)).h_full().bg(theme::outline()))
                    .child(self.track_column("Subtitles", "sid", &subs, cx)),
            )
    }

    pub fn render_mini(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let (x, bottom, w, h) = theme::MINI_HOLE;
        let bar = theme::MINI_BAR;
        let top = bar - bottom - h;
        let fill = theme::surface();
        let (pos, dur) = (self.player.time, self.player.duration);
        let frac = if dur > 0. { (pos / dur) as f32 } else { 0. };
        let hole = div()
            .id("mini-video")
            .w(px(w))
            .h(px(h))
            .flex_none()
            .cursor_pointer()
            .when(!self.player.video_ready, |d| d.bg(rgb(0x000000)))
            .on_click(cx.listener(|this, _, _, cx| this.expand_player(cx)));
        let mini_control = |id: &'static str, i: Icon, size: f32| {
            round_button(
                id,
                i,
                40.,
                size,
                theme::black(0.),
                theme::tint(0.1),
                theme::text(),
            )
        };
        let episodic = !self.player.queue.is_empty();
        let mini_skip = |id: &'static str,
                         i: Icon,
                         step: isize,
                         target: Option<crate::app::Card>,
                         cx: &mut Context<Self>| {
            let label: SharedString = target.as_ref().map_or_else(
                || {
                    if step < 0 {
                        "No previous episode"
                    } else {
                        "No next episode"
                    }
                    .into()
                },
                |c| {
                    format!(
                        "{} · {}",
                        if step < 0 { "Previous" } else { "Next" },
                        c.subtitle
                    )
                    .into()
                },
            );
            let b = mini_control(id, i, 24.).tooltip(tip(label));
            match target {
                Some(_) => {
                    b.on_click(cx.listener(move |this, _, _, cx| this.play_neighbor(step, cx)))
                }
                None => b.opacity(0.35).cursor_default(),
            }
        };
        let (prev, next) = (self.neighbor(-1), self.neighbor(1));
        div()
            .relative()
            .h(px(bar))
            .flex_none()
            .flex()
            .child(div().w(px(x)).h_full().bg(fill))
            .child(
                div()
                    .w(px(w))
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(div().h(px(top)).bg(fill))
                    .child(hole)
                    .child(div().flex_1().bg(fill)),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .bg(fill)
                    .flex()
                    .items_center()
                    .px(px(16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .min_w_0()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(self.link(
                                        "mini-title",
                                        self.player.title.clone(),
                                        self.player.title_link.clone(),
                                        cx,
                                    )),
                            )
                            .child(
                                self.crumbs("mini", cx)
                                    .text_size(px(12.))
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(theme::muted())
                                    .child(format!("{} / {}", fmt_time(pos), fmt_time(dur))),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .when(episodic, |d| {
                                d.child(mini_skip("mini-prev", Icon::SkipPrevious, -1, prev, cx))
                            })
                            .child(mini_control("mini-back", Icon::Replay, 24.).on_click(
                                cx.listener(|this, _, _, _| {
                                    this.seek_by(-(this.settings.seek_small as f64))
                                }),
                            ))
                            .child(
                                mini_control(
                                    "mini-pause",
                                    if self.player.paused {
                                        Icon::Play
                                    } else {
                                        Icon::Pause
                                    },
                                    30.,
                                )
                                .on_click(cx.listener(|this, _, _, _| this.toggle_pause())),
                            )
                            .child(mini_control("mini-fwd", Icon::Forward, 24.).on_click(
                                cx.listener(|this, _, _, _| {
                                    this.seek_by(this.settings.seek_small as f64)
                                }),
                            ))
                            .when(episodic, |d| {
                                d.child(mini_skip("mini-next", Icon::SkipNext, 1, next, cx))
                            })
                            .child(
                                mini_control("mini-stop", Icon::Stop, 24.)
                                    .on_click(cx.listener(|this, _, _, cx| this.stop_playback(cx))),
                            ),
                    )
                    .child(
                        div().flex_1().flex().justify_end().child(
                            mini_control("mini-expand", Icon::OpenFull, 22.)
                                .on_click(cx.listener(|this, _, _, cx| this.expand_player(cx))),
                        ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(3.))
                    .bg(theme::tint(0.15))
                    .child(div().h_full().w(gpui::relative(frac)).bg(theme::accent())),
            )
            .into_any_element()
    }
}
