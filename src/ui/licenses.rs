use crate::app::{Nav, Plaivo};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, round_button, row};
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, div, prelude::*, px, uniform_list};

include!(concat!(env!("OUT_DIR"), "/licenses.rs"));

impl Plaivo {
    pub fn render_licenses(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some((i, l)) = self.license.and_then(|i| Some((i, LICENSES.get(i)?))) {
            return div()
                .id(("license-text", i))
                .size_full()
                .overflow_y_scroll()
                .child(
                    div()
                        .w_full()
                        .px(px(32.))
                        .py(px(32.))
                        .flex()
                        .flex_col()
                        .gap(px(20.))
                        .child(header(l.name, subtitle(l), cx))
                        .children(l.parts().map(|text| div().p(px(20.)).rounded(px(12.)).bg(theme::surface()).text_size(px(13.)).text_color(theme::text()).child(text))),
                )
                .into_any_element();
        }
        let list = uniform_list(
            "licenses",
            LICENSES.len(),
            cx.processor(|_, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|i| {
                        let l = &LICENSES[i];
                        div().px(px(32.)).pb(px(8.)).child(
                            row(("license", i), None, l.name, subtitle(l))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme::tint(0.12)))
                                .child(icon(Icon::ChevronRight, 22., theme::muted()))
                                .on_click(cx.listener(move |this, _, _, cx| this.navigate(Nav::Licenses(Some(i)), true, cx))),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.licenses_scroll)
        .flex_1();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().px(px(32.)).pt(px(32.)).pb(px(20.)).child(header("Open-source licenses", format!("{} components", LICENSES.len()).into(), cx)))
            .child(list)
            .into_any_element()
    }
}

fn header(title: &'static str, detail: SharedString, cx: &mut Context<Plaivo>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(round_button("licenses-back", Icon::Back, 40., 22., theme::black(0.), theme::tint(0.08), theme::text()).on_click(cx.listener(|this, _, _, cx| this.back(cx))))
        .child(
            div()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(px(28.)).font_weight(FontWeight::BOLD).truncate().child(title))
                .child(div().text_size(px(13.)).text_color(theme::muted()).child(detail)),
        )
}

fn subtitle(l: &License) -> SharedString {
    if l.version.is_empty() { l.license.into() } else { format!("{} · {}", l.version, l.license).into() }
}

#[cfg(test)]
mod tests {
    use super::LICENSES;

    #[test]
    fn license_list_matches_cargo_tree() {
        super::assert_matches_cargo_tree("plaivo");
    }

    #[test]
    fn manifest_entries_lead_the_list_with_their_texts() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../licenses/desktop.json")).unwrap();
        let entries = manifest.as_array().unwrap();
        assert!(entries.len() > 50);
        for (e, l) in entries.iter().zip(&LICENSES) {
            assert_eq!(e["name"], l.name);
            assert_eq!(e["license"], l.license);
            assert_eq!(l.parts().count(), e["files"].as_array().unwrap().len(), "{}", l.name);
        }
        let text = |i: usize| LICENSES[i].parts().collect::<String>();
        assert!(LICENSES[0].name == "libmpv" && text(0).contains("Version 2.1, February 1999"));
        assert!(LICENSES[1].name == "FFmpeg" && text(1).contains("Version 3, 29 June 2007"));
        assert!(LICENSES.iter().find(|l| l.name == "FriBidi").unwrap().parts().any(|t| t.contains("Sharif FarsiWeb")));
    }
}
