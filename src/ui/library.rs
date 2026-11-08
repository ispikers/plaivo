use crate::app::{Card, Plaivo, SORTS};
use crate::ui::home::{card_width, content_width};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{chip, icon, shadow};
use gpui::{AnyElement, Context, FontWeight, SharedString, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Grid {
    Library,
    Results,
    Suggestions,
}

impl Plaivo {
    fn grid_len(&self, g: Grid) -> usize {
        match g {
            Grid::Library => self.library.as_ref().map_or(0, |l| l.items.len()),
            Grid::Results => self.results.len(),
            Grid::Suggestions => self.suggestions.len(),
        }
    }

    fn grid_item(&self, g: Grid, i: usize) -> Card {
        match g {
            Grid::Library => self.library.as_ref().and_then(|l| l.items.get(i).cloned().flatten()),
            Grid::Results => self.results.get(i).cloned(),
            Grid::Suggestions => self.suggestions.get(i).cloned(),
        }
        .unwrap_or_default()
    }

    fn grid(&mut self, g: Grid, cw: f32, scroll: UniformListScrollHandle, cx: &mut Context<Self>) -> AnyElement {
        let w = card_width(cw, false, self.settings.card_scale());
        let cols = ((cw - 16.) / w).round().max(1.) as usize;
        let count = self.grid_len(g);
        let rows = count.div_ceil(cols);
        let row_h = (w - 6.) * 1.5 + 46.;
        let tag = match g {
            Grid::Library => "lib",
            Grid::Results => "res",
            Grid::Suggestions => "sug",
        };
        uniform_list(
            tag,
            rows,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                if g == Grid::Library {
                    this.ensure_library_rows(range.start * cols, (range.end * cols).min(count), cx);
                }
                range
                    .map(|r| {
                        let cards: Vec<AnyElement> = (r * cols..((r + 1) * cols).min(count))
                            .map(|i| {
                                let card = this.grid_item(g, i);
                                this.media_card(SharedString::from(format!("{tag}-{i}")), &card, w, false, cx)
                            })
                            .collect();
                        div().h(px(row_h)).px(px(8.)).flex().children(cards)
                    })
                    .collect()
            }),
        )
        .track_scroll(&scroll)
        .flex_1()
        .into_any_element()
    }

    pub fn render_library(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(lib) = &self.library else { return div().into_any_element() };
        let (title, count, scroll, sort) = (lib.title.clone(), lib.items.len(), lib.scroll.clone(), lib.key.sort.clone());
        let loaded = lib.items.iter().any(Option::is_some);
        let cw = content_width(self, window);
        let grid = self.grid(Grid::Library, cw, scroll, cx);
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(64.))
                    .flex_none()
                    .px(px(16.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child(title))
                    .when(loaded, |d| d.child(div().text_size(px(13.)).text_color(theme::muted()).child(format!("{count} items"))))
                    .child(div().flex_1())
                    .child(
                        chip("sort", "Sort", false)
                            .gap(px(6.))
                            .child(icon(Icon::Sort, 16., theme::text()))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sort_open = !this.sort_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(grid)
            .when(self.sort_open, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(56.))
                        .right(px(16.))
                        .w(px(220.))
                        .p(px(6.))
                        .rounded(px(12.))
                        .bg(theme::surface())
                        .shadow(shadow(24., 0.5))
                        .flex()
                        .flex_col()
                        .children(SORTS.iter().map(|(id, label)| {
                            let selected = *id == sort;
                            div()
                                .id(SharedString::from(format!("sort-{id}")))
                                .h(px(40.))
                                .px(px(12.))
                                .rounded(px(8.))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::tint(0.06)))
                                .child(div().flex_1().font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).text_color(if selected { theme::text().into() } else { theme::muted() }).child(*label))
                                .when(selected, |d| d.child(icon(Icon::Check, 18., theme::text())))
                                .on_click(cx.listener(move |this, _, _, cx| this.set_sort(id, cx)))
                        })),
                )
            })
            .into_any_element()
    }

    pub fn render_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let query = self.search.read(cx).text().to_owned();
        let focused = self.search.read(cx).focus.is_focused(window);
        let cw = content_width(self, window);
        let field = div()
            .id("search-field")
            .h(px(48.))
            .mx(px(16.))
            .px(px(14.))
            .rounded(px(12.))
            .flex()
            .items_center()
            .gap(px(10.))
            .flex_none()
            .bg(theme::tint(if focused { 0.18 } else { 0.08 }))
            .child(icon(Icon::Search, 20., theme::muted()))
            .child(self.search.clone());
        let body: Vec<AnyElement> = if query.trim().is_empty() {
            let mut out = Vec::new();
            if !self.settings.recent_searches.is_empty() {
                out.push(div().px(px(16.)).text_size(px(18.)).font_weight(FontWeight::BOLD).child("Recent searches").into_any_element());
                out.push(
                    div()
                        .px(px(16.))
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .children(self.settings.recent_searches.clone().into_iter().enumerate().map(|(i, q)| {
                            let q2 = q.clone();
                            chip(("recent", i), q, false).on_click(cx.listener(move |this, _, _, cx| {
                                this.search.update(cx, |input, cx| input.set_text(&q2, cx));
                                this.search(q2.clone(), cx);
                            }))
                        }))
                        .into_any_element(),
                );
            }
            if !self.suggestions.is_empty() {
                out.push(div().px(px(16.)).text_size(px(18.)).font_weight(FontWeight::BOLD).child("Suggested for you").into_any_element());
                let scroll = self.grid_scroll.clone();
                out.push(self.grid(Grid::Suggestions, cw, scroll, cx));
            }
            out
        } else if self.results.is_empty() {
            let msg = if self.searching { "Searching…".to_owned() } else { format!("No results for “{}”", query.trim()) };
            vec![div().pt(px(24.)).flex().justify_center().text_color(theme::muted()).child(msg).into_any_element()]
        } else {
            let scroll = self.grid_scroll.clone();
            vec![self.grid(Grid::Results, cw, scroll, cx)]
        };
        div().size_full().flex().flex_col().pt(px(16.)).gap(px(12.)).child(field).children(body).into_any_element()
    }
}
