use crate::menu::{Hide, HideOthers, Minimize, OpenSettings, Quit, ToggleFullScreen};
use crate::ui::input::{self, *};
use gpui::{App, KeyBinding, actions};

actions!(plaivo, [Cancel]);

actions!(
    player,
    [
        TogglePause,
        SeekBack,
        SeekForward,
        SeekBackLarge,
        SeekForwardLarge,
        VolumeUp,
        VolumeDown,
        ToggleMute,
        SkipMarker,
        NextItem,
        PreviousItem
    ]
);

pub const PLAYER: &str = "Player";
pub const LOGS: &str = "Logs";

const MAC: bool = cfg!(target_os = "macos");

macro_rules! secondary {
    ($key:literal) => {
        if MAC {
            concat!("cmd-", $key)
        } else {
            concat!("ctrl-", $key)
        }
    };
}

pub fn install(cx: &mut App) {
    cx.bind_keys(defaults());
}

fn defaults() -> Vec<KeyBinding> {
    let player = Some(PLAYER);
    let logs = Some(LOGS);
    let text = Some(input::CONTEXT);
    let mut keys = vec![
        KeyBinding::new("escape", Cancel, None),
        KeyBinding::new("f11", ToggleFullScreen, None),
        KeyBinding::new("k", TogglePause, player),
        KeyBinding::new("left", SeekBack, player),
        KeyBinding::new("right", SeekForward, player),
        KeyBinding::new("shift-left", SeekBackLarge, player),
        KeyBinding::new("shift-right", SeekForwardLarge, player),
        KeyBinding::new("up", VolumeUp, player),
        KeyBinding::new("down", VolumeDown, player),
        KeyBinding::new("m", ToggleMute, player),
        KeyBinding::new("f", ToggleFullScreen, player),
        KeyBinding::new("s", SkipMarker, player),
        KeyBinding::new("n", NextItem, player),
        KeyBinding::new("p", PreviousItem, player),
        KeyBinding::new(secondary!("c"), Copy, logs),
        KeyBinding::new(secondary!("a"), SelectAll, logs),
        KeyBinding::new("backspace", Backspace, text),
        KeyBinding::new("delete", Delete, text),
        KeyBinding::new("left", Left, text),
        KeyBinding::new("right", Right, text),
        KeyBinding::new("shift-left", SelectLeft, text),
        KeyBinding::new("shift-right", SelectRight, text),
        KeyBinding::new("home", Home, text),
        KeyBinding::new("end", End, text),
        KeyBinding::new("shift-home", SelectHome, text),
        KeyBinding::new("shift-end", SelectEnd, text),
        KeyBinding::new("enter", Submit, text),
        KeyBinding::new(secondary!("a"), SelectAll, text),
        KeyBinding::new(secondary!("c"), Copy, text),
        KeyBinding::new(secondary!("x"), Cut, text),
        KeyBinding::new(secondary!("v"), Paste, text),
        KeyBinding::new(secondary!("z"), Undo, text),
        KeyBinding::new(secondary!("shift-z"), Redo, text),
    ];
    if MAC {
        keys.extend([
            KeyBinding::new("cmd-,", OpenSettings, None),
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-left", Home, text),
            KeyBinding::new("cmd-right", End, text),
            KeyBinding::new("cmd-shift-left", SelectHome, text),
            KeyBinding::new("cmd-shift-right", SelectEnd, text),
            KeyBinding::new("cmd-backspace", DeleteToStart, text),
            KeyBinding::new("alt-left", WordLeft, text),
            KeyBinding::new("alt-right", WordRight, text),
            KeyBinding::new("alt-shift-left", SelectWordLeft, text),
            KeyBinding::new("alt-shift-right", SelectWordRight, text),
            KeyBinding::new("alt-backspace", DeleteWordLeft, text),
            KeyBinding::new("alt-delete", DeleteWordRight, text),
        ]);
    } else {
        keys.extend([
            KeyBinding::new("ctrl-y", Redo, text),
            KeyBinding::new("ctrl-left", WordLeft, text),
            KeyBinding::new("ctrl-right", WordRight, text),
            KeyBinding::new("ctrl-shift-left", SelectWordLeft, text),
            KeyBinding::new("ctrl-shift-right", SelectWordRight, text),
            KeyBinding::new("ctrl-backspace", DeleteWordLeft, text),
            KeyBinding::new("ctrl-delete", DeleteWordRight, text),
        ]);
    }
    keys
}

#[cfg(test)]
mod tests {
    #[test]
    fn defaults_parse_without_shadowing() {
        let mut seen = std::collections::HashSet::new();
        for b in super::defaults() {
            let key = format!("{:?} {:?}", b.keystrokes(), b.predicate());
            assert!(seen.insert(key.clone()), "bound twice: {key}");
        }
    }
}
