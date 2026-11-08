use crate::app::{Card, Choice, Plaivo, Shape};
use crate::ui::theme::{self, Icon};
use gpui::{
    AnyElement, App, BoxShadow, Bounds, ClickEvent, Context, DispatchPhase, Div, ElementId, FontWeight, Hsla, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels,
    Img, ScrollHandle, SharedString, Stateful, Svg, Window, canvas, div, fill, img, point, prelude::*, px, size, svg,
};
use std::cell::RefCell;
use std::rc::Rc;

pub fn icon(i: Icon, size: f32, color: impl Into<Hsla>) -> Svg {
    svg().path(i.path()).size(px(size)).flex_none().text_color(color)
}

pub fn shadow(blur: f32, alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color: theme::black(alpha), offset: point(px(0.), px(blur / 4.)), blur_radius: px(blur), spread_radius: px(0.), inset: false }]
}

pub fn skeleton(w: impl Into<gpui::Length>, h: f32) -> Div {
    div().w(w.into()).h(px(h)).rounded(px(4.)).bg(theme::surface())
}

pub fn progress_bar(value: f32, height: f32, track: impl Into<Hsla>) -> Div {
    div().w_full().h(px(height)).rounded_full().bg(track.into()).child(div().h_full().rounded_full().w(gpui::relative(value.clamp(0., 1.))).bg(theme::accent()))
}

pub fn pill(id: impl Into<ElementId>, label: impl Into<SharedString>, icon_: Option<Icon>, primary: bool) -> Stateful<Div> {
    let (bg, fg) = if primary { (Hsla::from(theme::accent()), Hsla::from(theme::on_accent())) } else { (Hsla::from(theme::surface()), Hsla::from(theme::text())) };
    let label = label.into();
    div()
        .id(id.into())
        .h(px(44.))
        .px(px(18.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .bg(bg)
        .when(!primary, |d| d.border_1().border_color(theme::outline()))
        .cursor_pointer()
        .hover(move |s| s.bg(if primary { bg.opacity(0.88) } else { theme::tint(0.12) }))
        .when_some(icon_, |d, i| d.child(icon(i, 20., fg)))
        .when(!label.is_empty(), |d| d.child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(fg).child(label)))
}

pub fn chip(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .h(px(36.))
        .px(px(16.))
        .rounded(px(18.))
        .flex()
        .items_center()
        .flex_none()
        .cursor_pointer()
        .text_size(px(14.))
        .when(selected, |d| d.bg(theme::accent()).text_color(theme::on_accent()).font_weight(FontWeight::SEMIBOLD))
        .when(!selected, |d| d.bg(theme::surface()).text_color(theme::text()).hover(|s| s.bg(theme::tint(0.12))))
        .child(label.into())
}

pub fn round_button(id: impl Into<ElementId>, i: Icon, size: f32, icon_size: f32, idle: Hsla, hover: Hsla, fg: impl Into<Hsla>) -> Stateful<Div> {
    div()
        .id(id.into())
        .size(px(size))
        .flex_none()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(idle)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon(i, icon_size, fg))
}

pub fn backdrop(id: impl Into<ElementId>, close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Stateful<Div> {
    let close = Rc::new(close);
    let right = close.clone();
    floating(id).absolute().top_0().left_0().size_full().on_mouse_down(MouseButton::Left, move |e, w, cx| close(e, w, cx)).on_mouse_down(MouseButton::Right, move |e, w, cx| right(e, w, cx))
}

pub fn floating(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id.into()).occlude()
}

pub fn overlay_button(id: impl Into<ElementId>, i: Icon, size: f32, icon_size: f32, idle: Hsla, hover: Hsla, fg: impl Into<Hsla>) -> Stateful<Div> {
    round_button(id, i, size, icon_size, idle, hover, fg).occlude()
}

struct ScrollDrag {
    handle: ScrollHandle,
    start_y: f32,
    start_offset: f32,
    ratio: f32,
    max: f32,
}

thread_local! {
    static SCROLL_DRAG: RefCell<Option<ScrollDrag>> = const { RefCell::new(None) };
}

const SCROLLBAR_W: f32 = 6.;
const SCROLLBAR_MIN_THUMB: f32 = 40.;

