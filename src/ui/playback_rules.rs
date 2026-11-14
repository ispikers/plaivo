use crate::app::{Category, Nav, Plaivo, join};
use crate::plex::QUALITIES;
use crate::settings::PlaybackRule;
use crate::shaders;
use crate::ui::language_rules::{
    ADD_LIST_H, card, check_row, column, rule_buttons, rule_columns, toggle,
};
use crate::ui::settings::{note, quality_label};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, nav_row, row, scroll_box, section};
use gpui::{AnyElement, Context, Div, FontWeight, div, prelude::*, px};

pub struct PlaybackDraft {
    index: Option<usize>,
    rule: PlaybackRule,
    open: Option<&'static str>,
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
        self.playback_draft = Some(PlaybackDraft {
            index,
            rule,
            open: None,
        });
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
        let shader_options = [(
            None,
            format!("Default · {}", shaders::name(&self.settings.shader)),
        )]
        .into_iter()
        .chain(std::iter::once((Some(""), "Off".to_owned())))
        .chain(
            shaders::PRESETS
                .iter()
                .map(|p| (Some(p.id), p.name.to_owned())),
        );
        let shader = self.picker(
            "rule-shader",
            card("Shader", "NVScaler turns itself off for HDR video"),
            shader_options.collect(),
            r.shader
                .as_deref()
                .map(|id| shaders::find(id).map_or("", |p| p.id)),
            |r, id| r.shader = id.map(str::to_owned),
            cx,
        );
        let quality_options = [
            (
                None,
                format!("Default · {}", quality_label(self.settings.quality)),
            ),
            (Some(0), "Original".to_owned()),
        ]
        .into_iter()
        .chain(
            QUALITIES
                .iter()
                .map(|q| (Some(q.kbps), quality_label(q.kbps))),
        );
        let quality = self.picker(
            "rule-quality",
            card("Quality", "unless the file is already smaller"),
            quality_options.collect(),
            r.quality,
            |r, kbps| r.quality = kbps,
            cx,
        );
        rule_columns(libraries, column("Playback").child(shader).child(quality))
    }

    fn picker<T: PartialEq + Copy + 'static>(
        &self,
        key: &'static str,
        card: Div,
        options: Vec<(T, String)>,
        current: T,
        set: fn(&mut PlaybackRule, T),
        cx: &mut Context<Self>,
    ) -> Div {
        let open = self
            .playback_draft
            .as_ref()
            .is_some_and(|d| d.open == Some(key));
        let label = options
            .iter()
            .find(|(v, _)| *v == current)
            .map_or(String::new(), |(_, l)| l.clone());
        let header = div()
            .id((key, usize::MAX))
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
            .child(icon(
                if open {
                    Icon::ExpandLess
                } else {
                    Icon::ExpandMore
                },
                20.,
                theme::muted(),
            ))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.edit_playback_rule(
                    |d| d.open = if d.open == Some(key) { None } else { Some(key) },
                    cx,
                )
            }));
        let list = open.then(|| {
            scroll_box((key, usize::MAX - 1))
                .max_h(px(ADD_LIST_H))
                .rounded(px(8.))
                .border_1()
                .border_color(theme::outline())
                .p(px(4.))
                .flex()
                .flex_col()
                .children(options.into_iter().enumerate().map(|(i, (v, l))| {
                    check_row((key, i), l, v == current).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.edit_playback_rule(
                                |d| {
                                    set(&mut d.rule, v);
                                    d.open = None;
                                },
                                cx,
                            )
                        },
                    ))
                }))
        });
        card.child(header).children(list)
    }
}
