use crate::app::{Nav, Page, Plaivo, join, poll_until};
use crate::plex::{R, Server};
use crate::ui::input::{InputEvent, TextInput};
use crate::ui::settings::note;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{icon, nav_row, pill, round_button, row, section};
use gpui::{
    AnyElement, AppContext, Context, Div, Entity, FocusHandle, FontWeight, SharedString, Window,
    div, prelude::*, px,
};
use plaivo_core::jellyfin::{self, ServerInfo};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const QUICK_CONNECT_POLL: Duration = Duration::from_secs(2);
const QUICK_CONNECT_FOR: Duration = Duration::from_secs(600);

pub struct AddServer {
    address: Entity<TextInput>,
    username: Entity<TextInput>,
    password: Entity<TextInput>,
    found: Option<ServerInfo>,
    code: Option<SharedString>,
    busy: bool,
    error: Option<SharedString>,
    focus_next: Option<FocusHandle>,
    attempt: Arc<AtomicU64>,
}

impl AddServer {
    pub fn new(cx: &mut Context<Plaivo>) -> Self {
        let address = cx.new(|cx| TextInput::new("Server address, like 192.168.1.10:8096", cx));
        let username = cx.new(|cx| TextInput::new("Username", cx));
        let password = cx.new(|cx| TextInput::new("Password", cx).masked());
        cx.subscribe(
            &address,
            |this: &mut Plaivo, _, ev: &InputEvent, cx| match ev {
                InputEvent::Submit(_) => this.probe_jellyfin(cx),
                InputEvent::Changed(_) => {
                    this.add_server.cancel();
                    this.add_server.found = None;
                    this.add_server.code = None;
                    cx.notify();
                }
            },
        )
        .detach();
        for input in [&username, &password] {
            cx.subscribe(input, |this: &mut Plaivo, _, ev: &InputEvent, cx| {
                if let InputEvent::Submit(_) = ev {
                    this.sign_in_jellyfin(cx);
                }
            })
            .detach();
        }
        Self {
            address,
            username,
            password,
            found: None,
            code: None,
            busy: false,
            error: None,
            focus_next: None,
            attempt: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn cancel(&mut self) {
        self.attempt.fetch_add(1, Ordering::SeqCst);
        self.busy = false;
    }

    pub fn leave(&mut self, cx: &mut Context<Plaivo>) {
        self.cancel();
        if !self.password.read(cx).text().is_empty() {
            self.password.update(cx, |p, cx| p.set_text("", cx));
        }
    }

    pub fn reset(&mut self, cx: &mut Context<Plaivo>) {
        self.leave(cx);
        self.found = None;
        self.code = None;
        self.error = None;
        self.focus_next = Some(self.address.read(cx).focus.clone());
    }

    fn start(&mut self) -> u64 {
        self.busy = true;
        self.error = None;
        self.code = None;
        self.attempt.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn current(&self, attempt: u64) -> bool {
        self.attempt.load(Ordering::SeqCst) == attempt
    }
}

impl Plaivo {
    pub fn probe_jellyfin(&mut self, cx: &mut Context<Self>) {
        let address = self.add_server.address.read(cx).text().trim().to_owned();
        if address.is_empty() {
            return;
        }
        let attempt = self.add_server.start();
        self.add_server.found = None;
        let c = self.client.clone();
        Self::bg(
            cx,
            move || jellyfin::probe(&c, &address),
            move |this, r, cx| {
                let a = &mut this.add_server;
                if !a.current(attempt) {
                    return;
                }
                a.busy = false;
                match r {
                    Ok(info) => {
                        log::info!(
                            "jellyfin: found {} {} at {}",
                            info.name,
                            info.version,
                            info.uri
                        );
                        a.focus_next = Some(a.username.read(cx).focus.clone());
                        a.found = Some(info);
                    }
                    Err(e) => a.error = Some(format!("Could not connect: {e}").into()),
                }
            },
        );
        cx.notify();
    }

    pub fn sign_in_jellyfin(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.add_server.found.clone() else {
            return self.probe_jellyfin(cx);
        };
        let user = self.add_server.username.read(cx).text().trim().to_owned();
        let password = self.add_server.password.read(cx).text().to_owned();
        if user.is_empty() {
            self.add_server.error = Some("Enter your username".into());
            return cx.notify();
        }
        let attempt = self.add_server.start();
        let c = self.client.clone();
        Self::bg(
            cx,
            move || jellyfin::sign_in(&c, &info, &user, &password),
            move |this, r, cx| this.jellyfin_signed_in(attempt, r, cx),
        );
        cx.notify();
    }

    fn quick_connect(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.add_server.found.clone() else {
            return;
        };
        let attempt = self.add_server.start();
        let c = self.client.clone();
        Self::bg(
            cx,
            move || jellyfin::quick_connect_start(&c, &info).map(|qc| (qc, info)),
            move |this, r, cx| {
                if !this.add_server.current(attempt) {
                    return;
                }
                match r {
                    Ok((qc, info)) => {
                        this.add_server.busy = false;
                        this.add_server.code = Some(qc.code.into());
                        this.poll_quick_connect(attempt, info, qc.secret, cx);
                    }
                    Err(e) => this.jellyfin_signed_in(attempt, Err(e), cx),
                }
            },
        );
        cx.notify();
    }

    fn poll_quick_connect(
        &mut self,
        attempt: u64,
        info: ServerInfo,
        secret: String,
        cx: &mut Context<Self>,
    ) {
        let (c, flag) = (self.client.clone(), self.add_server.attempt.clone());
        Self::bg(
            cx,
            move || -> R<Server> {
                let ready = || match jellyfin::quick_connect_ready(&c, &info, &secret) {
                    Ok(false) => None,
                    Ok(true) => Some(jellyfin::quick_connect_finish(&c, &info, &secret)),
                    Err(e) => Some(Err(e)),
                };
                poll_until(
                    &flag,
                    attempt,
                    QUICK_CONNECT_POLL,
                    Instant::now() + QUICK_CONNECT_FOR,
                    ready,
                )
                .unwrap_or_else(|| Err("Quick Connect timed out. Try again.".into()))
            },
            move |this, r, cx| this.jellyfin_signed_in(attempt, r, cx),
        );
    }

    fn jellyfin_signed_in(&mut self, attempt: u64, r: R<Server>, cx: &mut Context<Self>) {
        if !self.add_server.current(attempt) {
            return;
        }
        self.add_server.busy = false;
        let srv = match r {
            Ok(srv) => srv,
            Err(e) => {
                self.add_server.code = None;
                self.add_server.error = Some(e.into());
                return;
            }
        };
        log::info!("jellyfin: signed in to {} as {}", srv.name, srv.user);
        let name = srv.name.clone();
        self.account.add_jellyfin(srv);
        self.account.save();
        self.add_server.leave(cx);
        self.servers_changed(cx);
        if self.nav.len() > 1 {
            self.back(cx);
        } else {
            self.nav = vec![Nav::Home];
            self.navigate(Nav::Home, false, cx);
        }
        self.toast(format!("Added {name}"), cx);
    }

    fn remove_jellyfin(&mut self, id: &str, cx: &mut Context<Self>) {
        self.account.jellyfin.retain(|s| s.id != id);
        self.account.save();
        self.servers_changed(cx);
    }

    pub fn add_server_standalone(&self) -> bool {
        self.page == Page::AddServer && !self.account.has_servers()
    }

    pub fn field(
        &self,
        id: &'static str,
        input: &Entity<TextInput>,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let focused = input.read(cx).focus.is_focused(window);
        div()
            .id(id)
            .h(px(48.))
            .w_full()
            .px(px(14.))
            .rounded(px(12.))
            .flex()
            .items_center()
            .bg(theme::tint(if focused { 0.18 } else { 0.08 }))
            .child(input.clone())
    }

    pub fn render_add_server(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(focus) = self.add_server.focus_next.take() {
            window.focus(&focus, cx);
        }
        let a = &self.add_server;
        let standalone = self.add_server_standalone();
        let header = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .when(standalone || self.nav.len() > 1, |d| {
                d.child(
                    round_button(
                        "add-server-back",
                        Icon::Back,
                        40.,
                        22.,
                        theme::black(0.),
                        theme::tint(0.08),
                        theme::text(),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.add_server_standalone() {
                            this.add_server.leave(cx);
                            this.page = Page::Auth;
                            cx.notify();
                        } else {
                            this.back(cx);
                        }
                    })),
                )
            })
            .child(
                div()
                    .text_size(px(28.))
                    .font_weight(FontWeight::BOLD)
                    .child("Add Library"),
            );
        let server = section("Jellyfin server")
            .child(note(
                "Enter the address you open Jellyfin with in a browser.",
            ))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(self.field("jf-address", &a.address, window, cx)),
                    )
                    .child(
                        pill("jf-connect", "Connect", None, a.found.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.probe_jellyfin(cx))),
                    ),
            )
            .when_some(a.found.as_ref(), |d, info| {
                d.child(
                    row(
                        "jf-found",
                        Some(Icon::Server),
                        info.name.clone(),
                        join(&[format!("Jellyfin {}", info.version), info.uri.clone()]),
                    )
                    .child(icon(Icon::Check, 22., theme::accent())),
                )
            });
        let sign_in = a.found.as_ref().map(|info| {
            let quick = match &a.code {
                Some(code) => div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().text_size(px(13.)).text_color(theme::muted()).child(format!("In Jellyfin on a signed-in device, open your profile → Quick Connect and enter this code for {}.", info.name)))
                    .child(div().text_size(px(40.)).font_weight(FontWeight::BOLD).child(code.clone()))
                    .child(div().text_size(px(13.)).text_color(theme::muted()).child("Waiting for approval…")),
                None => div(),
            };
            section("Sign in")
                .child(self.field("jf-username", &a.username, window, cx))
                .child(self.field("jf-password", &a.password, window, cx))
                .child(
                    div()
                        .flex()
                        .gap(px(12.))
                        .pt(px(4.))
                        .child(pill("jf-sign-in", "Sign in", None, true).on_click(cx.listener(|this, _, _, cx| this.sign_in_jellyfin(cx))))
                        .child(pill("jf-quick", "Use Quick Connect", None, false).on_click(cx.listener(|this, _, _, cx| this.quick_connect(cx)))),
                )
                .child(quick)
        });
        let status = div()
            .when(a.busy, |d| {
                d.text_size(px(13.))
                    .text_color(theme::muted())
                    .child("Connecting…")
            })
            .when_some(a.error.clone(), |d, e| {
                d.text_size(px(13.)).text_color(theme::error()).child(e)
            });
        div()
            .id("add-server")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(640.))
                    .px(px(32.))
                    .py(px(32.))
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .child(header)
                    .child(server)
                    .children(sign_in)
                    .child(status),
            )
            .into_any_element()
    }

    pub fn render_connections(&mut self, cx: &mut Context<Self>) -> Div {
        let plex = match self.account.token {
            Some(_) => row(
                "plex-account",
                Some(Icon::Person),
                "Plex account",
                "Signed in",
            )
            .child(
                pill("sign-out", "Sign out", Some(Icon::Logout), false)
                    .on_click(cx.listener(|this, _, _, cx| this.sign_out(cx))),
            ),
            None => row(
                "plex-account",
                Some(Icon::Person),
                "Plex account",
                "Not signed in",
            )
            .child(
                pill("sign-in", "Sign in", None, false).on_click(cx.listener(|this, _, _, cx| {
                    this.page = Page::Auth;
                    this.sign_in(cx);
                })),
            ),
        };
        let jellyfin: Vec<_> = self
            .account
            .jellyfin
            .iter()
            .map(|s| {
                let id = s.id.clone();
                row(
                    SharedString::from(format!("jf-{id}")),
                    Some(Icon::Server),
                    s.name.clone(),
                    join(&[s.user.clone(), s.uri.clone()]),
                )
                .child(
                    pill(
                        SharedString::from(format!("jf-remove-{id}")),
                        "Remove",
                        None,
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.remove_jellyfin(&id, cx))),
                )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(section("Plex").child(plex))
            .child(
                section("Jellyfin").children(jellyfin).child(
                    nav_row(
                        "add-jellyfin",
                        Icon::Add,
                        "Add server",
                        "Sign in with a username and password, or Quick Connect",
                    )
                    .on_click(
                        cx.listener(|this, _, _, cx| this.navigate(Nav::AddServer, true, cx)),
                    ),
                ),
            )
    }
}
