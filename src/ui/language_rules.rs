use crate::app::{Category, Nav, Plaivo, join};
use crate::prefs::{LANGUAGES, LanguageRule, SubtitleMode, language_name};
use crate::settings::library_key;
use crate::ui::settings::note;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{chip, icon, nav_row, pill, round_button, row, scroll_box, section};
use gpui::{AnyElement, Context, Div, ElementId, FontWeight, SharedString, Stateful, Window, div, prelude::*, px};


const ADD_LIST_H: f32 = 260.;

pub struct RuleDraft {
    index: Option<usize>,
    rule: LanguageRule,
    adding: Option<bool>,
}

fn toggle(list: &mut Vec<String>, value: &str) {
    match list.iter().position(|v| v.eq_ignore_ascii_case(value)) {
        Some(i) => {
            list.remove(i);
        }
        None => list.push(value.to_owned()),
    }
}

fn check_row(id: impl Into<ElementId>, title: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .h(px(40.))
        .px(px(14.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(10.))
        .cursor_pointer()
        .hover(|s| s.bg(theme::tint(0.08)))
        .when(on, |d| d.bg(theme::tint(0.06)))
        .child(div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).child(title.into()))
        .when(on, |d| d.child(icon(Icon::Check, 18., theme::text())))
}

fn card(title: &'static str, hint: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .p(px(16.))
        .rounded(px(12.))
        .bg(theme::surface())
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(div().flex().items_baseline().gap(px(8.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(title)).child(div().text_size(px(12.)).text_color(theme::muted()).child(hint.into())))
}

impl Plaivo {
    fn rule_targets(&self, r: &LanguageRule) -> String {
        let libraries = r.libraries.iter().map(|id| self.sections.iter().find(|s| &s.id == id).map_or_else(|| "Missing library".to_owned(), |s| s.title.to_string()));
        libraries.collect::<Vec<_>>().join(", ")
    }

    fn rule_detail(r: &LanguageRule) -> String {
        let names = |codes: &[String]| codes.iter().map(|c| language_name(c)).collect::<Vec<_>>().join(", ");
        join(&[
            if r.audio.is_empty() { String::new() } else { format!("Audio: {}", names(&r.audio)) },
            if r.subtitles.is_empty() { String::new() } else { format!("Subtitles: {}", names(&r.subtitles)) },
            r.subtitle_mode.label().to_owned(),
        ])
    }

    pub fn render_language_rules(&mut self, cx: &mut Context<Self>) -> Div {
        let rows: Vec<AnyElement> = self
            .settings
            .language_rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                row(("language-rule", i), Some(Icon::Sort), self.rule_targets(r), Self::rule_detail(r))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme::tint(0.12)))
                    .child(icon(Icon::ChevronRight, 22., theme::muted()))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_language_rule(Some(i), cx)))
                    .into_any_element()
            })
            .collect();
        section("Overrides")
            .child(note("Use different languages for some libraries, like Japanese audio for an anime library. Saved on this device. The first matching override wins."))
            .children(rows)
            .child(nav_row("language-rule-add", Icon::Add, "Add override", "Pick libraries and the languages to use").on_click(cx.listener(|this, _, _, cx| this.open_language_rule(None, cx))))
    }

    fn open_language_rule(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let rule = index.and_then(|i| self.settings.language_rules.get(i).cloned()).unwrap_or_else(|| LanguageRule { subtitle_mode: SubtitleMode::Always, ..Default::default() });
        self.rule_draft = Some(RuleDraft { index, rule, adding: None });
        self.navigate(Nav::Settings(Some(Category::LanguageOverride)), true, cx);
    }

    fn edit_rule(&mut self, f: impl FnOnce(&mut RuleDraft), cx: &mut Context<Self>) {
        if let Some(d) = &mut self.rule_draft {
            f(d);
        }
        cx.notify();
    }

    fn save_rule(&mut self, delete: bool, cx: &mut Context<Self>) {
        let Some(d) = self.rule_draft.take() else { return };
        let rules = &mut self.settings.language_rules;
        match (d.index, delete) {
            (Some(i), true) if i < rules.len() => {
                rules.remove(i);
            }
            (Some(i), false) if i < rules.len() => rules[i] = d.rule,
            (None, false) => rules.push(d.rule),
            _ => {}
        }
        self.settings.save();
        self.back(cx);
    }

    pub fn rule_actions(&self, cx: &mut Context<Self>) -> Option<Div> {
        let d = self.rule_draft.as_ref()?;
        let targeted = !d.rule.libraries.is_empty();
        Some(
            div()
                .flex()
                .gap(px(12.))
                .when(d.index.is_some(), |a| a.child(pill("rule-delete", "Delete", Some(Icon::Delete), false).on_click(cx.listener(|this, _, _, cx| this.save_rule(true, cx)))))
                .child(pill("rule-save", "Save", Some(Icon::Check), true).when(!targeted, |p| p.opacity(0.4)).when(targeted, |p| p.on_click(cx.listener(|this, _, _, cx| this.save_rule(false, cx))))),
        )
    }

    fn language_order(&self, d: &RuleDraft, subtitles: bool, cx: &mut Context<Self>) -> Div {
        let list = if subtitles { &d.rule.subtitles } else { &d.rule.audio };
        let prefix = if subtitles { "rule-sub" } else { "rule-audio" };
        let n = list.len();
        let rows = list.iter().enumerate().map(|(i, code)| {
            let mv = move |delta: isize| {
                move |this: &mut Self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                    this.edit_rule(
                        |d| {
                            let list = if subtitles { &mut d.rule.subtitles } else { &mut d.rule.audio };
                            if let Some(j) = i.checked_add_signed(delta).filter(|j| *j < list.len()) {
                                list.swap(i, j);
                            }
                        },
                        cx,
                    )
                }
            };
            let remove = move |this: &mut Self, _: &gpui::ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                this.edit_rule(
                    |d| {
                        (if subtitles { &mut d.rule.subtitles } else { &mut d.rule.audio }).remove(i);
                    },
                    cx,
                )
            };
            div()
                .h(px(44.))
                .pl(px(14.))
                .pr(px(4.))
                .rounded(px(8.))
                .bg(theme::tint(0.06))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().w(px(18.)).text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(theme::muted()).child(format!("{}", i + 1)))
                .child(div().flex_1().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(language_name(code)))
                .child(round_button((prefix, i * 3), Icon::ExpandLess, 34., 20., theme::black(0.), theme::tint(0.1), theme::text()).when(i == 0, |b| b.opacity(0.3)).when(i > 0, |b| b.on_click(cx.listener(mv(-1)))))
                .child(round_button((prefix, i * 3 + 1), Icon::ExpandMore, 34., 20., theme::black(0.), theme::tint(0.1), theme::text()).when(i + 1 == n, |b| b.opacity(0.3)).when(i + 1 < n, |b| b.on_click(cx.listener(mv(1)))))
                .child(round_button((prefix, i * 3 + 2), Icon::Close, 34., 18., theme::black(0.), theme::tint(0.1), theme::muted()).on_click(cx.listener(remove)))
        });
        let open = d.adding == Some(subtitles);
        let add_list = open.then(|| {
            scroll_box((prefix, 9999usize))
                .max_h(px(ADD_LIST_H))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::outline())
                .p(px(4.))
                .flex()
                .flex_col()
                .children(LANGUAGES.iter().enumerate().filter(|(_, (_, codes))| !list.iter().any(|c| c == codes[0])).map(|(i, (name, codes))| {
                    let code = codes[0];
                    check_row((prefix, 10_000 + i), *name, false).on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_rule(
                            |d| {
                                (if subtitles { &mut d.rule.subtitles } else { &mut d.rule.audio }).push(code.to_owned());
                                d.adding = None;
                            },
                            cx,
                        )
                    }))
                }))
        });
        let (title, hint) = if subtitles { ("Subtitles", "The first language the video has is used") } else { ("Audio", "The first language the video has is used") };
        card(title, hint)
            .when(list.is_empty(), |c| c.child(div().text_size(px(13.)).text_color(theme::muted()).child("Uses your account language")))
            .children(rows)
            .child(
                div().flex().child(
                    chip((prefix, 9998usize), if open { "Done" } else { "Add language" }, false)
                        .gap(px(6.))
                        .child(icon(if open { Icon::ExpandLess } else { Icon::Add }, 16., theme::text()))
                        .on_click(cx.listener(move |this, _, _, cx| this.edit_rule(|d| d.adding = if d.adding == Some(subtitles) { None } else { Some(subtitles) }, cx))),
                ),
            )
            .children(add_list)
    }

    pub fn render_language_override(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.rule_draft.as_ref() else { return div().child(note("Open an override from Audio & Subtitles to edit it.")) };
        let r = &d.rule;
        let multi = self.servers.len() > 1;
        let mut libraries = card("Libraries", "every video in the library");
        for srv in &self.servers {
            let sections: Vec<_> = self.sections.iter().enumerate().filter(|(_, s)| s.server == srv.id).collect();
            if sections.is_empty() {
                continue;
            }
            if multi {
                libraries = libraries.child(div().pt(px(6.)).px(px(14.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::muted()).child(srv.name.clone()));
            }
            for (i, s) in sections {
                let id = library_key(&s.server, &s.key);
                let on = r.libraries.contains(&id);
                libraries = libraries.child(check_row(("rule-library", i), s.title.clone(), on).on_click(cx.listener(move |this, _, _, cx| this.edit_rule(|d| toggle(&mut d.rule.libraries, &id), cx))));
            }
        }
        let modes = SubtitleMode::ALL
            .into_iter()
            .map(|m| chip(("rule-mode", m as usize), m.label(), m == r.subtitle_mode).on_click(cx.listener(move |this, _, _, cx| this.edit_rule(|d| d.rule.subtitle_mode = m, cx))));
        let mode = card("Show subtitles", "Forced subtitles for foreign dialogue show in every mode").child(div().flex().gap(px(6.)).children(modes));
        let left = div().flex_1().min_w_0().flex().flex_col().gap(px(12.)).child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::muted()).child("Applies to")).child(libraries);
        let right = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(theme::muted()).child("Languages"))
            .child(self.language_order(d, false, cx))
            .child(self.language_order(d, true, cx))
            .child(mode);
        div().flex().items_start().gap(px(24.)).child(left).child(right)
    }
}
