use crate::app::{Plaivo, open_url};
use crate::ui::player::spinner;
use crate::ui::settings::toggle_row;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{pill, row, section};
use crate::update::{Status, VERSION, notes_url, version_of};
use gpui::{Context, Div, SharedString, div, prelude::*, px};

impl Plaivo {
    pub fn render_updates(&mut self, cx: &mut Context<Self>) -> Div {
        let u = &self.updates;
        let (title, detail): (SharedString, SharedString) = match &u.status {
            Status::Unavailable(why) => ("Updates".into(), why.clone()),
            Status::Idle => (
                format!("Plaivo {VERSION}").into(),
                "Checks for updates once an hour".into(),
            ),
            Status::Checking => (
                "Checking for updates…".into(),
                format!("Plaivo {VERSION}").into(),
            ),
            Status::Current => (
                "Plaivo is up to date".into(),
                format!("Version {VERSION}").into(),
            ),
            Status::Downloading(r, pct) => (
                format!("Downloading Plaivo {}", version_of(r)).into(),
                format!("{pct}%").into(),
            ),
            Status::Ready(r) => (
                format!("Plaivo {} is ready", version_of(r)).into(),
                "Restart Plaivo to install it, or it installs the next time Plaivo starts".into(),
            ),
            Status::Installing => (
                "Starting the update…".into(),
                "Plaivo closes and reopens".into(),
            ),
            Status::Failed(e) => ("Couldn't update Plaivo".into(), e.clone()),
        };
        let notes = match &u.status {
            Status::Ready(r) => Some(notes_url(r)),
            _ => None,
        };
        let action = match &u.status {
            Status::Ready(..) => Some(
                pill("restart-to-update", "Restart to update", None, true)
                    .on_click(cx.listener(|this, _, _, cx| this.restart_to_update(cx)))
                    .into_any_element(),
            ),
            Status::Checking | Status::Downloading(..) | Status::Installing => {
                Some(spinner(20., theme::muted()).into_any_element())
            }
            Status::Idle | Status::Current | Status::Failed(_) => Some(
                pill("check-updates-now", "Check for updates", None, false)
                    .on_click(cx.listener(|this, _, _, cx| this.check_for_updates_now(cx)))
                    .into_any_element(),
            ),
            Status::Unavailable(_) => None,
        };
        section("Updates")
            .child(
                row("update-status", Some(Icon::Download), title, detail).child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .children(notes.map(|url| {
                            pill("update-notes", "What's new", Some(Icon::OpenInNew), false)
                                .on_click(move |_, _, _| open_url(&url))
                        }))
                        .children(action),
                ),
            )
            .when(!matches!(u.status, Status::Unavailable(_)), |d| {
                d.child(
                    toggle_row(
                        "check-updates",
                        "Check for updates automatically",
                        "Checks hourly and downloads updates in the background",
                        self.settings.check_updates,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.check_updates = !this.settings.check_updates;
                        this.save_settings_soon(cx);
                        if this.settings.check_updates {
                            this.check_for_updates(cx);
                        }
                        cx.notify();
                    })),
                )
            })
    }
}
