use crate::ui::theme;
use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point,
    ShapedLine, SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div,
    fill, point, prelude::*, px, relative, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        SelectHome,
        SelectEnd,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteWordRight,
        DeleteToStart,
        Paste,
        Cut,
        Copy,
        Submit,
        Undo,
        Redo
    ]
);

const CONTEXT: &str = "TextInput";
const MAC: bool = cfg!(target_os = "macos");
const CARET: f32 = 2.;

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(CONTEXT);
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, ctx),
        KeyBinding::new("delete", Delete, ctx),
        KeyBinding::new("left", Left, ctx),
        KeyBinding::new("right", Right, ctx),
        KeyBinding::new("shift-left", SelectLeft, ctx),
        KeyBinding::new("shift-right", SelectRight, ctx),
        KeyBinding::new(if MAC { "cmd-a" } else { "ctrl-a" }, SelectAll, ctx),
        KeyBinding::new(if MAC { "cmd-v" } else { "ctrl-v" }, Paste, ctx),
        KeyBinding::new(if MAC { "cmd-c" } else { "ctrl-c" }, Copy, ctx),
        KeyBinding::new(if MAC { "cmd-x" } else { "ctrl-x" }, Cut, ctx),
        KeyBinding::new("home", Home, ctx),
        KeyBinding::new("end", End, ctx),
        KeyBinding::new("enter", Submit, ctx),
        KeyBinding::new(if MAC { "cmd-z" } else { "ctrl-z" }, Undo, ctx),
        KeyBinding::new(if MAC { "cmd-shift-z" } else { "ctrl-shift-z" }, Redo, ctx),
    ]);
    cx.bind_keys([
        KeyBinding::new("shift-home", SelectHome, ctx),
        KeyBinding::new("shift-end", SelectEnd, ctx),
    ]);
    if MAC {
        cx.bind_keys([
            KeyBinding::new("cmd-left", Home, ctx),
            KeyBinding::new("cmd-right", End, ctx),
            KeyBinding::new("cmd-shift-left", SelectHome, ctx),
            KeyBinding::new("cmd-shift-right", SelectEnd, ctx),
            KeyBinding::new("cmd-backspace", DeleteToStart, ctx),
            KeyBinding::new("alt-left", WordLeft, ctx),
            KeyBinding::new("alt-right", WordRight, ctx),
            KeyBinding::new("alt-shift-left", SelectWordLeft, ctx),
            KeyBinding::new("alt-shift-right", SelectWordRight, ctx),
            KeyBinding::new("alt-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("alt-delete", DeleteWordRight, ctx),
        ]);
    } else {
        cx.bind_keys([
            KeyBinding::new("ctrl-y", Redo, ctx),
            KeyBinding::new("ctrl-left", WordLeft, ctx),
            KeyBinding::new("ctrl-right", WordRight, ctx),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, ctx),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, ctx),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, ctx),
            KeyBinding::new("ctrl-delete", DeleteWordRight, ctx),
        ]);
    }
}

pub enum InputEvent {
    Changed(SharedString),
    Submit(SharedString),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EditKind {
    Typing,
    Deleting,
    Other,
}

#[derive(Clone, PartialEq, Debug)]
struct Snapshot {
    text: SharedString,
    selection: Range<usize>,
}

#[derive(Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    last: Option<(EditKind, usize)>,
}

impl History {
    const LIMIT: usize = 100;

