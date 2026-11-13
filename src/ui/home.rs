use crate::app::{Hub, Plaivo, Shape, ShelfKey, Slide};
use crate::clock;
use crate::ui::shell::{ease_out, progress};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, overlay_button, pill, scrollbar, skeleton};
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, MouseButton, MouseDownEvent, MouseUpEvent,
    ObjectFit, ScrollEvent, ScrollHandle, SharedString, Stateful, Window, div, img,
    linear_color_stop, linear_gradient, point, prelude::*, px, rgb,
};
use std::ops::Range;
use std::time::{Duration, Instant};

const SWIPE: f32 = 60.;
const SLIDE_ANIM: Duration = Duration::from_millis(320);

pub fn content_width(this: &Plaivo, window: &Window) -> f32 {
    let sidebar = if matches!(
        this.page,
        crate::app::Page::Detail | crate::app::Page::HubList
    ) {
        0.
    } else if this.settings.sidebar_compact {
        theme::SIDEBAR_COMPACT
    } else {
        theme::SIDEBAR
    };
    f32::from(window.viewport_size().width) - sidebar
}

pub fn arrow_y(w: f32, wide: bool, pad: f32) -> f32 {
    (w - 2. * pad) * if wide { 9. / 32. } else { 0.75 } - 24. + pad
}

fn set_hover(slot: &mut Option<ShelfKey>, key: ShelfKey, on: bool) {
    if on {
        *slot = Some(key);
    } else if *slot == Some(key) {
        *slot = None;
    }
}

pub fn card_height(w: f32, wide: bool, pad: f32) -> f32 {
    let art = w - 2. * pad;
    (if wide { art * 9. / 16. } else { art * 1.5 }) + 40. + 2. * pad
}

pub fn card_width(content_w: f32, wide: bool, scale: f32) -> f32 {
    let cw = content_w - 16.;
    let extent = (if content_w >= 1100. { 210. } else { 160. }) * scale;
    let extent = if wide { extent * 1.8 } else { extent };
    cw / plaivo_core::grid_columns(cw, extent, 0.) as f32
}