pub fn scrollbar(handle: &ScrollHandle) -> impl IntoElement {
    let handle = handle.clone();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let max = f32::from(handle.max_offset().y);
            if max <= 0. {
                return;
            }
            let track_h = f32::from(bounds.size.height);
            let thumb_h = (track_h * track_h / (track_h + max)).clamp(SCROLLBAR_MIN_THUMB.min(track_h), track_h);
            let ratio = max / (track_h - thumb_h).max(1.);
            let offset = (-f32::from(handle.offset().y)).clamp(0., max);
            let x = bounds.right() - px(SCROLLBAR_W + 4.);
            let thumb = Bounds::new(point(x, bounds.top() + px(offset / ratio)), size(px(SCROLLBAR_W), px(thumb_h)));
            let track = Bounds::new(point(x - px(6.), bounds.top()), size(px(SCROLLBAR_W + 12.), bounds.size.height));
            let dragging = SCROLL_DRAG.with(|d| d.borrow().is_some());
            window.paint_quad(fill(thumb, theme::tint(if dragging { 0.5 } else { 0.25 })).corner_radii(px(SCROLLBAR_W / 2.)));
            let down = handle.clone();
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || e.button != MouseButton::Left || !track.contains(&e.position) {
                    return;
                }
                let y = f32::from(e.position.y);
                let start_offset = if thumb.contains(&e.position) { offset } else { ((y - f32::from(track.top()) - thumb_h / 2.) * ratio).clamp(0., max) };
                down.set_offset(point(px(0.), px(-start_offset)));
                SCROLL_DRAG.with(|d| *d.borrow_mut() = Some(ScrollDrag { handle: down.clone(), start_y: y, start_offset, ratio, max }));
                cx.stop_propagation();
                window.refresh();
            });
            window.on_mouse_event(|e: &MouseMoveEvent, _, window, _| {
                SCROLL_DRAG.with(|d| {
                    if let Some(s) = d.borrow().as_ref().filter(|_| e.dragging()) {
                        let next = (s.start_offset + (f32::from(e.position.y) - s.start_y) * s.ratio).clamp(0., s.max);
                        s.handle.set_offset(point(px(0.), px(-next)));
                        window.refresh();
                    }
                });
            });
            window.on_mouse_event(|_: &MouseUpEvent, _, window, _| {
                if SCROLL_DRAG.with(|d| d.borrow_mut().take()).is_some() {
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn scroll_box(id: impl Into<ElementId>) -> Stateful<Div> {
    div().id(id.into()).overflow_y_scroll().on_scroll(|_, _, cx| cx.stop_propagation())
}

pub fn section(title: impl Into<SharedString>) -> Div {
    div().flex().flex_col().gap(px(8.)).child(div().pb(px(4.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::muted()).child(title.into()))
}

pub fn row(id: impl Into<ElementId>, i: Option<Icon>, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Stateful<Div> {
    let detail = detail.into();
    div()
        .id(id.into())
        .w_full()
        .min_h(px(64.))
        .px(px(20.))
        .py(px(10.))
        .rounded(px(12.))
        .bg(theme::surface())
        .flex()
        .items_center()
        .gap(px(16.))
        .when_some(i, |d, i| d.child(icon(i, 24., theme::muted())))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).truncate().child(title.into()))
                .when(!detail.is_empty(), |d| d.child(div().text_size(px(12.)).text_color(theme::muted()).truncate().child(detail))),
        )
}

pub fn toggle(on: bool) -> Div {
    let (track, knob) = if on { (Hsla::from(theme::accent()), Hsla::from(theme::on_accent())) } else { (theme::tint(0.2), Hsla::from(theme::text())) };
    div().w(px(40.)).h(px(24.)).flex_none().p(px(3.)).rounded_full().flex().when(on, |d| d.justify_end()).bg(track).child(div().size(px(18.)).rounded_full().bg(knob))
}

pub fn nav_row(id: impl Into<ElementId>, i: Icon, title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Stateful<Div> {
    row(id, Some(i), title, detail).cursor_pointer().hover(|s| s.bg(theme::tint(0.12))).child(icon(Icon::ChevronRight, 22., theme::muted()))
}

pub fn stepper(id: &'static str, value: impl Into<SharedString>, on: impl Fn(&i8, &mut Window, &mut App) + 'static) -> Div {
    let on = Rc::new(on);
    let click = |step: i8| {
        let on = on.clone();
        move |_: &ClickEvent, window: &mut Window, cx: &mut App| on(&step, window, cx)
    };
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(round_button((id, 0usize), Icon::Remove, 32., 18., theme::black(0.), theme::tint(0.08), theme::text()).on_click(click(-1)))
        .child(div().id((id, 1usize)).min_w(px(72.)).flex().justify_center().text_size(px(14.)).cursor_pointer().tooltip(tip("Reset")).child(value.into()).on_click(click(0)))
        .child(round_button((id, 2usize), Icon::Add, 32., 18., theme::black(0.), theme::tint(0.08), theme::text()).on_click(click(1)))
}

pub struct Tip(pub SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().px(px(10.)).py(px(6.)).rounded(px(6.)).bg(theme::surface()).border_1().border_color(theme::outline()).shadow(shadow(8., 0.4)).text_size(px(13.)).text_color(theme::text()).child(self.0.clone())
    }
}

pub fn tip(text: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let text = text.into();
    move |_, cx| cx.new(|_| Tip(text.clone())).into()
}

pub fn bounds_probe(store: impl FnOnce(Bounds<Pixels>, &mut Window, &mut App) + 'static) -> impl IntoElement {
    canvas(move |bounds, window, cx| store(bounds, window, cx), |_, _, _, _| {}).absolute().size_full()
}

pub fn fmt_time(t: f64) -> String {
    let s = t.max(0.0) as u64;
    let (h, m, sec) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }
}

impl Plaivo {
    pub fn choice_list(&self, prefix: &'static str, i: Option<Icon>, choices: Vec<Choice>, pick: impl Fn(&mut Self, &str, &mut Context<Self>) + Copy + 'static, cx: &mut Context<Self>) -> Vec<AnyElement> {
        choices
            .into_iter()
            .enumerate()
            .map(|(n, c)| {
                let id = c.id;
                row((prefix, n), i, c.label, c.sub)
                    .cursor_pointer()
                    .when(c.selected, |d| d.bg(theme::tint(0.10)).child(icon(Icon::Check, 20., theme::text())))
                    .hover(|s| s.bg(theme::tint(0.12)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        pick(this, &id, cx);
                        cx.notify();
                    }))
                    .into_any_element()
            })
            .collect()
    }

    pub fn picture(&mut self, thumb: &Option<crate::app::Thumb>, shape: Shape) -> Option<Img> {
        let im = self.image(thumb, shape)?;
        Some(img(im).opacity(self.image_alpha()))
    }

    pub fn media_card(&mut self, id: impl Into<ElementId>, card: &Card, width: f32, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let pad = self.settings.card_pad();
        let art_w = width - 2. * pad;
        let art_h = if wide { art_w * 9. / 16. } else { art_w * 1.5 };
        let check = card.watched && self.settings.watched_indicators;
        let image = self.picture(&card.thumb, if wide { Shape::Wide } else { Shape::Poster });
        let placeholder = card.key.is_empty();
        let open = card.clone();
        let (menu_server, menu_key) = (card.server.to_string(), card.key.to_string());
        let art = div()
            .relative()
            .w(px(art_w))
            .h(px(art_h))
            .rounded(px(8.))
            .overflow_hidden()
            .bg(theme::surface())
            .when_some(image, |d, im| d.child(im.size_full().rounded(px(8.)).object_fit(ObjectFit::Cover)))
            .when(card.progress > 0. && !card.watched, |d| d.child(div().absolute().bottom(px(6.)).left(px(6.)).right(px(6.)).child(progress_bar(card.progress, 4., theme::surface()))))
            .when(check, |d| {
                d.child(
                    div().absolute().top(px(4.)).right(px(4.)).size(px(24.)).rounded_full().bg(theme::text()).flex().items_center().justify_center().shadow(shadow(4., 0.3)).child(icon(Icon::Check, 16., theme::bg())),
                )
            })
            .when(!card.watched && card.unwatched > 0, |d| {
                let n = if card.unwatched > 999 { "999+".to_owned() } else { card.unwatched.to_string() };
                d.child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .right(px(4.))
                        .h(px(24.))
                        .min_w(px(24.))
                        .px(px(6.))
                        .rounded_full()
                        .bg(theme::text())
                        .flex()
                        .items_center()
                        .justify_center()
                        .shadow(shadow(4., 0.3))
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme::bg())
                        .child(n),
                )
            });
        let caption = if placeholder {
            div().flex().flex_col().gap(px(6.)).pt(px(7.)).child(skeleton(px(art_w * 0.7), 10.)).child(skeleton(px(art_w * 0.4), 8.))
        } else {
            div()
                .flex()
                .flex_col()
                .pt(px(2. + pad * 2. / 3.))
                .w(px(art_w))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::text()).truncate().child(card.title.clone()))
                .child(div().text_size(px(11.)).text_color(theme::muted()).truncate().child(card.subtitle.clone()))
        };
        div()
            .id(id.into())
            .w(px(width))
            .h(px(art_h + 40. + 2. * pad))
            .flex_none()
            .p(px(pad))
            .rounded(px(8.))
            .flex()
            .flex_col()
            .when(!placeholder, |d| {
                d.cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.05)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open(&open, cx)))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &MouseDownEvent, _, cx| this.open_item_menu(menu_server.clone(), menu_key.clone(), ev.position, cx)))
            })
            .child(art)
            .child(caption)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_format_like_players_do() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(65.9), "1:05");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-3.0), "0:00");
    }
}
