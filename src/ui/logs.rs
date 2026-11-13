use crate::app::Plaivo;
use crate::logging;
use crate::ui::licenses::header;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::pill;
use gpui::{
    AnyElement, ClipboardItem, Context, HighlightStyle, Hsla, MouseButton, MouseDownEvent,
    MouseMoveEvent, StyledText, div, hsla, prelude::*, px, uniform_list,
};
use std::ops::Range;

const MONO: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "Consolas"
};

impl Plaivo {
    pub fn render_logs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let count = logging::len();
        let list = uniform_list(
            "logs",
            count,
            cx.processor(|this, range: Range<usize>, _, cx| {
                let selected = this.log_selection.map(|(a, b)| a.min(b)..=a.max(b));
                let start = range.start;
                logging::lines(range)
                    .into_iter()
                    .enumerate()
                    .map(|(n, line)| {
                        let i = start + n;
                        let spans = spans(&line);
                        div()
                            .px(px(32.))
                            .py(px(2.))
                            .when(selected.as_ref().is_some_and(|s| s.contains(&i)), |d| {
                                d.bg(theme::tint(0.14))
                            })
                            .font_family(MONO)
                            .text_size(px(12.))
                            .text_color(theme::text())
                            .whitespace_nowrap()
                            .truncate()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                                    this.log_selection = Some(match this.log_selection {
                                        Some((a, _)) if ev.modifiers.shift => (a, i),
                                        _ => (i, i),
                                    });
                                    cx.notify();
                                }),
                            )
                            .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                                if ev.dragging()
                                    && let Some((a, b)) = this.log_selection
                                    && b != i
                                {
                                    this.log_selection = Some((a, i));
                                    cx.notify();
                                }
                            }))
                            .child(StyledText::new(line).with_highlights(spans))
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.logs_scroll)
        .flex_1();
        let detail = match self.log_selection {
            Some((a, b)) => format!("{} of {count} lines selected", a.abs_diff(b) + 1),
            None => format!("{count} recent lines · drag or Shift+click to select"),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div().px(px(32.)).pt(px(32.)).pb(px(20.)).child(
                    header("Logs", detail.into(), cx)
                        .child(div().flex_1())
                        .child(
                            pill("copy-logs", "Copy", Some(Icon::Copy), false)
                                .on_click(cx.listener(|this, _, _, cx| this.copy_logs(cx))),
                        )
                        .child(
                            pill("export-logs", "Export", Some(Icon::OpenInNew), false).on_click(
                                cx.listener(|_, _, _, cx| {
                                    Self::bg(cx, logging::export, |this, r, cx| match r {
                                        Ok(path) => cx.reveal_path(&path),
                                        Err(e) => {
                                            this.toast(format!("Couldn't save the log: {e}"), cx)
                                        }
                                    })
                                }),
                            ),
                        ),
                ),
            )
            .child(list)
            .into_any_element()
    }

    pub fn copy_logs(&mut self, cx: &mut Context<Self>) {
        let range = self
            .log_selection
            .map_or(0..usize::MAX, |(a, b)| a.min(b)..a.max(b) + 1);
        let lines = logging::lines(range);
        cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
        self.toast(format!("Copied {} lines", lines.len()), cx);
    }
}

fn level_color(level: &str) -> Option<Hsla> {
    let hue = match level {
        "ERROR" => 0.,
        "WARN" => 38.,
        "INFO" => 130.,
        "DEBUG" => 195.,
        _ => return Some(theme::muted()),
    };
    let dark = Hsla::from(theme::text()).l > 0.5;
    Some(hsla(hue / 360., 0.7, if dark { 0.7 } else { 0.36 }, 1.))
}

fn spans(line: &str) -> Vec<(Range<usize>, HighlightStyle)> {
    let muted = HighlightStyle {
        color: Some(theme::muted()),
        ..Default::default()
    };
    let Some(level) = line.get(14..19) else {
        return Vec::new();
    };
    let mut out = vec![
        (0..13, muted),
        (
            14..14 + level.trim_end().len(),
            HighlightStyle {
                color: level_color(level.trim_end()),
                ..Default::default()
            },
        ),
    ];
    if let Some(end) = line.get(20..).and_then(|rest| rest.find(": ")) {
        out.push((20..21 + end, muted));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::spans;

    #[test]
    fn spans_cover_clock_level_and_target() {
        let line = "05:10:56.567Z WARN  plaivo::update: check failed: 404";
        let ranges: Vec<_> = spans(line).into_iter().map(|(r, _)| r).collect();
        assert_eq!(
            ranges.iter().map(|r| &line[r.clone()]).collect::<Vec<_>>(),
            ["05:10:56.567Z", "WARN", "plaivo::update:"]
        );
        assert!(spans("short").is_empty());
    }
}
