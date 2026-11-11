use gpui::{App, Bounds, DisplayId, Window, WindowBounds, point, px, size};
use plaivo_core::store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main window size and position, in GPUI's coordinates for `display` (a display UUID).
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Placement {
    display: String,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    maximized: bool,
}

fn path() -> PathBuf {
    store::config_dir().join("window.json")
}

pub fn load() -> Option<Placement> {
    store::read_json(&path())
}

pub fn save(p: &Placement) {
    store::write_json(&path(), p);
}

/// The window's current placement; `None` in full screen, so the last windowed placement is kept.
pub fn current(window: &Window, cx: &App) -> Option<Placement> {
    let (b, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(b) => (b, false),
        WindowBounds::Maximized(b) => (b, true),
        WindowBounds::Fullscreen(_) => return None,
    };
    Some(Placement {
        display: window.display(cx)?.uuid().ok()?.to_string(),
        x: b.origin.x.as_f32(),
        y: b.origin.y.as_f32(),
        width: b.size.width.as_f32(),
        height: b.size.height.as_f32(),
        maximized,
    })
}

/// Bounds to open the main window with: the saved placement if its display is still connected, else centered at the default size.
// shortcut: trusts saved bounds on a connected display even if its resolution shrank; clamp to the display if that bites.
pub fn restore(saved: Option<&Placement>, cx: &App) -> (WindowBounds, Option<DisplayId>) {
    let found = saved.and_then(|p| {
        let display = cx
            .displays()
            .into_iter()
            .find(|d| d.uuid().is_ok_and(|u| u.to_string() == p.display))?;
        let b = Bounds::new(point(px(p.x), px(p.y)), size(px(p.width), px(p.height)));
        let bounds = if p.maximized {
            WindowBounds::Maximized(b)
        } else {
            WindowBounds::Windowed(b)
        };
        Some((bounds, Some(display.id())))
    });
    found.unwrap_or_else(|| {
        let b = Bounds::centered(None, size(px(1440.), px(900.)), cx);
        (WindowBounds::Windowed(b), None)
    })
}
