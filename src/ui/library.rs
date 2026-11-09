use crate::app::{Card, LIB_TABS, LibTab, Listing, Nav, Plaivo, Shape};
use crate::plex::SORTS;
use crate::settings::{DENSITY_MAX, ViewMode};
use crate::ui::home::{card_height, card_width, content_width};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{backdrop, chip, floating, icon, round_button, scrollbar, shadow};
use gpui::{AnyElement, Context, Div, FontWeight, MouseButton, MouseDownEvent, ObjectFit, SharedString, Stateful, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    Library,
    HubList,
    Results,
    Suggestions,
    Offline,
}

impl Plaivo {
    fn grid_len(&self, g: Grid) -> usize {
        match g {
            Grid::Library => self.library.as_ref().map_or(0, |l| l.items.len()),
            Grid::HubList => self.hub_list.as_ref().map_or(0, |l| l.cards.len()),
            Grid::Results => self.results.len(),
            Grid::Suggestions => self.suggestions.len(),
            Grid::Offline => self.offline_cards.len(),
        }
    }

    fn grid_item(&self, g: Grid, i: usize) -> Card {
        match g {
            Grid::Library => self.library.as_ref().and_then(|l| l.items.get(i).cloned().flatten()),
            Grid::HubList => self.hub_list.as_ref().and_then(|l| l.cards.get(i).cloned()),
            Grid::Results => self.results.get(i).cloned(),
            Grid::Suggestions => self.suggestions.get(i).cloned(),
            Grid::Offline => self.offline_cards.get(i).cloned(),
        }
        .unwrap_or_default()
    }

