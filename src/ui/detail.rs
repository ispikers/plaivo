use crate::app::{Card, Plaivo, Shape};
use crate::ui::home::content_width;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{chip, icon, info_chip, pill, progress_bar, round_button, skeleton};
use gpui::{AnyElement, Context, FontWeight, ObjectFit, Window, div, img, linear_color_stop, linear_gradient, prelude::*, px, rgb};

impl Plaivo {
    pub fn render_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let cw = content_width(self, window);
        let vh = f32::from(window.viewport_size().height);
        let backdrop_h = (cw * 9. / 16.).clamp(vh * 0.6, vh * 0.86);
        let hero_h = vh * 0.6;
        let back = div().absolute().top(px(8.)).left(px(8.)).child(round_button("back", Icon::Back, 40., 20., theme::black(0.3), theme::black(0.5), rgb(0xFFFFFF)).on_click(cx.listener(|this, _, _, cx| this.back(cx))));
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
                        .child(div().flex().gap(px(4.)).children((0..4).map(|_| div().w(px(64.)).h(px(30.)).rounded_full().bg(theme::surface()))))
                        .child(div().w(px(140.)).h(px(48.)).rounded_full().bg(theme::surface())),
                )
                .child(back)
                .into_any_element();
        };
        let (eyebrow, title, chips, genres, summary, play_label, play_key, server, progress, art, is_show) =
            (d.eyebrow.clone(), d.title.clone(), d.chips.clone(), d.genres.clone(), d.summary.clone(), d.play_label.clone(), d.play_key.clone(), d.server.clone(), d.progress, d.art.clone(), d.is_show);
        let backdrop = self.image(&art, Shape::Backdrop);
        let bg = theme::bg();
        let episodes: Vec<AnyElement> = self
            .episodes
            .clone()
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let download = self.episode_download_button(i, cx);
                self.episode_row(i, e, download, cx)
            })
            .collect();
        let download = self.detail_download_button(cx);
        let season_download = self.season_download_chip(cx);
        let hero = div()
            .relative()
            .w_full()
            .min_h(px(hero_h))
            .flex()
            .flex_col()
            .justify_end()
            .when_some(backdrop, |d, im| d.child(img(im).absolute().top_0().left_0().w_full().h(px(backdrop_h)).object_fit(ObjectFit::Cover)))
            .child(div().absolute().top_0().left_0().w_full().h(px(backdrop_h)).bg(linear_gradient(180., linear_color_stop(bg.opacity(0.25), 0.), linear_color_stop(bg.opacity(1.), 0.94))))
            .child(
                div()
                    .relative()
                    .px(px(24.))
                    .pt(px(74.))
                    .pb(px(16.))
                    .max_w(px(900.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .when(!eyebrow.is_empty(), |d| d.child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::muted()).child(eyebrow)))
                    .child(div().text_size(px(40.)).line_height(px(46.)).font_weight(FontWeight::BOLD).child(title))
                    .child(div().flex().flex_wrap().gap(px(4.)).children(chips.into_iter().map(info_chip)))
                    .when(!genres.is_empty(), |d| d.child(div().flex().flex_wrap().gap(px(4.)).children(genres.into_iter().map(info_chip))))
                    .child(
                        div()
                            .pt(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .when(!play_key.is_empty(), |d| {
                                let (s, k) = (server.to_string(), play_key.to_string());
                                let (ws, wk) = (s.clone(), k.clone());
                                d.child(pill("play", play_label, Some(Icon::Play), true).h(px(48.)).on_click(cx.listener(move |this, _, _, cx| this.play(s.clone(), k.clone(), cx))))
                                    .child(pill("watch-together", "Watch together", Some(Icon::Group), false).h(px(48.)).on_click(cx.listener(move |this, _, _, cx| this.watch_together(ws.clone(), wk.clone(), cx))))
                            })
                            .children(download)
                            .when(progress > 0., |d| d.child(div().w(px(120.)).rounded(px(2.)).overflow_hidden().child(progress_bar(progress, 4., theme::outline())))),
                    ),
            );
        let body = div()
            .px(px(24.))
            .pb(px(32.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .when(!summary.is_empty(), |d| d.child(div().text_size(px(16.)).line_height(px(25.)).max_w(px(1000.)).child(summary)))
            .when(is_show, |d| {
                d.child(div().pt(px(12.)).text_size(px(22.)).font_weight(FontWeight::BOLD).child("Episodes"))
                    .child(div().flex().flex_wrap().gap(px(8.)).children(self.seasons.iter().enumerate().map(|(i, s)| {
                        let id = s.id.to_string();
                        chip(("season", i), s.label.clone(), s.selected).on_click(cx.listener(move |this, _, _, cx| this.select_season(id.clone(), cx)))
                    }))
                    .children(season_download))
                    .children(episodes)
            });
        div()
            .size_full()
            .relative()
            .child(div().id("detail").size_full().overflow_y_scroll().track_scroll(&self.detail_scroll).child(hero).child(body))
            .child(back)
            .into_any_element()
    }

    pub fn episode_row(&mut self, i: usize, e: &Card, trailing: Option<AnyElement>, cx: &mut Context<Self>) -> AnyElement {
        let image = self.image(&e.thumb, Shape::Wide);
        let (server, key) = (e.server.to_string(), e.key.to_string());
        div()
            .id(("episode", i))
            .p(px(6.))
            .rounded(px(8.))
            .flex()
            .gap(px(12.))
            .cursor_pointer()
            .hover(|s| s.bg(theme::surface()))
            .on_click(cx.listener(move |this, _, _, cx| this.play(server.clone(), key.clone(), cx)))
            .child(
                div()
                    .relative()
                    .w(px(144.))
                    .h(px(81.))
                    .flex_none()
                    .rounded(px(6.))
                    .overflow_hidden()
                    .bg(theme::surface())
                    .when_some(image, |d, im| d.child(img(im).size_full().rounded(px(6.)).object_fit(ObjectFit::Cover)))
                    .child(div().absolute().size_full().flex().items_center().justify_center().child(div().size(px(32.)).rounded_full().bg(theme::black(0.6)).flex().items_center().justify_center().child(icon(Icon::Play, 20., rgb(0xFFFFFF)))))
                    .when(e.watched, |d| {
                        d.child(div().absolute().top(px(3.)).right(px(3.)).size(px(18.)).rounded_full().bg(theme::text()).flex().items_center().justify_center().child(icon(Icon::Check, 12., theme::bg())))
                    })
                    .when(e.progress > 0. && !e.watched, |d| d.child(div().absolute().bottom(px(6.)).left(px(6.)).right(px(6.)).child(progress_bar(e.progress, 3., theme::surface())))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).truncate().child(e.title.clone()))
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(theme::muted()).line_clamp(3).child(e.summary.clone()))
                    .child(div().text_size(px(12.)).text_color(theme::muted()).child(e.meta.clone())),
            )
            .children(trailing)
            .into_any_element()
    }
}

