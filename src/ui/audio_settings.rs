use crate::app::{Category, Choice, Nav, Plaivo};
use crate::prefs::{self, LANGUAGES, LanguagePrefs, SubtitleMode};
use crate::settings::{SUBTITLE_COLORS, Settings};
use crate::ui::settings::nudge;
use crate::ui::theme::Icon;
use crate::ui::widgets::{Step, chip, nav_row, row, section, stepper};
use gpui::{Context, Div, FontWeight, Hsla, div, hsla, prelude::*, px, rgb};

const PREVIEW_H: f32 = 150.;

// Mirrors apply_sub_style: mpv drops the outline when the background box is on.
fn subtitle_preview(s: &Settings) -> Div {
    let color: Hsla = rgb(u32::from_str_radix(&s.subtitle_color, 16).unwrap_or(0xFF_FF_FF)).into();
    let back = s.subtitle_background.min(100) as f32 / 100.;
    let outline = if back > 0. {
        0.
    } else {
        s.subtitle_outline as f32 * 0.6
    };
    let lift = (100 - s.subtitle_position.min(100)) as f32 / 100.;
    let line = |c: Hsla| {
        div()
            .text_size(px(20. * s.subtitle_size as f32 / 100.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c)
            .whitespace_nowrap()
            .child("Subtitles will look like this")
    };
    let black = hsla(0., 0., 0., 1.);
    let mut caption = div().relative();
    if outline > 0. {
        for (dx, dy) in [
            (-1., -1.),
            (-1., 1.),
            (1., -1.),
            (1., 1.),
            (0., -1.),
            (0., 1.),
            (-1., 0.),
            (1., 0.),
        ] {
            caption = caption.child(
                line(black)
                    .absolute()
                    .left(px(dx * outline))
                    .top(px(dy * outline)),
            );
        }
    }
    div()
        .h(px(PREVIEW_H))
        .w_full()
        .rounded(px(12.))
        .overflow_hidden()
        .bg(hsla(220. / 360., 0.12, 0.16, 1.))
        .flex()
        .flex_col()
        .justify_end()
        .items_center()
        .pb(px(12. + lift * PREVIEW_H))
        .child(
            div()
                .px(px(6.))
                .bg(hsla(0., 0., 0., back))
                .child(caption.child(line(color))),
        )
}

impl Plaivo {
    pub fn languages_summary(&self) -> String {
        format!(
            "Audio: {} · Subtitles: {}",
            prefs::language_name(&self.settings.audio_language),
            self.settings.subtitle_mode.label()
        )
    }

    pub fn sync_languages(&mut self, cx: &mut Context<Self>) {
        let (c, token, jellyfin, before) = (
            self.client.clone(),
            self.account.token.clone(),
            self.account.jellyfin.clone(),
            self.settings.languages(),
        );
        Self::bg(
            cx,
            move || prefs::account(&c, token.as_deref(), &jellyfin),
            move |this, r, cx| match r {
                Ok(Some(p)) if p != before && this.settings.languages() == before => {
                    this.settings.set_languages(&p);
                    this.save_settings_soon(cx);
                }
                Ok(_) => {}
                Err(e) => log::warn!("could not load account language preferences: {e}"),
            },
        );
    }

    fn edit_languages(&mut self, change: impl FnOnce(&mut LanguagePrefs), cx: &mut Context<Self>) {
        let mut p = self.settings.languages();
        change(&mut p);
        self.settings.set_languages(&p);
        self.save_settings_soon(cx);
        let (c, token, jellyfin, generation) = (
            self.client.clone(),
            self.account.token.clone(),
            self.account.jellyfin.clone(),
            prefs::next_generation(),
        );
        Self::bg(
            cx,
            move || prefs::save(&c, token.as_deref(), &jellyfin, &p, generation),
            |this, r, cx| {
                if let Err(e) = r {
                    this.toast(format!("Could not save to your account: {e}"), cx);
                }
            },
        );
    }

    #[expect(clippy::too_many_arguments)]
    fn style_row(
        id: &'static str,
        title: &'static str,
        detail: &'static str,
        label: String,
        value: u32,
        field: fn(&mut Settings) -> &mut u32,
        nudged: impl Fn(u32, Step) -> u32 + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        row(id, None, title, detail).child(stepper(
            id,
            label,
            (
                nudged(value, Step::Down) != value,
                nudged(value, Step::Up) != value,
            ),
            cx.listener(move |this, step: &Step, _, cx| {
                let f = field(&mut this.settings);
                *f = nudged(*f, *step);
                this.save_settings_soon(cx);
                this.apply_sub_style();
                cx.notify();
            }),
        ))
    }

    pub fn render_audio_settings(&mut self, cx: &mut Context<Self>) -> Div {
        let s = &self.settings;
        let (audio, subtitles, mode, color) = (
            prefs::language_name(&s.audio_language),
            prefs::language_name(&s.subtitle_language),
            s.subtitle_mode,
            s.subtitle_color.as_str(),
        );
        let off_or = |v: u32, text: String| if v == 0 { "Off".to_owned() } else { text };
        let lift = 100 - s.subtitle_position.min(100);
        let (size, outline, background, position) = (
            s.subtitle_size,
            s.subtitle_outline,
            s.subtitle_background,
            s.subtitle_position,
        );
        let d = Settings::default();
        let modes = SubtitleMode::ALL.map(|m| {
            chip(("subtitle-mode", m as usize), m.label(), m == mode).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.edit_languages(|p| p.mode = m, cx);
                    cx.notify();
                },
            ))
        });
        let colors = SUBTITLE_COLORS
            .iter()
            .enumerate()
            .map(|(i, &(hex, name))| {
                chip(("subtitle-color", i), name, hex.eq_ignore_ascii_case(color)).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.settings.subtitle_color = hex.into();
                        this.save_settings_soon(cx);
                        this.apply_sub_style();
                        cx.notify();
                    }),
                )
            })
            .collect::<Vec<_>>();
        let languages = section("Languages")
            .child(
                nav_row("audio-language", Icon::Volume, "Audio language", audio).on_click(
                    cx.listener(|this, _, _, cx| {
                        this.navigate(Nav::Settings(Some(Category::AudioLanguage)), true, cx)
                    }),
                ),
            )
            .child(
                nav_row(
                    "subtitle-language",
                    Icon::Subtitles,
                    "Subtitle language",
                    subtitles,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.navigate(Nav::Settings(Some(Category::SubtitleLanguage)), true, cx)
                })),
            )
            .child(
                row(
                    "subtitle-mode",
                    None,
                    "Show subtitles",
                    "Forced subtitles always show",
                )
                .child(div().flex().gap(px(6.)).children(modes)),
            );
        let style = section("Subtitle style")
            .child(subtitle_preview(&self.settings))
            .child(Self::style_row(
                "subtitle-size",
                "Size",
                "Text size",
                format!("{size}%"),
                size,
                |s| &mut s.subtitle_size,
                move |v, step| nudge(v, step, 10, 50, 200, d.subtitle_size),
                cx,
            ))
            .child(
                row("subtitle-color", None, "Colour", "Text colour")
                    .child(div().flex().gap(px(6.)).children(colors)),
            )
            .child(Self::style_row(
                "subtitle-outline",
                "Outline",
                "Black outline around the text",
                off_or(outline, outline.to_string()),
                outline,
                |s| &mut s.subtitle_outline,
                move |v, step| nudge(v, step, 1, 0, 6, d.subtitle_outline),
                cx,
            ))
            .child(Self::style_row(
                "subtitle-background",
                "Background",
                "Opacity of the box behind the text",
                off_or(background, format!("{background}%")),
                background,
                |s| &mut s.subtitle_background,
                move |v, step| nudge(v, step, 25, 0, 100, d.subtitle_background),
                cx,
            ))
            .child(Self::style_row(
                "subtitle-position",
                "Position",
                "Height above the bottom of the video",
                if lift == 0 {
                    "Bottom".into()
                } else {
                    format!("{lift}% up")
                },
                position,
                |s| &mut s.subtitle_position,
                move |v, step| {
                    100 - nudge(
                        100 - v.min(100),
                        step,
                        5,
                        0,
                        50,
                        100 - d.subtitle_position.min(100),
                    )
                },
                cx,
            ));
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(languages)
            .child(self.render_language_rules(cx))
            .child(style)
    }

    pub fn render_language_list(&mut self, subtitles: bool, cx: &mut Context<Self>) -> Div {
        let current = if subtitles {
            &self.settings.subtitle_language
        } else {
            &self.settings.audio_language
        };
        let choices = std::iter::once(("", "Any"))
            .chain(LANGUAGES.iter().map(|(name, codes)| (codes[0], *name)))
            .map(|(code, name)| Choice {
                id: code.into(),
                label: name.into(),
                sub: "".into(),
                selected: *current == code,
            })
            .collect();
        let prefix = if subtitles {
            "subtitle-language"
        } else {
            "audio-language"
        };
        section(if subtitles {
            "Preferred subtitle language"
        } else {
            "Preferred audio language"
        })
        .children(self.choice_list(
            prefix,
            None,
            choices,
            move |this, id, cx| {
                let id = id.to_owned();
                this.edit_languages(
                    |p| {
                        if subtitles {
                            p.subtitles = id
                        } else {
                            p.audio = id
                        }
                    },
                    cx,
                );
            },
            cx,
        ))
    }
}