impl Plaivo {
    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let offline = self
            .home_server()
            .filter(|s| crate::plex::is_down(&s.id))
            .map(|s| SharedString::from(format!("Can't reach {} right now.", s.name)));
        if let Some(e) = self.home_error.clone().or(offline) {
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(16.))
                .child(div().text_color(theme::muted()).child(e))
                .child(
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(
                            pill("retry", "Retry", Some(Icon::Refresh), true)
                                .on_click(cx.listener(|this, _, _, cx| this.refresh_home(cx))),
                        )
                        .child(
                            pill("change", "Change server", None, false).on_click(cx.listener(
                                |this, _, _, cx| this.navigate(crate::app::Nav::Servers, true, cx),
                            )),
                        )
                        .when(self.has_downloads(), |d| {
                            d.child(
                                pill("offline", "Downloads", Some(Icon::Download), false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.navigate(crate::app::Nav::Downloads(None), true, cx)
                                    }),
                                ),
                            )
                        }),
                )
                .into_any_element();
        }
        let cw = content_width(self, window);
        let vh = f32::from(window.viewport_size().height);
        let hero = (!self.hero.is_empty()).then(|| self.render_hero(cw, vh * 0.75, window, cx));
        let hubs: Vec<AnyElement> = if self.hubs.is_empty() {
            (0..3).map(|i| self.render_hub_skeleton(i, cw)).collect()
        } else {
            let top = if self.hero.is_empty() {
                0.
            } else {
                vh * 0.75 + 16.
            };
            self.render_hubs(false, cw, top, window, cx)
        };
        let page = div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.home_scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .pb(px(24.))
                    .children(hero)
                    .children(hubs),
            );
        let fullscreen = window.is_fullscreen();
        if fullscreen {
            self.tick_clock(cx);
        }
        div()
            .size_full()
            .relative()
            .child(page)
            .child(
                div()
                    .absolute()
                    .top(px(12.))
                    .right(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(fullscreen, |d| {
                        d.child(
                            div()
                                .h(px(40.))
                                .px(px(14.))
                                .flex()
                                .items_center()
                                .rounded_full()
                                .bg(theme::black(0.3))
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(0xFFFFFF))
                                .child(clock::local_time_in(0.)),
                        )
                    })
                    .child(
                        overlay_button(
                            "refresh",
                            Icon::Refresh,
                            40.,
                            20.,
                            theme::black(0.3),
                            theme::black(0.5),
                            rgb(0xFFFFFF),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_home(cx))),
                    ),
            )
            .child(scrollbar(&self.home_scroll))
            .into_any_element()
    }

    fn render_hero(
        &mut self,
        width: f32,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.view.hero_h = height;
        let item = self.hero[self.hero_index.min(self.hero.len() - 1)].clone();
        let current = self.image(&item.thumb, Shape::Backdrop);
        let arrived = self.image_alpha();
        let upcoming = self.hero[(self.hero_index + 1) % self.hero.len()]
            .thumb
            .clone();
        self.prefetch(&upcoming, Shape::Backdrop);
        let fade = self.view.hero_prev.map(|(i, t0)| {
            (
                i,
                ease_out(progress(Some(t0), Duration::from_millis(500), window)),
            )
        });
        let previous = fade
            .filter(|(_, p)| *p < 1.)
            .and_then(|(i, _)| self.hero.get(i).cloned())
            .and_then(|c| self.image(&c.thumb, Shape::Backdrop));
        let alpha = fade.map_or(1., |(_, p)| p);
        let bg = theme::bg();
        let dots = (self.hero.len() > 1).then(|| {
            div()
                .absolute()
                .bottom(px(24.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .gap(px(8.))
                .children((0..self.hero.len()).map(|i| {
                    let active = i == self.hero_index;
                    div()
                        .id(("dot", i))
                        .w(px(if active { 24. } else { 8. }))
                        .h(px(8.))
                        .rounded_full()
                        .cursor_pointer()
                        .bg(if active {
                            theme::text().into()
                        } else {
                            theme::tint(0.4)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let d = i as isize - this.hero_index as isize;
                            this.step_hero(d, cx);
                        }))
                }))
        });
        let play_key = (item.server.to_string(), item.key.to_string());
        let open = item.clone();
        div()
            .id("hero")
            .relative()
            .w(px(width))
            .h(px(height))
            .flex_none()
            .overflow_hidden()
            .bg(theme::bg())
            .when_some(previous, |d, im| {
                d.child(img(im).absolute().size_full().object_fit(ObjectFit::Cover))
            })
            .when_some(current, |d, im| {
                d.child(
                    img(im)
                        .absolute()
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .opacity(alpha * arrived),
                )
            })
            .child(div().absolute().size_full().bg(linear_gradient(
                180.,
                linear_color_stop(bg.opacity(0.), 0.5),
                linear_color_stop(bg.opacity(1.), 0.94),
            )))
            .child(div().absolute().size_full().bg(linear_gradient(
                90.,
                linear_color_stop(bg.opacity(0.65), 0.),
                linear_color_stop(bg.opacity(0.), 0.6),
            )))
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| this.hero_wheel(ev, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, _| {
                    this.view.hero_drag = Some(f32::from(ev.position.x))
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                    this.hero_release(f32::from(ev.position.x), cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseUpEvent, _, cx| {
                    this.hero_release(f32::from(ev.position.x), cx)
                }),
            )
            .child(
                div()
                    .absolute()
                    .left(px(40.))
                    .bottom(px(64.))
                    .w(px((width - 240.).clamp(320., 760.)))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(40.))
                            .font_weight(FontWeight::BOLD)
                            .line_height(px(46.))
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(item.meta.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(14.))
                            .line_height(px(20.))
                            .text_color(theme::tint(0.7))
                            .line_clamp(2)
                            .child(item.summary.clone()),
                    )
                    .child(
                        div()
                            .pt(px(8.))
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .id("hero-play")
                                    .h(px(44.))
                                    .px(px(24.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .bg(rgb(0xFFFFFF))
                                    .hover(|s| s.bg(rgb(0xE0E0E0)))
                                    .cursor_pointer()
                                    .child(icon(Icon::Play, 20., rgb(0x000000)))
                                    .when(item.progress > 0., |d| {
                                        d.child(
                                            div()
                                                .w(px(40.))
                                                .h(px(6.))
                                                .rounded(px(3.))
                                                .bg(theme::black(0.25))
                                                .child(
                                                    div()
                                                        .h_full()
                                                        .rounded(px(3.))
                                                        .w(gpui::relative(item.progress))
                                                        .bg(rgb(0x000000)),
                                                ),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_size(px(14.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(0x000000))
                                            .child(if item.progress > 0. {
                                                "Resume"
                                            } else {
                                                "Play"
                                            }),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.play(play_key.0.clone(), play_key.1.clone(), cx)
                                    })),
                            )
                            .child(
                                pill("hero-details", "Details", None, false).on_click(
                                    cx.listener(move |this, _, _, cx| this.open(&open, cx)),
                                ),
                            ),
                    ),
            )
            .children(dots)
            .into_any_element()
    }

    fn hero_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(20.));
        let (dx, dy) = (f32::from(d.x), f32::from(d.y));
        if !ev.modifiers.shift && dy.abs() >= dx.abs() {
            return;
        }
        cx.stop_propagation();
        if self
            .view
            .hero_cooldown
            .is_some_and(|t| t.elapsed() < Duration::from_millis(450))
        {
            return;
        }
        self.view.hero_acc += if ev.modifiers.shift { dx + dy } else { dx };
        if self.view.hero_acc.abs() >= SWIPE {
            let step = if self.view.hero_acc < 0. { 1 } else { -1 };
            self.view.hero_acc = 0.;
            self.view.hero_cooldown = Some(Instant::now());
            self.step_hero(step, cx);
        }
    }

    fn hero_release(&mut self, x: f32, cx: &mut Context<Self>) {
        if let Some(start) = self.view.hero_drag.take() {
            let d = x - start;
            if d.abs() >= SWIPE {
                self.step_hero(if d < 0. { 1 } else { -1 }, cx);
            }
        }
    }

    fn hub_vec(&mut self, lib: bool) -> &mut Vec<Hub> {
        match (lib, &mut self.lib_hubs) {
            (true, Some(h)) => h,
            _ => &mut self.hubs,
        }
    }

    fn shelf_state(&mut self, key: ShelfKey) -> Option<(ScrollHandle, &mut Slide)> {
        match key {
            ShelfKey::Hub(lib, i) => self
                .hub_vec(lib)
                .get_mut(i)
                .map(|h| (h.scroll.clone(), &mut h.slide)),
            ShelfKey::Detail(i) => {
                if self.detail_shelves.len() <= i {
                    self.detail_shelves.resize_with(i + 1, Default::default);
                }
                let (scroll, slide) = &mut self.detail_shelves[i];
                Some((scroll.clone(), slide))
            }
        }
    }

    pub fn shelf_offset(&mut self, key: ShelfKey, window: &mut Window) -> f32 {
        let Some((scroll, slide)) = self.shelf_state(key) else {
            return 0.;
        };
        if let Some((from, to, t0)) = *slide {
            let p = ease_out(progress(Some(t0), SLIDE_ANIM, window));
            scroll.set_offset(point(px(from + (to - from) * p), px(0.)));
            if p >= 1. {
                *slide = None;
            }
        }
        f32::from(scroll.offset().x)
    }

    pub fn shelf(
        &mut self,
        key: ShelfKey,
        id: impl Into<ElementId>,
        arrow_y: f32,
        row: Div,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let Some((scroll, slide)) = self.shelf_state(key) else {
            return div().id(id);
        };
        let target = slide.map(|(_, to, _)| to);
        let offset = f32::from(scroll.offset().x);
        let base = target.unwrap_or(offset);
        let max = f32::from(scroll.max_offset().x);
        let page = f32::from(scroll.bounds().size.width) * 0.8;
        let hovered =
            self.view.shelf_hover == Some(key) || self.view.shelf_arrow_hover == Some(key);
        let arrows: Vec<AnyElement> = [(true, offset < -1.), (false, -offset < max - 1.)]
            .into_iter()
            .filter(|&(_, show)| hovered && show)
            .map(|(left, _)| {
                let to = if left {
                    (base + page).min(0.)
                } else {
                    (base - page).max(-max)
                };
                div()
                    .absolute()
                    .top(px(arrow_y))
                    .map(|d| {
                        if left {
                            d.left(px(8.))
                        } else {
                            d.right(px(8.))
                        }
                    })
                    .child(
                        overlay_button(
                            if left { "shelf-left" } else { "shelf-right" },
                            if left {
                                Icon::ChevronLeft
                            } else {
                                Icon::ChevronRight
                            },
                            48.,
                            32.,
                            theme::black(0.7),
                            theme::black(0.9),
                            rgb(0xFFFFFF),
                        )
                        // The arrow occludes the row's hitbox, so it tracks its own hover.
                        .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                            set_hover(&mut this.view.shelf_arrow_hover, key, *on);
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some((_, slide)) = this.shelf_state(key) {
                                *slide = Some((offset, to, Instant::now()));
                                cx.notify();
                            }
                        })),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id(id)
            .relative()
            .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                if *on {
                    set_hover(&mut this.view.shelf_arrow_hover, key, false);
                }
                set_hover(&mut this.view.shelf_hover, key, *on);
                cx.notify();
            }))
            .child(
                row.id("row")
                    .overflow_x_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&scroll),
            )
            .children(arrows)
    }

    pub fn render_hubs(
        &mut self,
        lib: bool,
        cw: f32,
        mut top: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let vh = f32::from(window.viewport_size().height);
        let scrolled = -f32::from(
            if lib {
                &self.lib_hubs_scroll
            } else {
                &self.home_scroll
            }
            .offset()
            .y,
        );
        let mut prefetch = Vec::new();
        let hubs = (0..self.hub_vec(lib).len())
            .map(|i| {
                let wide = self.hub_vec(lib)[i].wide;
                let w = card_width(cw, wide, self.settings.card_scale());
                let height = 38. + card_height(w, wide, self.settings.card_pad());
                let band = |above: f32, below: f32| {
                    top + height > scrolled - above * vh && top < scrolled + below * vh
                };
                let bands = (band(1., 2.), band(0.25, 1.25));
                top += height + 16.;
                let (hub, rest) = self.render_hub(lib, i, cw, bands, window, cx);
                prefetch.push((i, rest));
                hub
            })
            .collect();
        // After every visible card, so on-screen art is queued first.
        for (i, rest) in prefetch {
            let shape = if self.hub_vec(lib)[i].wide {
                Shape::Wide
            } else {
                Shape::Poster
            };
            let items = std::mem::take(&mut self.hub_vec(lib)[i].items);
            for c in rest.into_iter().flat_map(|r| &items[r]) {
                self.image(&c.thumb, shape);
            }
            self.hub_vec(lib)[i].items = items;
        }
        hubs
    }

    pub fn render_hub_skeleton(&self, i: usize, cw: f32) -> AnyElement {
        let wide = i == 0;
        let w = card_width(cw, wide, self.settings.card_scale());
        let pad = self.settings.card_pad();
        let art_h = if wide {
            (w - 2. * pad) * 9. / 16.
        } else {
            (w - 2. * pad) * 1.5
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .pt(if i == 0 { px(16.) } else { px(0.) })
            .child(div().pl(px(4.)).child(skeleton(px(180.), 20.)))
            .child(div().flex().overflow_hidden().children((0..8).map(|_| {
                div().w(px(w)).flex_none().p(px(pad)).child(
                    div()
                        .w(px(w - 2. * pad))
                        .h(px(art_h))
                        .rounded(px(8.))
                        .bg(theme::surface()),
                )
            })))
            .into_any_element()
    }

    fn render_hub(
        &mut self,
        lib: bool,
        i: usize,
        cw: f32,
        (near, visible): (bool, bool),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (AnyElement, [Range<usize>; 2]) {
        let (title, resume, wide, key, server) = {
            let h = &self.hub_vec(lib)[i];
            (
                h.title.clone(),
                h.resume,
                h.wide,
                h.key.clone(),
                h.server.clone(),
            )
        };
        let w = card_width(cw, wide, self.settings.card_scale());
        let pad = self.settings.card_pad();
        let shelf = ShelfKey::Hub(lib, i);
        let offset = self.shelf_offset(shelf, window);
        let items = std::mem::take(&mut self.hub_vec(lib)[i].items);
        let cards_in = |show: bool, before: f32, after: f32| {
            let last = if show {
                (((-offset + after) / w).ceil().max(0.) as usize).min(items.len())
            } else {
                0
            };
            (((-offset - before) / w).floor().max(0.) as usize).min(last)..last
        };
        let shown = cards_in(visible, w, cw + w);
        let near = cards_in(near, cw, 2. * cw);
        let spacer = |n: usize| {
            div()
                .w(px(n as f32 * w))
                .h(px(card_height(w, wide, pad)))
                .flex_none()
                .into_any_element()
        };
        let mut cards = vec![spacer(shown.start)];
        for j in shown.clone() {
            cards.push(self.media_card(("card", j), &items[j], w, wide, cx));
        }
        cards.push(spacer(items.len() - shown.end));
        self.hub_vec(lib)[i].items = items;
        let rest = [
            near.start..shown.start.clamp(near.start, near.end),
            shown.end.clamp(near.start, near.end)..near.end,
        ];
        let row = self.shelf(
            shelf,
            "shelf",
            arrow_y(w, wide, pad),
            div().px(px(8.)).flex().children(cards),
            cx,
        );
        let hub = div()
            .id(("hub", i))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div().h(px(34.)).pl(px(12.)).flex().items_center().child(
                    div()
                        .id(("hub-title", i))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon(
                            if resume {
                                Icon::PlayCircle
                            } else {
                                Icon::Sparkle
                            },
                            24.,
                            theme::text(),
                        ))
                        .child(div().text_size(px(22.)).child(title.clone()))
                        .when(!key.is_empty(), |d| {
                            d.cursor_pointer()
                                .child(icon(Icon::ChevronRight, 22., theme::muted()))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.navigate(
                                        crate::app::Nav::Hub(
                                            server.to_string(),
                                            key.to_string(),
                                            title.to_string(),
                                            resume,
                                        ),
                                        true,
                                        cx,
                                    )
                                }))
                        }),
                ),
            )
            .child(row)
            .into_any_element();
        (hub, rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_fill_the_row_exactly() {
        for cw in [800., 1300., 1700.] {
            for wide in [false, true] {
                let w = card_width(cw, wide, if wide { 1. } else { 1.3 });
                let n = ((cw - 16.) / w).round();
                assert!(((cw - 16.) - n * w).abs() < 0.01, "{cw} {wide}");
            }
        }
    }
}
