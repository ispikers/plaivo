use crate::app::{Card, CastMember, Nav, Plaivo, Shape, ShelfKey};
use crate::plex::{DISCOVER_ID, RatingSource};
use crate::ui::home::{arrow_y, card_width, content_width};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{
    chip, icon, overlay_button, pill, progress_bar, round_button, scrollbar, skeleton, tip,
};
use gpui::{
    AnyElement, Context, Div, ElementId, FontWeight, Hsla, MouseButton, MouseDownEvent, ObjectFit,
    SharedString, Window, div, img, linear_color_stop, linear_gradient, prelude::*, px, rgb,
};

const CAST_W: f32 = 150.;
const SUMMARY_LINES: usize = 4;
const EXTRA_W: f32 = 240.;

const LOGO_W: f32 = 440.;
const LOGO_H: f32 = 150.;

fn badge(bg: Hsla) -> Div {
    div()
        .h(px(26.))
        .px(px(10.))
        .rounded_full()
        .flex()
        .items_center()
        .gap(px(6.))
        .bg(bg)
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
}

fn source_tag(source: RatingSource) -> AnyElement {
    let tag = |label: &'static str, bg: u32, fg: u32| {
        div()
            .h(px(16.))
            .px(px(4.))
            .rounded(px(3.))
            .flex()
            .items_center()
            .bg(rgb(bg))
            .text_color(rgb(fg))
            .text_size(px(10.))
            .font_weight(FontWeight::EXTRA_BOLD)
            .child(label)
            .into_any_element()
    };
    let glyph = |g: &'static str| div().text_size(px(13.)).child(g).into_any_element();
    match source {
        RatingSource::Imdb => tag("IMDb", 0xF5C518, 0x000000),
        RatingSource::Tmdb => tag("TMDB", 0x01B4E4, 0x0D253F),
        RatingSource::Critics => glyph("🍅"),
        RatingSource::Audience => glyph("🍿"),
        RatingSource::Stars => icon(Icon::Star, 14., theme::accent()).into_any_element(),
    }
}

fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .pt(px(16.))
        .pb(px(4.))
        .text_size(px(22.))
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

fn action(id: &'static str, i: Icon, label: &'static str) -> gpui::Stateful<Div> {
    round_button(
        id,
        i,
        48.,
        24.,
        theme::tint(0.08),
        theme::tint(0.16),
        theme::text(),
    )
    .tooltip(tip(label))
}

impl Plaivo {
    fn detail_shelf(
        &mut self,
        n: usize,
        id: impl Into<ElementId>,
        arrow_y: f32,
        children: Vec<AnyElement>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = ShelfKey::Detail(n);
        self.shelf_offset(key, window);
        self.shelf(
            key,
            id,
            arrow_y,
            div().mx(px(-3.)).flex().children(children),
            cx,
        )
        .into_any_element()
    }

