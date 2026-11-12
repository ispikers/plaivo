use crate::app::{Nav, Plaivo, join};
use crate::ui::settings::{note, toggle_row};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{chip, icon, pill, row, section};
use gpui::{
    AnyElement, ClipboardItem, Context, Div, FontWeight, SharedString, Window, div, prelude::*, px,
};
use plaivo_core::watch_together::{Connection, Control, format_code, sync_label};

fn error_text(text: SharedString) -> Div {
    div()
        .text_size(px(13.))
        .text_color(theme::error())
        .child(text)
}

fn code_of(code: &str) -> String {
    format_code(code).unwrap_or_else(|| code.to_owned())
}

impl Plaivo {
    pub fn render_watch(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let w = &self.watch;
        let body = if w.in_room() {
            self.render_room(cx)
        } else if w.busy() {
            section("Room").child(
                row("watch-connecting", Some(Icon::Group), "Connecting…", "").child(
                    pill("watch-cancel", "Cancel", None, false)
                        .on_click(cx.listener(|this, _, _, cx| this.watch_leave(cx))),
                ),
            )
        } else {
            self.render_lobby(window, cx)
        };
        div()
            .id("watch")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .px(px(32.))
                    .py(px(32.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(
                        div()
                            .text_size(px(28.))
                            .font_weight(FontWeight::BOLD)
                            .child("Watch Together"),
                    )
                    .child(body),
            )
            .into_any_element()
    }

    fn render_lobby(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let code = self.watch.code.clone();
        let join_typed = cx.listener(move |this, _, _, cx| {
            let text = this.watch.code.read(cx).text().to_owned();
            this.watch_join(text, cx)
        });
        let recent = self.settings.watch_rooms.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                section("Start a room").child(
                    row(
                        "watch-create",
                        Some(Icon::Group),
                        "Create a room",
                        "Get a code to share with friends",
                    )
                    .child(
                        pill("watch-create-button", "Create", None, true)
                            .on_click(cx.listener(|this, _, _, cx| this.watch_create(cx))),
                    ),
                ),
            )
            .child(
                section("Join a room")
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(div().flex_1().child(self.field(
                                "watch-code",
                                &code,
                                window,
                                cx,
                            )))
                            .child(pill("watch-join", "Join", None, false).on_click(join_typed)),
                    )
                    .when_some(self.watch.error.clone(), |d, e| d.child(error_text(e))),
            )
            .when(!recent.is_empty(), |d| {
                d.child(
                    section("Recent rooms").children(recent.into_iter().enumerate().map(
                        |(i, c)| {
                            let label = code_of(&c);
                            row(("watch-recent", i), Some(Icon::Group), label, "").child(
                                pill(("watch-recent-join", i), "Join", None, false).on_click(
                                    cx.listener(move |this, _, _, cx| {
                                        this.watch_join(c.clone(), cx)
                                    }),
                                ),
                            )
                        },
                    )),
                )
            })
    }

    fn render_room(&mut self, cx: &mut Context<Self>) -> Div {
        let s = &self.watch.state;
        let Some(room) = s.room.clone() else {
            return div();
        };
        let me = s.peer_id.clone().unwrap_or_default();
        let host = s.is_host();
        let code = code_of(&room.code);
        let status = match (s.connection, host) {
            (Connection::Reconnecting, _) => "Reconnecting…".to_owned(),
            (_, true) => "You're the host. Share this code to invite others.".to_owned(),
            _ => room
                .participants
                .iter()
                .find(|p| p.host)
                .map_or(String::new(), |p| format!("Hosted by {}", p.name)),
        };
        let playing = s.media.as_ref().map(|m| match &m.series {
            Some(series) => join(&[
                series.title.clone(),
                m.season
                    .zip(m.episode)
                    .map(|(s, e)| format!("S{s} · E{e}"))
                    .unwrap_or_default(),
                m.title.clone(),
            ]),
            None => m.title.clone(),
        });
        let mine = if self.watch.unavailable {
            "Not in your libraries"
        } else {
            sync_label(s.player, s.match_, true)
        };
        let copy = code.clone();
        let participants = room.participants.iter().map(|p| {
            let you = p.peer_id == me;
            let title = if you {
                format!("{} (you)", p.name)
            } else {
                p.name.clone()
            };
            let app = p.client.split('/').next().unwrap_or_default().to_owned();
            let detail = join(&[
                if p.host { "Host".into() } else { String::new() },
                app,
                sync_label(p.player, p.match_, p.connected).into(),
            ]);
            let (transfer, kick) = (p.peer_id.clone(), p.peer_id.clone());
            let id = SharedString::from(format!("watch-peer/{}", p.peer_id));
            row((id.clone(), 0), Some(Icon::Person), title, detail).when(host && !you, |r| {
                r.child(
                    pill((id.clone(), 1), "Make host", None, false).on_click(cx.listener(
                        move |this, _, _, _| this.watch_call(|wt| wt.transfer_host(&transfer)),
                    )),
                )
                .child(pill((id, 2), "Remove", None, false).on_click(
                    cx.listener(move |this, _, _, _| this.watch_call(|wt| wt.kick(&kick))),
                ))
            })
        });
        let everyone = room.control == Control::Everyone;
        let locked = room.locked;
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                section("Room")
                    .child(
                        row("watch-room", Some(Icon::Group), code, status)
                            .child(
                                pill("watch-copy", "Copy code", Some(Icon::Copy), false).on_click(
                                    move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy.clone(),
                                        ))
                                    },
                                ),
                            )
                            .child(
                                pill("watch-leave", "Leave", Some(Icon::Logout), false)
                                    .on_click(cx.listener(|this, _, _, cx| this.watch_leave(cx))),
                            ),
                    )
                    .child(
                        row(
                            "watch-playing",
                            Some(Icon::Movie),
                            playing.unwrap_or_else(|| "Nothing playing yet".into()),
                            if s.media.is_some() { mine } else { "" },
                        )
                        .when(self.watch.can_open(), |r| {
                            r.child(
                                pill("watch-open", "Open party", Some(Icon::Play), true)
                                    .on_click(cx.listener(|this, _, _, cx| this.watch_open(cx))),
                            )
                        }),
                    )
                    .when(host && s.media.is_none(), |d| {
                        d.child(note(
                            "Open a movie or episode and choose Watch together to start.",
                        ))
                    }),
            )
            .child(
                section(format!("Participants · {}", room.participants.len()))
                    .children(participants),
            )
            .when(host, |d| {
                d.child(
                    section("Host controls")
                        .child(
                            toggle_row(
                                "watch-everyone",
                                "Everyone can control playback",
                                "Anyone can pause, play and skip. Otherwise only you can.",
                                everyone,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, _| {
                                    this.watch_call(|wt| {
                                        wt.set_control(if everyone {
                                            Control::Host
                                        } else {
                                            Control::Everyone
                                        })
                                    })
                                },
                            )),
                        )
                        .child(
                            toggle_row("watch-lock", "Lock room", "No one else can join.", locked)
                                .on_click(cx.listener(move |this, _, _, _| {
                                    this.watch_call(|wt| wt.lock(!locked))
                                })),
                        )
                        .child(
                            row(
                                "watch-end",
                                None,
                                "End room",
                                "Closes the room for everyone.",
                            )
                            .child(
                                pill("watch-end-button", "End", None, false).on_click(
                                    cx.listener(|this, _, _, _| this.watch_call(|wt| wt.end())),
                                ),
                            ),
                        ),
                )
            })
    }

    pub fn render_watch_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let s = &self.watch.state;
        let room = s.room.as_ref().filter(|_| self.watch.in_room())?;
        let state = match s.connection {
            Connection::Reconnecting => "Reconnecting",
            _ if self.watch.unavailable => "Not in your libraries",
            _ => sync_label(s.player, s.match_, true),
        };
        Some(
            div()
                .id("watch-pill")
                .flex_none()
                .h(px(32.))
                .px(px(12.))
                .rounded_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .bg(theme::black(0.45))
                .hover(|d| d.bg(theme::black(0.65)))
                .cursor_pointer()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::white())
                .child(icon(Icon::Group, 18., theme::white()))
                .child(join(&[
                    code_of(&room.code),
                    room.participants.len().to_string(),
                    state.into(),
                ]))
                .on_click(cx.listener(|this, _, _, cx| this.open_from_player(Nav::Watch, cx)))
                .into_any_element(),
        )
    }

    pub fn watch_summary(&self) -> String {
        let name = if self.settings.watch_name.is_empty() {
            "your account name".to_owned()
        } else {
            self.settings.watch_name.clone()
        };
        format!(
            "Name: {name} · Relay: {}",
            if self.settings.watch_relay.is_empty() {
                "Public"
            } else {
                "Custom"
            }
        )
    }

    pub fn render_watch_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let (name, relay, custom) = (
            self.watch.name.clone(),
            self.watch.relay.clone(),
            self.watch.custom_relay,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                section("Display name")
                    .child(note("Leave empty to use your Plex or Jellyfin name."))
                    .child(self.field("watch-name", &name, window, cx)),
            )
            .child(
                section("Relay")
                    .child(note("Everyone in a room must use the same relay."))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(chip("watch-relay-public", "Public", !custom).on_click(
                                cx.listener(|this, _, _, cx| this.use_public_relay(true, cx)),
                            ))
                            .child(chip("watch-relay-custom", "Custom", custom).on_click(
                                cx.listener(|this, _, _, cx| this.use_public_relay(false, cx)),
                            )),
                    )
                    .when(custom, |d| {
                        d.child(self.field("watch-relay", &relay, window, cx))
                            .when_some(self.watch.relay_error.clone(), |d, e| {
                                d.child(error_text(e))
                            })
                    }),
            )
            .child(note(
                "Changes apply the next time you create or join a room.",
            ))
    }
}