    // Typing or deleting that carries on from where the last such edit left the caret is one undo step.
    fn record(
        &mut self,
        before: Snapshot,
        kind: EditKind,
        range: &Range<usize>,
        caret_after: usize,
    ) {
        let continues = match (kind, self.last) {
            (EditKind::Typing, Some((EditKind::Typing, caret))) => range.start == caret,
            (EditKind::Deleting, Some((EditKind::Deleting, caret))) => {
                range.end == caret || range.start == caret
            }
            _ => false,
        };
        if !continues {
            self.undo.push(before);
            if self.undo.len() > Self::LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.last = (kind != EditKind::Other).then_some((kind, caret_after));
    }
}

pub struct TextInput {
    pub focus: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    scroll_x: Pixels,
    is_selecting: bool,
    masked: bool,
    history: History,
}

impl EventEmitter<InputEvent> for TextInput {}

impl TextInput {
    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: placeholder.to_owned().into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_x: px(0.),
            is_selecting: false,
            masked: false,
            history: History::default(),
        }
    }

    pub fn masked(mut self) -> Self {
        self.masked = true;
        self
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.content = text.to_owned().into();
        self.selected_range = text.len()..text.len();
        self.marked_range = None;
        self.history = History::default();
        cx.emit(InputEvent::Changed(self.content.clone()));
        cx.notify();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.content.clone(),
            selection: self.selected_range.clone(),
        }
    }

    fn restore(&mut self, to: Snapshot, cx: &mut Context<Self>) {
        self.content = to.text;
        self.selected_range = to.selection;
        self.selection_reversed = false;
        self.marked_range = None;
        self.history.last = None;
        self.changed(cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.history.undo.pop() {
            self.history.redo.push(self.snapshot());
            self.restore(previous, cx);
        }
    }

    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(next) = self.history.redo.pop() {
            self.history.undo.push(self.snapshot());
            self.restore(next, cx);
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Changed(self.content.clone()));
        cx.notify();
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(0, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.content.len(), cx);
    }

    fn word_left(&mut self, _: &WordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_start_before(self.cursor_offset()), cx);
    }

    fn word_right(&mut self, _: &WordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.word_end_after(self.cursor_offset()), cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.word_start_before(self.cursor_offset()), cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.word_end_after(self.cursor_offset()), cx);
    }

    fn delete_word_left(
        &mut self,
        _: &DeleteWordLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.word_start_before(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_word_right(
        &mut self,
        _: &DeleteWordRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.selected_range.is_empty() {
            self.select_to(self.word_end_after(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete_to_start(&mut self, _: &DeleteToStart, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(0, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn submit(&mut self, _: &Submit, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(InputEvent::Submit(self.content.clone()));
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus, cx);
        let index = self.index_for_mouse_position(event.position);
        if event.modifiers.shift {
            self.is_selecting = true;
            return self.select_to(index, cx);
        }
        let range = match event.click_count {
            0 | 1 => {
                self.is_selecting = true;
                return self.move_to(index, cx);
            }
            2 if !self.masked => word_at(&self.content, index),
            _ => 0..self.content.len(),
        };
        self.is_selecting = false;
        self.move_to(range.start, cx);
        self.select_to(range.end, cx);
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &single_line(&text), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.history.last = None;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        let i = line.closest_index_for_x(position.x - bounds.left() + self.scroll_x);
        (0..=i)
            .rev()
            .find(|&i| self.content.is_char_boundary(i))
            .unwrap_or(0)
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        offset_from_utf16(&self.content, offset)
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        offset_to_utf16(&self.content, offset)
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        previous_boundary(&self.content, offset)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        next_boundary(&self.content, offset)
    }

    // A masked field is one word, so word jumps don't reveal where a password has spaces.
    fn word_start_before(&self, offset: usize) -> usize {
        if self.masked {
            0
        } else {
            previous_word_start(&self.content, offset)
        }
    }

    fn word_end_after(&self, offset: usize) -> usize {
        if self.masked {
            self.content.len()
        } else {
            next_word_end(&self.content, offset)
        }
    }
}

fn offset_from_utf16(text: &str, offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in text.chars() {
        if utf16_count >= offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }
    utf8_offset
}

fn offset_to_utf16(text: &str, offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for ch in text.chars() {
        if utf8_count >= offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

fn previous_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(idx, _)| (idx < offset).then_some(idx))
        .unwrap_or(0)
}

fn next_boundary(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(idx, _)| (idx > offset).then_some(idx))
        .unwrap_or(text.len())
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

fn previous_word_start(text: &str, offset: usize) -> usize {
    text.split_word_bound_indices()
        .rev()
        .find(|&(i, w)| i < offset && is_word(w))
        .map_or(0, |(i, _)| i)
}

fn next_word_end(text: &str, offset: usize) -> usize {
    text.split_word_bound_indices()
        .map(|(i, w)| (i + w.len(), w))
        .find(|&(end, w)| end > offset && is_word(w))
        .map_or(text.len(), |(end, _)| end)
}

// The word under a double-click; at a word's end, the word rather than the space after it.
fn word_at(text: &str, offset: usize) -> Range<usize> {
    let segments: Vec<Range<usize>> = text
        .split_word_bound_indices()
        .map(|(i, w)| i..i + w.len())
        .collect();
    let here = segments.iter().find(|r| r.contains(&offset));
    let before = segments.iter().find(|r| r.end == offset);
    match (here, before) {
        (Some(h), Some(b)) if !is_word(&text[h.clone()]) && is_word(&text[b.clone()]) => b.clone(),
        (Some(h), _) => h.clone(),
        (None, Some(b)) => b.clone(),
        (None, None) => offset..offset,
    }
}

fn marked_selection(start: usize, new_text: &str, selected_utf16: &Range<usize>) -> Range<usize> {
    start + offset_from_utf16(new_text, selected_utf16.start)
        ..start + offset_from_utf16(new_text, selected_utf16.end)
}

fn single_line(text: &str) -> String {
    text.trim_end_matches(['\r', '\n'])
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
}

fn scroll_to_cursor(scroll: f32, cursor: f32, line_width: f32, view_width: f32) -> f32 {
    scroll
        .max(cursor + CARET - view_width)
        .min(cursor)
        .min(line_width + CARET - view_width)
        .max(0.)
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        // A composition was recorded when it started, so committing it isn't a step of its own.
        if self.marked_range.is_none() {
            let kind =
                if new_text.is_empty() && next_boundary(&self.content, range.start) == range.end {
                    EditKind::Deleting
                } else if range.is_empty() && new_text.chars().count() == 1 {
                    EditKind::Typing
                } else {
                    EditKind::Other
                };
            let before = self.snapshot();
            self.history
                .record(before, kind, &range, range.start + new_text.len());
        }
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        self.changed(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|r| self.range_from_utf16(r))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        if self.marked_range.is_none() {
            let before = self.snapshot();
            self.history.record(
                before,
                EditKind::Other,
                &range,
                range.start + new_text.len(),
            );
        }
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.marked_range =
            (!new_text.is_empty()).then(|| range.start..range.start + new_text.len());
        self.selected_range = new_selected_range_utf16
            .map(|r| marked_selection(range.start, new_text, &r))
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let left = bounds.left() - self.scroll_x;
        Some(Bounds::from_corners(
            point(left + last_layout.x_for_index(range.start), bounds.top()),
            point(left + last_layout.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index = last_layout.closest_index_for_x(line_point.x + self.scroll_x);
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll_x: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();
        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), theme::muted())
        } else if input.masked {
            // One ASCII mask byte per content byte keeps every layout index identical to a content index.
            ("*".repeat(content.len()).into(), style.color)
        } else {
            (content, style.color)
        };
        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = match input.marked_range.as_ref() {
            Some(marked) => vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);
        let cursor_pos = line.x_for_index(cursor);
        let scroll_x = px(scroll_to_cursor(
            input.scroll_x.into(),
            cursor_pos.into(),
            line.width().into(),
            bounds.size.width.into(),
        ));
        let left = bounds.left() - scroll_x;
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(left + cursor_pos, bounds.top()),
                        size(px(CARET), bounds.bottom() - bounds.top()),
                    ),
                    theme::text(),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(left + line.x_for_index(selected_range.start), bounds.top()),
                        point(left + line.x_for_index(selected_range.end), bounds.bottom()),
                    ),
                    theme::tint(0.25),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            scroll_x,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let line = prepaint.line.take().unwrap();
        let scroll_x = prepaint.scroll_x;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection)
            }
            line.paint(
                point(bounds.left() - scroll_x, bounds.top()),
                window.line_height(),
                gpui::TextAlign::Left,
                None,
                window,
                cx,
            )
            .unwrap();
            if focus.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
        });
        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
            input.scroll_x = scroll_x;
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_1()
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::delete_to_start))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .text_size(px(16.))
            .line_height(px(24.))
            .text_color(theme::text())
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ime_caret_is_relative_to_inserted_text() {
        assert_eq!(marked_selection(0, "か", &(1..1)), 3..3);
        assert_eq!(marked_selection(2, "か", &(1..1)), 5..5);
        assert_eq!(marked_selection(1, "é🙂", &(1..3)), 3..7);
        assert_eq!(marked_selection(1, "漢", &(5..9)), 4..4);
        assert_eq!(marked_selection(1, "", &(0..0)), 1..1);
    }

    #[test]
    fn utf16_offsets_round_trip_surrogate_pairs() {
        let text = "a🙂b";
        assert_eq!(offset_to_utf16(text, 1), 1);
        assert_eq!(offset_to_utf16(text, 5), 3);
        assert_eq!(offset_to_utf16(text, 6), 4);
        assert_eq!(offset_from_utf16(text, 3), 5);
        assert_eq!(offset_from_utf16(text, 4), 6);
        assert_eq!(offset_from_utf16(text, 99), 6);
    }

    #[test]
    fn boundaries_step_whole_graphemes() {
        let family = "a👨‍👩‍👧";
        assert_eq!(previous_boundary(family, family.len()), 1);
        assert_eq!(next_boundary(family, 1), family.len());
        let accent = "xe\u{301}";
        assert_eq!(previous_boundary(accent, accent.len()), 1);
        assert_eq!(next_boundary(accent, 1), accent.len());
    }

    #[test]
    fn word_jumps_skip_spaces_and_punctuation() {
        let text = "hello world,  foo";
        assert_eq!(previous_word_start(text, text.len()), 14);
        assert_eq!(previous_word_start(text, 14), 6);
        assert_eq!(previous_word_start(text, 8), 6);
        assert_eq!(previous_word_start(text, 3), 0);
        assert_eq!(previous_word_start(text, 0), 0);
        assert_eq!(next_word_end(text, 0), 5);
        assert_eq!(next_word_end(text, 5), 11);
        assert_eq!(next_word_end(text, 7), 11);
        assert_eq!(next_word_end(text, 11), text.len());
        assert_eq!(next_word_end(text, text.len()), text.len());
        assert_eq!(previous_word_start("   ", 3), 0);
        assert_eq!(next_word_end("   ", 0), 3);
        let url = "http://host:32400";
        assert_eq!(previous_word_start(url, url.len()), 12);
        assert_eq!(previous_word_start(url, 12), 7);
    }

    fn snap(text: &'static str, caret: usize) -> Snapshot {
        Snapshot {
            text: text.into(),
            selection: caret..caret,
        }
    }

    #[test]
    fn typing_and_deleting_coalesce_into_single_steps() {
        let mut h = History::default();
        h.record(snap("", 0), EditKind::Typing, &(0..0), 1);
        h.record(snap("a", 1), EditKind::Typing, &(1..1), 2);
        h.record(snap("ab", 2), EditKind::Typing, &(2..2), 3);
        assert_eq!(h.undo, vec![snap("", 0)]);
        h.record(snap("abc", 3), EditKind::Deleting, &(2..3), 2);
        h.record(snap("ab", 2), EditKind::Deleting, &(1..2), 1);
        assert_eq!(h.undo, vec![snap("", 0), snap("abc", 3)]);
        h.record(snap("a", 1), EditKind::Other, &(0..1), 5);
        h.record(snap("hello", 5), EditKind::Other, &(0..5), 0);
        assert_eq!(h.undo.len(), 4);
        h.redo.push(snap("x", 1));
        h.record(snap("", 0), EditKind::Typing, &(0..0), 1);
        assert!(h.redo.is_empty());
    }

    #[test]
    fn typing_elsewhere_starts_a_new_step() {
        let mut h = History::default();
        h.record(snap("", 0), EditKind::Typing, &(0..0), 1);
        h.record(snap("a", 1), EditKind::Typing, &(0..0), 1);
        assert_eq!(h.undo.len(), 2);
    }

    #[test]
    fn history_is_capped() {
        let mut h = History::default();
        for i in 0..History::LIMIT + 5 {
            h.record(snap("", i), EditKind::Other, &(0..0), 0);
        }
        assert_eq!(h.undo.len(), History::LIMIT);
        assert_eq!(h.undo[0], snap("", 5));
    }

    #[test]
    fn double_click_picks_the_word() {
        let text = "hello world, foo";
        assert_eq!(word_at(text, 1), 0..5);
        assert_eq!(word_at(text, 5), 0..5);
        assert_eq!(word_at(text, 6), 6..11);
        assert_eq!(word_at(text, 11), 6..11);
        assert_eq!(word_at(text, text.len()), 13..16);
        assert_eq!(word_at("a  b", 2), 1..3);
        assert_eq!(word_at("", 0), 0..0);
    }

    #[test]
    fn paste_flattens_to_one_line() {
        assert_eq!(single_line("http://host:32400\r\n"), "http://host:32400");
        assert_eq!(single_line("a\r\nb\nc\rd"), "a b c d");
        assert_eq!(single_line(" pass "), " pass ");
    }

    #[test]
    fn scroll_keeps_cursor_visible() {
        assert_eq!(scroll_to_cursor(0., 50., 100., 200.), 0.);
        assert_eq!(scroll_to_cursor(0., 500., 500., 200.), 302.);
        assert_eq!(scroll_to_cursor(302., 0., 500., 200.), 0.);
        assert_eq!(scroll_to_cursor(302., 400., 500., 200.), 302.);
        assert_eq!(scroll_to_cursor(302., 100., 100., 200.), 0.);
        assert_eq!(scroll_to_cursor(0., 0., 0., 0.), 0.);
    }
}