    pub fn grid(&mut self, g: Grid, cw: f32, scroll: UniformListScrollHandle, cx: &mut Context<Self>) -> AnyElement {
        if g == Grid::Library && self.settings.view_mode == ViewMode::List {
            return self.list(scroll, cx);
        }
        let w = card_width(cw, false, self.settings.card_scale());
        let cols = ((cw - 16.) / w).round().max(1.) as usize;
        let count = self.grid_len(g);
        let rows = count.div_ceil(cols);
        let row_h = card_height(w, false, self.settings.card_pad());
        let tag = match g {
            Grid::Library => "lib",
            Grid::HubList => "hublist",
            Grid::Results => "res",
            Grid::Suggestions => "sug",
            Grid::Offline => "offline",
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

    fn list(&mut self, scroll: UniformListScrollHandle, cx: &mut Context<Self>) -> AnyElement {
        let density = self.settings.density.clamp(1, DENSITY_MAX);
        let f = (density - 1) as f32 / 4.;
        let poster_w = 70. + 50. * f;
        let row_h = poster_w * 1.5 + 16.;
        let summary_lines = [2, 2, 3, 4, 5][density as usize - 1];
        let count = self.grid_len(Grid::Library);
        uniform_list(
            "lib-list",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                this.ensure_library_rows(range.start, range.end.min(count), cx);
                range
                    .map(|i| {
                        let card = this.grid_item(Grid::Library, i);
                        let image = this.picture(&card.thumb, Shape::Poster);
                        let check = card.watched && this.settings.watched_indicators;
                        let (open, server, key) = (card.clone(), card.server.to_string(), card.key.to_string());
                        div()
                            .id(SharedString::from(format!("lib-row-{i}")))
                            .h(px(row_h))
                            .mx(px(8.))
                            .p(px(8.))
                            .rounded(px(8.))
                            .flex()
                            .gap(px(12.))
                            .when(!card.key.is_empty(), |d| {
                                d.cursor_pointer()
                                    .hover(|s| s.bg(theme::tint(0.05)))
                                    .on_click(cx.listener(move |this, _, _, cx| this.open(&open, cx)))
                                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &MouseDownEvent, _, cx| this.open_item_menu(server.clone(), key.clone(), ev.position, cx)))
                            })
                            .child(
                                div()
                                    .relative()
                                    .w(px(poster_w))
                                    .h(px(poster_w * 1.5))
                                    .flex_none()
                                    .rounded(px(8.))
                                    .overflow_hidden()
                                    .bg(theme::surface())
                                    .when_some(image, |d, im| d.child(im.size_full().rounded(px(8.)).object_fit(ObjectFit::Cover)))
                                    .when(check, |d| {
                                        d.child(div().absolute().top(px(4.)).right(px(4.)).size(px(20.)).rounded_full().bg(theme::text()).flex().items_center().justify_center().child(icon(Icon::Check, 14., theme::bg())))
                                    }),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.))
                                    .child(div().text_size(px(13. + 3. * f)).font_weight(FontWeight::SEMIBOLD).line_clamp(2).child(card.title.clone()))
                                    .child(div().text_size(px(10. + 3. * f)).text_color(theme::muted()).truncate().child(card.meta.clone()))
                                    .child(div().text_size(px(10. + 3. * f)).line_height(px((10. + 3. * f) * 1.3)).text_color(theme::tint(0.5)).line_clamp(summary_lines).child(card.summary.clone())),
                            )
                    })
                    .collect()
            }),
        )
        .track_scroll(&scroll)
        .flex_1()
        .into_any_element()
    }

    pub fn render_library(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cw = content_width(self, window);
        let tabbed = matches!(self.nav.last(), Some(Nav::Library(..)));
        let tab = self.lib_tab;
        let header = div().h(px(64.)).flex_none().px(px(16.)).flex().items_center().gap(px(12.)).when(tabbed, |d| {
            d.child(div().flex().gap(px(8.)).children(LIB_TABS.iter().enumerate().map(|(i, &(t, label))| chip(("lib-tab", i), label, t == tab).on_click(cx.listener(move |this, _, _, cx| this.set_lib_tab(t, cx))))))
        });
        if tabbed && tab == LibTab::Recommended {
            let body = self.render_lib_hubs(cw, window, cx);
            return div().size_full().flex().flex_col().child(header).child(body).into_any_element();
        }
        let Some(lib) = &self.library else { return div().size_full().flex().flex_col().child(header).into_any_element() };
        let (title, count, scroll, sort, listing) = (lib.title.clone(), lib.items.len(), lib.scroll.clone(), lib.key.sort.clone(), lib.key.listing);
        let loaded = lib.items.iter().any(Option::is_some);
        let browse = listing == Listing::Items;
        let body = if lib.items.is_empty() {
            message(if listing == Listing::Collections { "No collections" } else { "Nothing here" })
        } else {
            self.grid(Grid::Library, cw, scroll, cx)
        };
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(
                header
                    .when(listing == Listing::Collection, |d| d.child(back_button("collection-back", cx)).child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child(title)))
                    .when(loaded, |d| d.child(div().text_size(px(13.)).text_color(theme::muted()).child(format!("{count} items"))))
                    .child(div().flex_1())
                    .when(browse, |d| d.child(self.sort_chip(cx))),
            )
            .child(body)
            .when(self.sort_open && browse, |d| d.children(self.sort_menu(&sort, SORTS.to_vec(), false, cx)))
            .into_any_element()
    }

    fn render_lib_hubs(&mut self, cw: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let rows = match self.lib_hubs.as_ref().map(Vec::len) {
            None => (0..3).map(|i| self.render_hub_skeleton(i, cw)).collect(),
            Some(0) => vec![message("Nothing to recommend yet")],
            Some(_) => self.render_hubs(true, cw, 0., window, cx),
        };
        div()
            .flex_1()
            .min_h_0()
            .relative()
            .child(div().id("lib-hubs").size_full().overflow_y_scroll().restrict_scroll_to_axis().track_scroll(&self.lib_hubs_scroll).child(div().flex().flex_col().gap(px(16.)).pb(px(24.)).children(rows)))
            .child(scrollbar(&self.lib_hubs_scroll))
            .into_any_element()
    }

    pub fn render_hub_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(l) = &self.hub_list else { return div().into_any_element() };
        let (title, sort, scroll, loaded, empty) = (l.title.clone(), l.sort.clone(), l.scroll.clone(), l.loaded, l.cards.is_empty());
        let options = l.sorts();
        let cw = content_width(self, window);
        let body = if empty {
            message(if loaded { "Nothing here" } else { "" })
        } else {
            self.grid(Grid::HubList, cw, scroll, cx)
        };
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
                    .child(back_button("hub-back", cx))
                    .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child(title))
                    .child(div().flex_1())
                    .child(self.sort_chip(cx)),
            )
            .child(body)
            .when(self.sort_open, |d| d.children(self.sort_menu(&sort, options, true, cx)))
            .into_any_element()
    }

    fn sort_chip(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        chip("sort", "Sort", false).gap(px(6.)).child(icon(Icon::Sort, 16., theme::text())).on_click(cx.listener(|this, _, _, cx| {
            this.sort_open = !this.sort_open;
            cx.notify();
        }))
    }

    fn sort_menu(&self, sort: &str, options: Vec<(&'static str, &'static str)>, hub: bool, cx: &mut Context<Self>) -> [AnyElement; 2] {
        let close = backdrop(
            "sort-backdrop",
            cx.listener(|this, _, _, cx| {
                this.sort_open = false;
                cx.notify();
            }),
        )
        .into_any_element();
        let menu = floating("sort-menu")
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
            .children(options.into_iter().map(|(id, label)| {
                let selected = id == sort;
                div()
                    .id(SharedString::from(format!("sort-{id}")))
                    .h(px(40.))
                    .px(px(12.))
                    .rounded(px(8.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.06)))
                    .child(div().flex_1().font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).text_color(if selected { theme::text().into() } else { theme::muted() }).child(label))
                    .when(selected, |d| d.child(icon(Icon::Check, 18., theme::text())))
                    .on_click(cx.listener(move |this, _, _, cx| if hub { this.set_hub_sort(id, cx) } else { this.set_sort(id, cx) }))
            }))
            .into_any_element();
        [close, menu]
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

fn message(text: &'static str) -> AnyElement {
    div().pt(px(24.)).flex().justify_center().text_color(theme::muted()).child(text).into_any_element()
}

fn back_button(id: &'static str, cx: &mut Context<Plaivo>) -> Stateful<Div> {
    round_button(id, Icon::Back, 40., 22., theme::black(0.), theme::tint(0.08), theme::text()).on_click(cx.listener(|this, _, _, cx| this.back(cx)))
}
