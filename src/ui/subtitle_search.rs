use crate::app::{Plaivo, first, join};
use crate::player::Panel;
use crate::plex::{Ctx, Source, Stream, SubtitleResult};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{chip, icon};
use gpui::{Context, FontWeight, SharedString, Window, div, prelude::*, px};
use plaivo_core::prefs::{LANGUAGES, language_label, two_letter};
use std::time::Duration;

const POLL: Duration = Duration::from_millis(1500);
const POLL_TRIES: usize = 10;
const CHIP_LANGUAGES: usize = 10;

fn subtitle_keys(streams: &[Stream]) -> Vec<String> {
    streams.iter().filter(|s| s.stream_type == 3 && !s.key.is_empty()).map(|s| s.key.clone()).collect()
}

impl Plaivo {
    pub fn open_subtitle_search(&mut self, cx: &mut Context<Self>) {
        let language = two_letter(self.preferred_subtitle_language()).unwrap_or("en").to_owned();
        self.open_panel(Panel::SubtitleSearch, cx);
        self.search_subtitles(language, cx);
    }

    fn search_subtitles(&mut self, language: String, cx: &mut Context<Self>) {
        let Some(srv) = self.player.server.clone() else { return };
        let (c, key) = (self.client.clone(), self.player.key.clone());
        let s = &mut self.player.sub_search;
        s.generation += 1;
        let generation = s.generation;
        s.language = language.clone();
        s.results = None;
        s.downloading = None;
        Self::bg(cx, move || srv.search_subtitles(&c, &key, &language), move |this, r, _| {
            let s = &mut this.player.sub_search;
            if s.generation == generation {
                s.results = Some(r);
            }
        });
        cx.notify();
    }

    fn download_subtitle(&mut self, result: SubtitleResult, cx: &mut Context<Self>) {
        let Some(srv) = self.player.server.clone() else { return };
        if self.player.sub_search.downloading.is_some() {
            return;
        }
        self.player.sub_search.downloading = Some(result.key.clone());
        let (c, key, language) = (self.client.clone(), self.player.key.clone(), self.player.sub_search.language.clone());
        let before = self.player.media.part.first().map(|p| subtitle_keys(&p.stream)).unwrap_or_default();
        let watching = key.clone();
        Self::bg(
            cx,
            move || -> Result<Option<Stream>, String> {
                srv.download_subtitle(&c, &key, &result, &language)?;
                let ctx = Ctx::new(c.clone(), Source::Live);
                for _ in 0..POLL_TRIES {
                    std::thread::sleep(POLL);
                    let Ok(it) = srv.metadata(&ctx, &key) else { continue };
                    let streams = it.media.into_iter().flat_map(|m| m.part).flat_map(|p| p.stream);
                    if let Some(new) = streams.filter(|s| s.stream_type == 3 && !s.key.is_empty()).find(|s| !before.contains(&s.key)) {
                        return Ok(Some(new));
                    }
                }
                Ok(None)
            },
            move |this, r, cx| {
                this.player.sub_search.downloading = None;
                if this.player.key != watching {
                    return;
                }
                match r {
                    Ok(Some(st)) => {
                        let url = this.player.server.as_ref().map(|s| s.media_url(&st.key)).unwrap_or_default();
                        if let Some(part) = this.player.media.part.first_mut() {
                            part.stream.push(st.clone());
                        }
                        this.add_external_subtitle(url, st.display_title.clone(), st.language_code.clone());
                        this.player.panel = None;
                        this.toast("Subtitle added", cx);
                    }
                    Ok(None) => this.toast("Plex is still fetching the subtitle. It will be listed next time you play this.", cx),
                    Err(e) => this.toast(format!("Could not download the subtitle: {e}"), cx),
                }
            },
        );
        cx.notify();
    }

    pub fn render_subtitle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vh = f32::from(window.viewport_size().height);
        let s = &self.player.sub_search;
        let current = s.language.clone();
        let preferred = two_letter(self.preferred_subtitle_language());
        let mut codes: Vec<&'static str> = preferred.into_iter().chain(LANGUAGES.iter().filter_map(|(_, c)| c.iter().copied().find(|c| c.len() == 2))).collect();
        let mut seen = std::collections::HashSet::new();
        codes.retain(|c| seen.insert(*c));
        codes.truncate(CHIP_LANGUAGES);
        let chips = div().flex().flex_wrap().gap(px(6.)).p(px(12.)).children(codes.into_iter().enumerate().map(|(i, code)| {
            let selected = code == current;
            chip(("sub-lang", i), language_label(code).unwrap_or(code), selected).on_click(cx.listener(move |this, _, _, cx| this.search_subtitles(code.to_owned(), cx)))
        }));
        let downloading = s.downloading.clone();
        let body = match &s.results {
            None => div().p(px(16.)).text_color(theme::muted()).child("Searching…").into_any_element(),
            Some(Err(e)) => div().p(px(16.)).text_color(theme::muted()).child(format!("Search failed: {e}")).into_any_element(),
            Some(Ok(list)) if list.is_empty() => div().p(px(16.)).text_color(theme::muted()).child("No subtitles found for this language.").into_any_element(),
            Some(Ok(list)) => div()
                .id("sub-results")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(8.))
                .pb(px(8.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .children(list.iter().enumerate().map(|(i, r)| {
                    let busy = downloading.as_deref() == Some(r.key.as_str());
                    let tags = [
                        r.provider_title.clone(),
                        r.score.map(|s| format!("Score {s:.0}")).unwrap_or_default(),
                        if r.perfect_match { "Perfect match".into() } else { String::new() },
                        if r.hearing_impaired { "SDH".into() } else { String::new() },
                        if r.forced { "Forced".into() } else { String::new() },
                    ];
                    let result = r.clone();
                    div()
                        .id(("sub-result", i))
                        .min_h(px(44.))
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
                                .child(div().text_size(px(14.)).truncate().child(SharedString::from(first(&[&r.title, &r.display_title]).to_owned())))
                                .child(div().text_size(px(12.)).text_color(theme::muted()).truncate().child(join(&tags))),
                        )
                        .child(if busy { div().text_size(px(12.)).text_color(theme::muted()).child("Downloading…").into_any_element() } else { icon(Icon::Download, 20., theme::muted()).into_any_element() })
                        .on_click(cx.listener(move |this, _, _, cx| this.download_subtitle(result.clone(), cx)))
                }))
                .into_any_element(),
        };
        self.panel_shell("sub-search", Panel::SubtitleSearch, cx)
            .w(px(640.))
            .h(px((vh - 220.).clamp(240., 620.)))
            .child(chips)
            .child(div().flex_1().min_h_0().flex().flex_col().font_weight(FontWeight::NORMAL).child(body))
    }
}
