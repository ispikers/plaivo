use crate::app::{Category, Choice, Nav, Plaivo, join};
use crate::plex::QUALITIES;
use crate::settings::PlaybackRule;
use crate::shaders;
use crate::ui::item_menu::Pick;
use crate::ui::language_rules::{card, column, rule_buttons, rule_columns, toggle};
use crate::ui::settings::{note, quality_label};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, nav_row, row, section};
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, div, prelude::*, px};

const RULE_DEFAULT: &str = "*";

pub struct PlaybackDraft {
    index: Option<usize>,
    rule: PlaybackRule,
}

fn rule_detail(r: &PlaybackRule) -> String {
    let detail = join(&[
        r.shader
            .as_deref()
            .map_or(String::new(), |s| format!("Shader: {}", shaders::name(s))),
        r.quality
            .map_or(String::new(), |q| format!("Quality: {}", quality_label(q))),
    ]);
    if detail.is_empty() {
        "Uses your defaults".into()
    } else {
        detail
    }
}

impl Plaivo {
    pub fn render_playback_rules(&mut self, cx: &mut Context<Self>) -> Div {
        let rows: Vec<AnyElement> = self
            .settings
            .playback_rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                row(
                    ("playback-rule", i),
                    Some(Icon::Sort),
                    self.rule_targets(&r.libraries),
                    rule_detail(r),
                )
                .cursor_pointer()
                .hover(|s| s.bg(theme::tint(0.12)))
                .child(icon(Icon::ChevronRight, 22., theme::muted()))
                .on_click(cx.listener(move |this, _, _, cx| this.open_playback_rule(Some(i), cx)))
                .into_any_element()
            })
            .collect();
        section("Overrides").children(rows).child(
            nav_row(
                "playback-rule-add",
                Icon::Add,
                "Add override",
                "Pick libraries and the shader or quality to use",
            )
            .on_click(cx.listener(|this, _, _, cx| this.open_playback_rule(None, cx))),
        )
    }

    fn open_playback_rule(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        let rule = index
            .and_then(|i| self.settings.playback_rules.get(i).cloned())
            .unwrap_or_default();
        self.playback_draft = Some(PlaybackDraft { index, rule });
        self.navigate(Nav::Settings(Some(Category::PlaybackOverride)), true, cx);
    }

    fn edit_playback_rule(&mut self, f: impl FnOnce(&mut PlaybackDraft), cx: &mut Context<Self>) {
        if let Some(d) = &mut self.playback_draft {
            f(d);
        }
        cx.notify();
    }

    fn save_playback_rule(&mut self, delete: bool, cx: &mut Context<Self>) {
        let Some(d) = self.playback_draft.take() else {
            return;
        };
        let rules = &mut self.settings.playback_rules;
        match (d.index, delete) {
            (Some(i), true) if i < rules.len() => {
                rules.remove(i);
            }
            (Some(i), false) if i < rules.len() => rules[i] = d.rule,
            (None, false) => rules.push(d.rule),
            _ => {}
        }
        self.save_settings_soon(cx);
        self.back(cx);
    }

    pub fn playback_rule_actions(&self, cx: &mut Context<Self>) -> Option<Div> {
        let d = self.playback_draft.as_ref()?;
        Some(rule_buttons(
            d.index.is_some(),
            !d.rule.libraries.is_empty(),
            Self::save_playback_rule,
            cx,
        ))
    }

    pub fn render_playback_override(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.playback_draft.as_ref() else {
            return div().child(note("Open an override from Playback to edit it."));
        };
        let r = &d.rule;
        let libraries = self.library_card(
            &r.libraries,
            |this, id, cx| this.edit_playback_rule(|d| toggle(&mut d.rule.libraries, &id), cx),
            cx,
        );
        let label = |choices: Vec<Choice>| {
            choices
                .into_iter()
                .find(|c| c.selected)
                .map_or(SharedString::default(), |c| c.label)
        };
        let shader = self.picker(
            "rule-shader",
            card("Shader", "NVScaler turns itself off for HDR video"),
            label(self.rule_choices(Pick::RuleShader)),
            Pick::RuleShader,
            cx,
        );
        let quality = self.picker(
            "rule-quality",
            card("Quality", "unless the file is already smaller"),
            label(self.rule_choices(Pick::RuleQuality)),
            Pick::RuleQuality,
            cx,
        );
        rule_columns(libraries, column("Playback").child(shader).child(quality))
    }

    pub fn rule_choices(&self, p: Pick) -> Vec<Choice> {
        let Some(r) = self.playback_draft.as_ref().map(|d| &d.rule) else {
            return Vec::new();
        };
        let choice = |id: String, label: String, selected: bool| Choice {
            id: id.into(),
            label: label.into(),
            sub: SharedString::default(),
            selected,
        };
        if p == Pick::RuleShader {
            let current = r
                .shader
                .as_deref()
                .map(|id| shaders::find(id).map_or("", |p| p.id));
            let default = format!("Default · {}", shaders::name(&self.settings.shader));
            return std::iter::once(choice(RULE_DEFAULT.into(), default, current.is_none()))
                .chain(std::iter::once(choice(
                    String::new(),
                    "Off".into(),
                    current == Some(""),
                )))
                .chain(
                    shaders::PRESETS
                        .iter()
                        .map(|x| choice(x.id.into(), x.name.into(), current == Some(x.id))),
                )
                .collect();
        }
        let default = format!("Default · {}", quality_label(self.settings.quality));
        std::iter::once(choice(RULE_DEFAULT.into(), default, r.quality.is_none()))
            .chain(std::iter::once(choice(
                "0".into(),
                "Original".into(),
                r.quality == Some(0),
            )))
            .chain(QUALITIES.iter().map(|q| {
                choice(
                    q.kbps.to_string(),
                    quality_label(q.kbps),
                    r.quality == Some(q.kbps),
                )
            }))
            .collect()
    }

    pub fn pick_rule(&mut self, p: Pick, id: &str, cx: &mut Context<Self>) {
        let id = (id != RULE_DEFAULT).then(|| id.to_owned());
        self.edit_playback_rule(
            |d| match p {
                Pick::RuleShader => d.rule.shader = id,
                _ => d.rule.quality = id.map(|kbps| kbps.parse().unwrap_or(0)),
            },
            cx,
        )
    }

    fn picker(
        &self,
        key: &'static str,
        card: Div,
        label: SharedString,
        pick: Pick,
        cx: &mut Context<Self>,
    ) -> Div {
        let header = div()
            .id(key)
            .h(px(40.))
            .px(px(14.))
            .rounded(px(8.))
            .bg(theme::tint(0.06))
            .flex()
            .items_center()
            .cursor_pointer()
            .hover(|s| s.bg(theme::tint(0.1)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label),
            )
            .child(icon(Icon::ChevronRight, 20., theme::muted()))
            .on_click(cx.listener(move |this, _, _, cx| this.open_picker(pick, cx)));
        card.child(header)
    }
}