    pub fn render_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cw = content_width(self, window);
        let vh = f32::from(window.viewport_size().height);
        let backdrop_h = (cw * 9. / 16.).clamp(vh * 0.6, vh * 0.86);
        let hero_h = vh * 0.6;
        let back = div().absolute().top(px(8.)).left(px(8.)).child(
            overlay_button(
                "back",
                Icon::Back,
                40.,
                20.,
                theme::black(0.3),
                theme::black(0.5),
                rgb(0xFFFFFF),
            )
            .on_click(cx.listener(|this, _, _, cx| this.back(cx))),
        );
        let Some(d) = self.detail.as_ref() else {
            return div()
                .size_full()
                .relative()
                .child(
                    div()
                        .absolute()
                        .left(px(24.))
                        .top(px(hero_h - 140.))
                        .flex()
                        .flex_col()
                        .gap(px(12.))
                        .child(skeleton(px(360.), 40.))
                        .child(div().flex().gap(px(4.)).children((0..4).map(|_| {
                            div()
                                .w(px(64.))
                                .h(px(30.))
                                .rounded_full()
                                .bg(theme::surface())
                        })))
                        .child(
                            div()
                                .w(px(140.))
                                .h(px(48.))
                                .rounded_full()
                                .bg(theme::surface()),
                        ),
                )
                .child(back)
                .into_any_element();
        };
        let (
            eyebrow,
            title,
            chips,
            genres,
            summary,
            play_label,
            play_key,
            server,
            progress,
            art,
            is_show,
        ) = (
            d.eyebrow.clone(),
            d.title.clone(),
            d.chips.clone(),
            d.genres.clone(),
            d.summary.clone(),
            d.play_label.clone(),
            d.play_key.clone(),
            d.server.clone(),
            d.progress,
            d.art.clone(),
            d.is_show,
        );
        let (logo, ratings, media_line, info, watchlisted, photo) = (
            d.logo.clone(),
            d.ratings.clone(),
            d.media_line.clone(),
            d.info.clone(),
            d.watchlisted,
            d.photo.clone(),
        );
        let it = &d.item;
        let person = it.kind == "person";
        let show_link =
            (it.kind == "episode" && !it.grandparent_rating_key.is_empty()).then(|| {
                Nav::Detail(
                    server.to_string(),
                    it.grandparent_rating_key.clone(),
                    Some(it.parent_rating_key.clone()).filter(|k| !k.is_empty()),
                )
            });
        let local = *server != *DISCOVER_ID;
        let library = local && matches!(it.kind.as_str(), "movie" | "episode" | "show" | "season");
        let watched = it.watched();
        let trailer = it.trailer().map(|t| t.rating_key.clone());
        let user_stars = it
            .user_rating
            .map(|r| (r / 2.).round() as u8)
            .filter(|&s| s > 0);
        let can_rate = library && self.server(&server).is_some_and(|s| s.can_rate());
        let can_watchlist = self.account.token.is_some()
            && it.plex_guid().is_some()
            && matches!(it.kind.as_str(), "movie" | "show");
        let menu_key = it.rating_key.clone();
        let backdrop = self.picture(&art, Shape::Backdrop);
        let logo = self
            .image(&logo, Shape::Logo)
            .map(|im| (im, self.image_alpha()));
        let bg = theme::bg();
        let episode_cards = std::mem::take(&mut self.episodes);
        let episodes: Vec<AnyElement> = episode_cards
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let download = self.episode_download_button(i, cx);
                self.episode_row(i, e, download, cx)
            })
            .collect();
        self.episodes = episode_cards;
        let download = self.detail_download_button(cx);
        let season_download = self.season_download_chip(cx);
        let title_el = match logo {
            Some((im, alpha)) => {
                let size = im.size(0);
                let (w, h) = (
                    i32::from(size.width).max(1) as f32,
                    i32::from(size.height).max(1) as f32,
                );
                let s = (LOGO_W / w).min(LOGO_H / h);
                img(im)
                    .w(px(w * s))
                    .h(px(h * s))
                    .opacity(alpha)
                    .into_any_element()
            }
            None => div()
                .text_size(px(40.))
                .line_height(px(46.))
                .font_weight(FontWeight::BOLD)
                .child(title)
                .into_any_element(),
        };
        let title_el = if person {
            let photo = self.picture(&photo, Shape::Poster);
            div()
                .flex()
                .items_end()
                .gap(px(24.))
                .child(
                    div()
                        .w(px(160.))
                        .h(px(240.))
                        .flex_none()
                        .rounded(px(12.))
                        .overflow_hidden()
                        .bg(theme::surface())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(match photo {
                            Some(im) => im
                                .size_full()
                                .rounded(px(12.))
                                .object_fit(ObjectFit::Cover)
                                .into_any_element(),
                            None => icon(Icon::Person, 64., theme::muted()).into_any_element(),
                        }),
                )
                .child(title_el)
                .into_any_element()
        } else {
            title_el
        };
        let chip_bg = Hsla::from(bg).opacity(0.55);
        let rate_chip = can_rate.then(|| {
            badge(chip_bg)
                .id("rate-chip")
                .cursor_pointer()
                .hover(|s| s.bg(theme::tint(0.16)))
                .child(icon(
                    if user_stars.is_some() {
                        Icon::Star
                    } else {
                        Icon::StarBorder
                    },
                    16.,
                    theme::accent(),
                ))
                .child(user_stars.map_or("Rate".to_owned(), |s| format!("{s}/5")))
                .on_click(cx.listener(|this, _, _, cx| this.detail_action_rate(cx)))
        });
        let actions = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .when(!play_key.is_empty(), |d| {
                let (s, k) = (server.to_string(), play_key.to_string());
                let (ws, wk) = (s.clone(), k.clone());
                d.child(
                    pill("play", play_label, Some(Icon::Play), true)
                        .h(px(48.))
                        .when(progress > 0., |p| {
                            p.child(
                                div()
                                    .w(px(40.))
                                    .h(px(4.))
                                    .rounded_full()
                                    .bg(theme::black(0.25))
                                    .child(
                                        div()
                                            .h_full()
                                            .rounded_full()
                                            .w(gpui::relative(progress))
                                            .bg(theme::on_accent()),
                                    ),
                            )
                        })
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.play(s.clone(), k.clone(), cx)),
                        ),
                )
                .child(
                    action("watch-together", Icon::Group, "Watch together").on_click(cx.listener(
                        move |this, _, _, cx| this.watch_together(ws.clone(), wk.clone(), cx),
                    )),
                )
            })
            .when_some(trailer, |d, key| {
                let s = server.to_string();
                d.child(action("trailer", Icon::Movie, "Play trailer").on_click(
                    cx.listener(move |this, _, _, cx| this.play(s.clone(), key.clone(), cx)),
                ))
            })
            .children(download)
            .when(library, |d| {
                d.child(
                    action(
                        "watched",
                        if watched { Icon::Remove } else { Icon::Check },
                        if watched {
                            "Mark as unwatched"
                        } else {
                            "Mark as watched"
                        },
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.detail_action_watched(!watched, cx)),
                    ),
                )
            })
            .when(can_watchlist, |d| {
                let on = watchlisted == Some(true);
                d.child(
                    action(
                        "watchlist",
                        if on {
                            Icon::Bookmark
                        } else {
                            Icon::BookmarkAdd
                        },
                        if on {
                            "Remove from watchlist"
                        } else {
                            "Add to watchlist"
                        },
                    )
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.detail_action_watchlist(!on, cx)),
                    ),
                )
            })
            .child({
                let s = server.to_string();
                action("more", Icon::MoreVert, "More").on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        this.open_item_menu(s.clone(), menu_key.clone(), ev.position, cx)
                    }),
                )
            })
            .child(div().flex_1())
            .when(!is_show && !media_line.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::tint(0.8))
                        .child(media_line),
                )
            });
        let info_rows = info.clone();
        let info_panel = (!info.is_empty()).then(|| {
            div()
                .absolute()
                .top(px(64.))
                .right(px(24.))
                .w(px(300.))
                .p(px(16.))
                .rounded(px(12.))
                .bg(bg.opacity(0.6))
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(info.into_iter().map(|(k, v)| {
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::tint(0.6))
                                .child(k),
                        )
                        .child(div().text_size(px(14.)).line_height(px(19.)).child(v))
                }))
        });
        let hero = div()
            .relative()
            .w_full()
            .min_h(px(hero_h))
            .flex()
            .flex_col()
            .justify_end()
            .when_some(backdrop, |d, im| {
                d.child(
                    im.absolute()
                        .top_0()
                        .left_0()
                        .w_full()
                        .h(px(backdrop_h))
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(backdrop_h))
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(bg.opacity(0.25), 0.),
                        linear_color_stop(bg.opacity(1.), 0.94),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(backdrop_h))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(bg.opacity(0.6), 0.),
                        linear_color_stop(bg.opacity(0.), 0.55),
                    )),
            )
            .when(cw >= 1000., |d| d.children(info_panel))
            .child(
                div()
                    .relative()
                    .px(px(24.))
                    .pt(px(74.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .when(!eyebrow.is_empty(), |d| {
                        d.child(
                            div().flex().child(
                                div()
                                    .id("eyebrow")
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::muted())
                                    .child(eyebrow)
                                    .when_some(show_link, |d, nav| {
                                        d.cursor_pointer()
                                            .hover(|s| s.text_color(theme::text()))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.navigate(nav.clone(), true, cx)
                                            }))
                                    }),
                            ),
                        )
                    })
                    .child(div().pb(px(4.)).child(title_el))
                    .child(
                        div()
                            .max_w(px(900.))
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(px(6.))
                            .children(chips.into_iter().map(|c| badge(chip_bg).child(c)))
                            .when(!ratings.is_empty(), |d| {
                                d.child(badge(chip_bg).gap(px(12.)).children(
                                    ratings.into_iter().map(|(source, value)| {
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(5.))
                                            .child(source_tag(source))
                                            .child(value)
                                    }),
                                ))
                            })
                            .children(rate_chip),
                    )
                    .when(!genres.is_empty(), |d| {
                        d.child(
                            div()
                                .max_w(px(900.))
                                .flex()
                                .flex_wrap()
                                .gap(px(6.))
                                .children(genres.into_iter().map(|g| badge(chip_bg).child(g))),
                        )
                    })
                    .when(!person, |d| d.child(div().pt(px(6.)).child(actions))),
            );
        let d = self.detail.as_mut().expect("checked above");
        let (cast, extras, related_rows) = (
            std::mem::take(&mut d.cast),
            std::mem::take(&mut d.extras),
            std::mem::take(&mut d.related),
        );
        let cast_cards: Vec<AnyElement> = cast
            .iter()
            .enumerate()
            .map(|(i, p)| self.cast_card(i, p, cx))
            .collect();
        let extra_cards: Vec<AnyElement> = extras
            .iter()
            .enumerate()
            .map(|(i, c)| self.media_card(("extra", i), c, EXTRA_W, true, cx))
            .collect();
        let pad = self.settings.card_pad();
        let cast_shelf = (!cast_cards.is_empty()).then(|| {
            let y = 3. + (CAST_W - 6.) / 2. - 24.;
            self.detail_shelf(0, "cast-shelf", y, cast_cards, window, cx)
        });
        let extras_shelf = (!extra_cards.is_empty()).then(|| {
            let y = arrow_y(EXTRA_W, true, pad);
            self.detail_shelf(1, "extras-shelf", y, extra_cards, window, cx)
        });
        let poster_w = card_width(cw - 32., false, self.settings.card_scale());
        let related: Vec<AnyElement> = related_rows
            .iter()
            .enumerate()
            .map(|(h, (title, cards))| {
                let cards: Vec<AnyElement> = cards
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        self.media_card(("related", h * 1000 + i), c, poster_w, false, cx)
                    })
                    .collect();
                let y = arrow_y(poster_w, false, pad);
                div()
                    .flex()
                    .flex_col()
                    .child(heading(title.clone()))
                    .child(self.detail_shelf(2 + h, ("related-shelf", h), y, cards, window, cx))
                    .into_any_element()
            })
            .collect();
        let d = self.detail.as_mut().expect("checked above");
        (d.cast, d.extras, d.related) = (cast, extras, related_rows);
        let posters = self.settings.season_posters;
        let season_tabs: Vec<AnyElement> = self
            .seasons
            .clone()
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                let id = s.id.to_string();
                let tab = chip(("season", i), s.label.clone(), s.selected);
                if !posters {
                    return tab
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.select_season(id.clone(), cx)),
                        )
                        .into_any_element();
                }
                let art = self
                    .season_items
                    .iter()
                    .find(|x| *x.rating_key == *s.id)
                    .and_then(|x| crate::app::thumb(&server, &x.thumb));
                let image = self.picture(&art, Shape::Poster);
                div()
                    .id(("season-tab", i))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(6.))
                    .cursor_pointer()
                    .child(
                        div()
                            .w(px(72.))
                            .h(px(108.))
                            .rounded(px(6.))
                            .overflow_hidden()
                            .bg(theme::surface())
                            .when_some(image, |d, im| {
                                d.child(im.size_full().rounded(px(6.)).object_fit(ObjectFit::Cover))
                            }),
                    )
                    .child(tab)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_season(id.clone(), cx)))
                    .into_any_element()
            })
            .collect();
        let narrow_info =
            (cw < 1000. && !info_rows.is_empty()).then(|| {
                div().pt(px(8.)).flex().flex_wrap().gap(px(32.)).children(
                    info_rows.into_iter().map(|(k, v)| {
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::muted())
                                    .child(k),
                            )
                            .child(div().text_size(px(14.)).child(v))
                    }),
                )
            });
        let mut style = window.text_style();
        style.font_family = ".SystemUIFont".into();
        let long = window
            .text_system()
            .shape_text(
                summary.clone(),
                px(16.),
                &[style.to_run(summary.len())],
                Some(px((cw - 48.).min(1100.))),
                None,
            )
            .is_ok_and(|lines| {
                lines
                    .iter()
                    .map(|l| l.wrap_boundaries().len() + 1)
                    .sum::<usize>()
                    > SUMMARY_LINES
            });
        let expanded = self.summary_expanded;
        let summary: SharedString = if long && !expanded {
            // line_clamp applies per paragraph, so join them to clamp the whole text.
            summary
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
                .into()
        } else {
            summary
        };
        let body = div()
            .px(px(24.))
            .pb(px(32.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .when(!summary.is_empty(), |d| {
                d.child(heading(if person { "Biography" } else { "Overview" }).pt(px(0.)))
                    .child(
                        div()
                            .text_size(px(16.))
                            .line_height(px(25.))
                            .max_w(px(1100.))
                            .when(long && !expanded, |d| d.line_clamp(SUMMARY_LINES))
                            .child(summary),
                    )
                    .when(long, |d| {
                        d.child(
                            div().flex().child(
                                div()
                                    .id("summary-more")
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::muted())
                                    .cursor_pointer()
                                    .hover(|s| s.text_color(theme::text()))
                                    .child(if expanded { "Less" } else { "More" })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.summary_expanded = !this.summary_expanded;
                                        cx.notify();
                                    })),
                            ),
                        )
                    })
            })
            .children(narrow_info)
            .when(is_show, |d| {
                d.child(heading("Episodes"))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_end()
                            .gap(px(8.))
                            .children(season_tabs)
                            .children(season_download),
                    )
                    .children(episodes)
            })
            .when_some(cast_shelf, |d, s| d.child(heading("Cast")).child(s))
            .when_some(extras_shelf, |d, s| {
                d.child(heading("Trailers & Extras")).child(s)
            })
            .children(related);
        div()
            .size_full()
            .relative()
            .child(
                div()
                    .id("detail")
                    .size_full()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .track_scroll(&self.detail_scroll)
                    .child(hero)
                    .child(body),
            )
            .child(scrollbar(&self.detail_scroll))
            .child(back)
            .into_any_element()
    }

    fn cast_card(&mut self, i: usize, p: &CastMember, cx: &mut Context<Self>) -> AnyElement {
        let image = self.picture(&p.thumb, Shape::Poster);
        let side = CAST_W - 6.;
        let server = self.detail.as_ref().map(|d| d.server.to_string());
        let person = server
            .filter(|_| !p.id.is_empty())
            .map(|s| Nav::Person(s, p.id.clone(), p.name.to_string()));
        div()
            .id(("cast", i))
            .w(px(CAST_W))
            .flex_none()
            .p(px(3.))
            .rounded(px(10.))
            .when_some(person, |d, nav| {
                d.cursor_pointer()
                    .hover(|s| s.bg(theme::surface()))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.navigate(nav.clone(), true, cx)),
                    )
            })
            .flex()
            .flex_col()
            .child(
                div()
                    .size(px(side))
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(theme::surface())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(match image {
                        Some(im) => im
                            .size_full()
                            .rounded(px(8.))
                            .object_fit(ObjectFit::Cover)
                            .into_any_element(),
                        None => icon(Icon::Person, 48., theme::muted()).into_any_element(),
                    }),
            )
            .child(
                div()
                    .pt(px(6.))
                    .w(px(side))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .child(p.name.clone()),
            )
            .child(
                div()
                    .w(px(side))
                    .text_size(px(12.))
                    .text_color(theme::muted())
                    .truncate()
                    .child(p.role.clone()),
            )
            .into_any_element()
    }

    pub fn episode_row(
        &mut self,
        i: usize,
        e: &Card,
        trailing: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let image = self.picture(&e.thumb, Shape::Wide);
        let (server, key) = (e.server.to_string(), e.key.to_string());
        let (menu_server, menu_key) = (server.clone(), key.clone());
        let check = e.watched && self.settings.watched_indicators;
        div()
            .id(("episode", i))
            .p(px(6.))
            .rounded(px(8.))
            .flex()
            .gap(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::surface()))
            .on_click(cx.listener(move |this, _, _, cx| this.play(server.clone(), key.clone(), cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                    this.open_item_menu(menu_server.clone(), menu_key.clone(), ev.position, cx)
                }),
            )
            .child(
                div()
                    .relative()
                    .w(px(144.))
                    .h(px(81.))
                    .flex_none()
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::surface())
                    .when_some(image, |d, im| {
                        d.child(im.size_full().rounded(px(6.)).object_fit(ObjectFit::Cover))
                    })
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .size(px(32.))
                                    .rounded_full()
                                    .bg(theme::black(0.6))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(icon(Icon::Play, 20., rgb(0xFFFFFF))),
                            ),
                    )
                    .when(check, |d| {
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
                    .when(e.progress > 0. && !e.watched, |d| {
                        d.child(
                            div()
                                .absolute()
                                .bottom(px(6.))
                                .left(px(6.))
                                .right(px(6.))
                                .child(progress_bar(e.progress, 3., theme::surface())),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(e.title.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(16.))
                            .text_color(theme::muted())
                            .line_clamp(3)
                            .child(e.summary.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme::muted())
                            .child(e.meta.clone()),
                    ),
            )
            .children(trailing)
            .into_any_element()
    }
}
