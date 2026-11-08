use crate::app::{Plaivo, Shape};
use crate::ui::shell::{ease_out, progress};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, overlay_button, pill, scrollbar, skeleton};
use gpui::{
    AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, MouseUpEvent, ObjectFit, ScrollEvent, SharedString, Window, div, img, linear_color_stop, linear_gradient,
    point, prelude::*, px, rgb,
};
use std::time::{Duration, Instant};

const SWIPE: f32 = 60.;
const SLIDE_ANIM: Duration = Duration::from_millis(320);

pub fn content_width(this: &Plaivo, window: &Window) -> f32 {
    let sidebar = if this.page == crate::app::Page::Detail {
        0.
    } else if this.settings.sidebar_compact {
        theme::SIDEBAR_COMPACT
    } else {
        theme::SIDEBAR
    };
    f32::from(window.viewport_size().width) - sidebar
}

pub fn card_height(w: f32, wide: bool, pad: f32) -> f32 {
    let art = w - 2. * pad;
    (if wide { art * 9. / 16. } else { art * 1.5 }) + 40. + 2. * pad
}

pub fn card_width(content_w: f32, wide: bool, scale: f32) -> f32 {
    let cw = content_w - 16.;
    let extent = (if content_w >= 1100. { 210. } else { 160. }) * scale;
    let extent = if wide { extent * 1.8 } else { extent };
    cw / (cw / extent).ceil().max(1.)
}

