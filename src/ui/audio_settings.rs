use crate::app::{Category, Choice, Nav, Plaivo};
use crate::prefs::{self, LANGUAGES, LanguagePrefs, SubtitleMode};
use crate::settings::{SUBTITLE_COLORS, Settings};
use crate::ui::settings::{note, nudge};
use crate::ui::theme::Icon;
use crate::ui::widgets::{chip, nav_row, row, section, stepper};
use gpui::{Context, Div, div, prelude::*, px};

impl Plaivo {
    pub fn languages_summary(&self) -> String {
        format!("Audio: {} · Subtitles: {}", prefs::language_name(&self.settings.audio_language), self.settings.subtitle_mode.label())
    }

    pub fn sync_languages(&mut self, cx: &mut Context<Self>) {
        let (c, token, jellyfin, before) = (self.client.clone(), self.account.token.clone(), self.account.jellyfin.clone(), self.settings.languages());
        Self::bg(cx, move || prefs::account(&c, token.as_deref(), &jellyfin), move |this, r, _| match r {
            Ok(Some(p)) if p != before && this.settings.languages() == before => {
                this.settings.set_languages(&p);
                this.settings.save();
            }
            Ok(_) => {}
            Err(e) => log::warn!("could not load account language preferences: {e}"),
        });
    }

    fn edit_languages(&mut self, change: impl FnOnce(&mut LanguagePrefs), cx: &mut Context<Self>) {
        let mut p = self.settings.languages();
        change(&mut p);
        self.settings.set_languages(&p);
        self.settings.save();
        let (c, token, jellyfin, generation) = (self.client.clone(), self.account.token.clone(), self.account.jellyfin.clone(), prefs::next_generation());
        Self::bg(cx, move || prefs::save(&c, token.as_deref(), &jellyfin, &p, generation), |this, r, cx| {
            if let Err(e) = r {
                this.toast(format!("Could not save to your account: {e}"), cx);
            }
        });
    }

    fn style_row(&self, id: &'static str, title: &'static str, detail: &'static str, value: String, change: fn(&mut Settings, i8), cx: &mut Context<Self>) -> impl IntoElement {
        row(id, None, title, detail).child(stepper(
            id,
            value,
            cx.listener(move |this, step: &i8, _, cx| {
                change(&mut this.settings, *step);
                this.settings.save();
                this.apply_sub_style();
                cx.notify();
            }),
        ))
    }

    pub fn render_audio_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let s = &self.settings;
        let (audio, subtitles, mode, color) = (prefs::language_name(&s.audio_language), prefs::language_name(&s.subtitle_language), s.subtitle_mode, s.subtitle_color.clone());
        let off_or = |v: u32, text: String| if v == 0 { "Off".to_owned() } else { text };
        let lift = 100 - s.subtitle_position.min(100);
        let (size, outline, background) = (s.subtitle_size, s.subtitle_outline, s.subtitle_background);
        let modes = SubtitleMode::ALL.map(|m| {
            chip(("subtitle-mode", m as usize), m.label(), m == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.edit_languages(|p| p.mode = m, cx);
                cx.notify();
            }))
        });
        let colors = SUBTITLE_COLORS.iter().enumerate().map(|(i, &(hex, name))| {
            chip(("subtitle-color", i), name, hex.eq_ignore_ascii_case(&color)).on_click(cx.listener(move |this, _, _, cx| {
                this.settings.subtitle_color = hex.into();
                this.settings.save();
                this.apply_sub_style();
                cx.notify();
            }))
        }).collect::<Vec<_>>();
        let languages = section("Languages")
            .child(note("Saved to your Plex account and Jellyfin users, so other apps follow them too. This device keeps a copy for offline playback."))
            .child(nav_row("audio-language", Icon::Volume, "Audio language", audio).on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Settings(Some(Category::AudioLanguage)), true, cx))))
            .child(nav_row("subtitle-language", Icon::Subtitles, "Subtitle language", subtitles).on_click(cx.listener(|this, _, _, cx| this.navigate(Nav::Settings(Some(Category::SubtitleLanguage)), true, cx))))
            .child(row("subtitle-mode", None, "Show subtitles", "Forced subtitles for foreign dialogue show in every mode").child(div().flex().gap(px(6.)).children(modes)));
        let style = section("Subtitle style")
            .child(note("Applies to text subtitles. Styled ASS subtitles keep their own colours."))
            .child(self.style_row("subtitle-size", "Size", "Text size", format!("{size}%"), |s, step| s.subtitle_size = nudge(s.subtitle_size, step, 10, 50, 200, 100), cx))
            .child(row("subtitle-color", None, "Colour", "Text colour").child(div().flex().gap(px(6.)).children(colors)))
            .child(self.style_row("subtitle-outline", "Outline", "Black outline around the text", off_or(outline, outline.to_string()), |s, step| s.subtitle_outline = nudge(s.subtitle_outline, step, 1, 0, 6, 3), cx))
            .child(self.style_row("subtitle-background", "Background", "Opacity of the box behind the text", off_or(background, format!("{background}%")), |s, step| {
                s.subtitle_background = nudge(s.subtitle_background, step, 25, 0, 100, 0)
            }, cx))
            .child(self.style_row("subtitle-position", "Position", "Height above the bottom of the video", if lift == 0 { "Bottom".into() } else { format!("{lift}% up") }, |s, step| {
                s.subtitle_position = 100 - nudge(100 - s.subtitle_position.min(100), step, 5, 0, 50, 0)
            }, cx));
        div().flex().flex_col().gap(px(28.)).child(languages).child(self.render_language_rules(cx)).child(style)
    }

    pub fn render_language_list(&mut self, subtitles: bool, cx: &mut Context<Self>) -> Div {
        let current = if subtitles { &self.settings.subtitle_language } else { &self.settings.audio_language };
        let choices = std::iter::once(("", "Any"))
            .chain(LANGUAGES.iter().map(|(name, codes)| (codes[0], *name)))
            .map(|(code, name)| Choice { id: code.into(), label: name.into(), sub: "".into(), selected: *current == code })
            .collect();
        let prefix = if subtitles { "subtitle-language" } else { "audio-language" };
        section(if subtitles { "Preferred subtitle language" } else { "Preferred audio language" }).children(self.choice_list(prefix, None, choices, move |this, id, cx| {
            let id = id.to_owned();
            this.edit_languages(|p| if subtitles { p.subtitles = id } else { p.audio = id }, cx);
        }, cx))
    }
}