impl Plaivo {
    pub fn render_home(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let offline = self.home_server().filter(|s| crate::plex::is_down(&s.id)).map(|s| SharedString::from(format!("Can't reach {} right now.", s.name)));
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
                        .child(pill("retry", "Retry", Some(Icon::Refresh), true).on_click(cx.listener(|this, _, _, cx| this.refresh_home(cx))))
                        .child(pill("change", "Change server", None, false).on_click(cx.listener(|this, _, _, cx| this.navigate(crate::app::Nav::Servers, true, cx))))
                        .when(self.has_downloads(), |d| {
                            d.child(pill("offline", "Downloads", Some(Icon::Download), false).on_click(cx.listener(|this, _, _, cx| this.navigate(crate::app::Nav::Downloads(None), true, cx))))
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
            let scrolled = -f32::from(self.home_scroll.offset().y);
            let mut top = if self.hero.is_empty() { 0. } else { vh * 0.75 + 16. };
            (0..self.hubs.len())
                .map(|i| {
                    let w = card_width(cw, self.hubs[i].wide, self.settings.card_scale());
                    let height = 38. + card_height(w, self.hubs[i].wide, self.settings.card_pad());
                    let near = top + height > scrolled - vh && top < scrolled + 2. * vh;
                    top += height + 16.;
                    self.render_hub(i, cw, near, window, cx)
                })
                .collect()
        };
        let page = div()
            .id("home")
            .size_full()
            .overflow_y_scroll()
            .restrict_scroll_to_axis()
            .track_scroll(&self.home_scroll)
            .child(div().flex().flex_col().gap(px(16.)).pb(px(24.)).children(hero).children(hubs))
            .child(
                div().absolute().top(px(12.)).right(px(12.)).child(
                    overlay_button("refresh", Icon::Refresh, 40., 20., theme::black(0.3), theme::black(0.5), rgb(0xFFFFFF)).on_click(cx.listener(|this, _, _, cx| this.refresh_home(cx))),
                ),
            );
        div().size_full().relative().child(page).child(scrollbar(&self.home_scroll)).into_any_element()
    }

    fn render_hero(&mut self, width: f32, height: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let item = self.hero[self.hero_index.min(self.hero.len() - 1)].clone();
        let current = self.image(&item.thumb, Shape::Backdrop);
        let arrived = self.image_alpha();
        let upcoming = self.hero[(self.hero_index + 1) % self.hero.len()].thumb.clone();
        self.image(&upcoming, Shape::Backdrop);
        let fade = self.view.hero_prev.map(|(i, t0)| (i, ease_out(progress(Some(t0), Duration::from_millis(500), window))));
        let previous = fade.filter(|(_, p)| *p < 1.).and_then(|(i, _)| self.hero.get(i).cloned()).and_then(|c| self.image(&c.thumb, Shape::Backdrop));
        let alpha = fade.map_or(1., |(_, p)| p);
        let bg = theme::bg();
        let dots = (self.hero.len() > 1).then(|| {
            div().absolute().bottom(px(24.)).left_0().right_0().flex().justify_center().gap(px(8.)).children((0..self.hero.len()).map(|i| {
                let active = i == self.hero_index;
                div()
                    .id(("dot", i))
                    .w(px(if active { 24. } else { 8. }))
                    .h(px(8.))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(if active { theme::text().into() } else { theme::tint(0.4) })
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
            .when_some(previous, |d, im| d.child(img(im).absolute().size_full().object_fit(ObjectFit::Cover)))
            .when_some(current, |d, im| d.child(img(im).absolute().size_full().object_fit(ObjectFit::Cover).opacity(alpha * arrived)))
            .child(div().absolute().size_full().bg(linear_gradient(180., linear_color_stop(bg.opacity(0.), 0.5), linear_color_stop(bg.opacity(1.), 0.94))))
            .child(div().absolute().size_full().bg(linear_gradient(90., linear_color_stop(bg.opacity(0.65), 0.), linear_color_stop(bg.opacity(0.), 0.6))))
            .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| this.hero_wheel(ev, cx)))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, _| this.view.hero_drag = Some(f32::from(ev.position.x))))
            .on_mouse_up(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.hero_release(f32::from(ev.position.x), cx)))
            .on_mouse_up_out(MouseButton::Left, cx.listener(|this, ev: &MouseUpEvent, _, cx| this.hero_release(f32::from(ev.position.x), cx)))
            .child(
                div()
                    .absolute()
                    .left(px(40.))
                    .bottom(px(64.))
                    .w(px((width - 240.).clamp(320., 760.)))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(div().text_size(px(40.)).font_weight(FontWeight::BOLD).line_height(px(46.)).child(item.title.clone()))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(item.meta.clone()))
                    .child(div().text_size(px(14.)).line_height(px(20.)).text_color(theme::tint(0.7)).line_clamp(2).child(item.summary.clone()))
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
                                        d.child(div().w(px(40.)).h(px(6.)).rounded(px(3.)).bg(theme::black(0.25)).child(div().h_full().rounded(px(3.)).w(gpui::relative(item.progress)).bg(rgb(0x000000))))
                                    })
                                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(0x000000)).child(if item.progress > 0. { "Resume" } else { "Play" }))
                                    .on_click(cx.listener(move |this, _, _, cx| this.play(play_key.0.clone(), play_key.1.clone(), cx))),
                            )
                            .child(pill("hero-details", "Details", None, false).on_click(cx.listener(move |this, _, _, cx| this.open(&open, cx)))),
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
        if self.view.hero_cooldown.is_some_and(|t| t.elapsed() < Duration::from_millis(450)) {
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

    fn render_hub_skeleton(&self, i: usize, cw: f32) -> AnyElement {
        let wide = i == 0;
        let w = card_width(cw, wide, self.settings.card_scale());
        let pad = self.settings.card_pad();
        let art_h = if wide { (w - 2. * pad) * 9. / 16. } else { (w - 2. * pad) * 1.5 };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(8.))
            .pt(if i == 0 { px(16.) } else { px(0.) })
            .child(div().pl(px(4.)).child(skeleton(px(180.), 20.)))
            .child(div().flex().overflow_hidden().children((0..8).map(|_| div().w(px(w)).flex_none().p(px(pad)).child(div().w(px(w - 2. * pad)).h(px(art_h)).rounded(px(8.)).bg(theme::surface())))))
            .into_any_element()
    }

    fn render_hub(&mut self, i: usize, cw: f32, near: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (title, resume, wide, scroll) = {
            let h = &self.hubs[i];
            (h.title.clone(), h.resume, h.wide, h.scroll.clone())
        };
        let w = card_width(cw, wide, self.settings.card_scale());
        let pad = self.settings.card_pad();
        let hovered = self.view.hub_hover == Some(i);
        if let Some((from, to, t0)) = self.hubs[i].slide {
            let p = ease_out(progress(Some(t0), SLIDE_ANIM, window));
            scroll.set_offset(point(px(from + (to - from) * p), px(0.)));
            if p >= 1. {
                self.hubs[i].slide = None;
            }
        }
        let offset = f32::from(scroll.offset().x);
        let first = ((-offset - cw) / w).floor().max(0.) as usize;
        let last = if near { ((-offset + 2. * cw) / w).ceil() as usize } else { 0 };
        let items = std::mem::take(&mut self.hubs[i].items);
        let cards: Vec<AnyElement> = items
            .iter()
            .enumerate()
            .map(|(j, c)| {
                if (first..last).contains(&j) {
                    self.media_card(("card", j), c, w, wide, cx)
                } else {
                    div().w(px(w)).h(px(card_height(w, wide, pad))).flex_none().into_any_element()
                }
            })
            .collect();
        self.hubs[i].items = items;
        let base = self.hubs[i].slide.map_or(offset, |(_, to, _)| to);
        let max = f32::from(scroll.max_offset().x);
        let page = (cw - 16.) * 0.8;
        let arrow_y = if wide { (w - 2. * pad) * 9. / 32. } else { (w - 2. * pad) * 0.75 } - 24. + pad;
        let left = hovered && offset < -1.;
        let right = hovered && -offset < max - 1.;
        div()
            .id(("hub", i))
            .flex()
            .flex_col()
            .gap(px(4.))
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                this.view.hub_hover = if *hovered { Some(i) } else if this.view.hub_hover == Some(i) { None } else { this.view.hub_hover };
                cx.notify();
            }))
            .child(
                div()
                    .h(px(34.))
                    .pl(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon(if resume { Icon::PlayCircle } else { Icon::Sparkle }, 24., theme::text()))
                    .child(div().text_size(px(22.)).child(title)),
            )
            .child(
                div()
                    .relative()
                    .child(div().id(("hub-scroll", i)).overflow_x_scroll().restrict_scroll_to_axis().track_scroll(&scroll).px(px(8.)).flex().children(cards))
                    .when(left, |d| {
                        d.child(
                            div().absolute().left(px(8.)).top(px(arrow_y)).child(
                                overlay_button(("hub-left", i), Icon::ChevronLeft, 48., 32., theme::black(0.7), theme::black(0.9), rgb(0xFFFFFF)).on_click(cx.listener(move |this, _, _, cx| {
                                    this.hubs[i].slide = Some((offset, (base + page).min(0.), Instant::now()));
                                    cx.notify();
                                })),
                            ),
                        )
                    })
                    .when(right, |d| {
                        d.child(
                            div().absolute().right(px(8.)).top(px(arrow_y)).child(
                                overlay_button(("hub-right", i), Icon::ChevronRight, 48., 32., theme::black(0.7), theme::black(0.9), rgb(0xFFFFFF)).on_click(cx.listener(move |this, _, _, cx| {
                                    this.hubs[i].slide = Some((offset, (base - page).max(-max), Instant::now()));
                                    cx.notify();
                                })),
                            ),
                        )
                    }),
            )
            .into_any_element()
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

