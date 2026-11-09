use crate::app::{Plaivo, open_url, poll_until};
use crate::plex::Item;
use crate::ui::settings::{note, toggle_row};
use crate::ui::player::spinner;
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{backdrop, floating, pill, row, section, shadow};
use gpui::{AnyElement, Context, Div, FontWeight, SharedString, canvas, div, fill, point, prelude::*, px, size};
use plaivo_core::trackers::{Media, Poll, Service, SignIn, Trackers};
use qrcodegen::{QrCode, QrCodeEcc};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const WATCHED_PCT: f64 = 90.0;

const QR_SIZE: f32 = 220.;

struct Pending {
    service: Service,
    url: Option<SharedString>,
    code: Option<SharedString>,
    qr: Option<(usize, Vec<bool>)>,
}

pub struct Services {
    pub trackers: Trackers,
    pending: Option<Pending>,
    error: Option<(Service, SharedString)>,
    attempt: Arc<AtomicU64>,
}

impl Services {
    pub fn start() -> Self {
        let trackers = Trackers::start();
        trackers.set_watched_threshold(WATCHED_PCT);
        Self { trackers, pending: None, error: None, attempt: Arc::new(AtomicU64::new(0)) }
    }

    pub fn leave(&mut self) {
        self.attempt.fetch_add(1, Ordering::SeqCst);
        self.pending = None;
    }

    pub fn cancel(&mut self) -> bool {
        let open = self.pending.is_some();
        self.leave();
        open
    }

    fn current(&self, attempt: u64) -> bool {
        self.attempt.load(Ordering::SeqCst) == attempt
    }

    fn active(&self) -> bool {
        Service::ALL.into_iter().any(|s| {
            let st = self.trackers.status(s);
            st.signed_in && st.scrobble
        })
    }

    pub fn summary(&self) -> String {
        let signed_in: Vec<&str> = Service::ALL.into_iter().filter(|&s| self.trackers.status(s).signed_in).map(name).collect();
        match signed_in.is_empty() {
            false => format!("Signed in: {}", signed_in.join(", ")),
            true if Service::ALL.into_iter().any(Service::configured) => "Not signed in".into(),
            true => "Not configured".into(),
        }
    }
}

fn name(s: Service) -> &'static str {
    match s {
        Service::Trakt => "Trakt",
        Service::Mal => "MyAnimeList",
    }
}

fn qr(text: &str) -> Option<(usize, Vec<bool>)> {
    let qr = QrCode::encode_text(text, QrCodeEcc::Medium).ok()?;
    let n = qr.size();
    Some((n as usize, (0..n).flat_map(|y| (0..n).map(move |x| (x, y))).map(|(x, y)| qr.get_module(x, y)).collect()))
}

fn qr_code((n, modules): (usize, Vec<bool>)) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let cell = f32::from(bounds.size.width) / n as f32;
            for (y, line) in modules.chunks(n).enumerate() {
                let mut x = 0;
                while x < n {
                    let run = line[x..].iter().take_while(|&&dark| dark == line[x]).count();
                    if line[x] {
                        let origin = bounds.origin + point(px(x as f32 * cell), px(y as f32 * cell));
                        window.paint_quad(fill(gpui::Bounds::new(origin, size(px(run as f32 * cell), px(cell))), gpui::black()));
                    }
                    x += run;
                }
            }
        },
    )
    .size(px(QR_SIZE))
}

fn failure(e: &str) -> String {
    match e {
        "expired" => "The sign-in request expired. Try again.".into(),
        "denied" => "Sign-in was denied.".into(),
        e => format!("Sign-in failed: {e}"),
    }
}

impl Plaivo {
    pub fn resolve_tracker_media(&mut self, it: &Item, cx: &mut Context<Self>) {
        let Some(srv) = self.player.server.clone().filter(|_| self.services.active()) else { return };
        let Some(mut media) = Media::for_item(it) else { return };
        let Some(show) = Some(it.grandparent_rating_key.clone()).filter(|k| it.kind == "episode" && !k.is_empty()) else {
            self.player.tracker = Some(media);
            return;
        };
        let (c, downloads, trackers, key, server) = (self.client.clone(), self.downloads.clone(), self.services.trackers.clone(), it.rating_key.clone(), srv.id.clone());
        Self::bg(
            cx,
            move || trackers.show_ids(&c, &srv, downloads.as_deref(), &show),
            move |this, ids, _| {
                if this.player.key == key && this.player.server.as_ref().is_some_and(|s| s.id == server) {
                    media.show_ids = ids.unwrap_or_default();
                    this.player.tracker = Some(media);
                }
            },
        );
    }

    fn service_button(&mut self, service: Service, cx: &mut Context<Self>) {
        if self.services.trackers.status(service).signed_in {
            return self.set_tracker(cx, move |t| t.sign_out(service));
        }
        let s = &mut self.services;
        let waiting = s.pending.as_ref().is_some_and(|p| p.service == service);
        s.leave();
        s.error = None;
        if waiting {
            return cx.notify();
        }
        s.pending = Some(Pending { service, url: None, code: None, qr: None });
        let (attempt, t) = (s.attempt.load(Ordering::SeqCst), s.trackers.clone());
        Self::bg(cx, move || t.begin_sign_in(service), move |this, r, cx| {
            if !this.services.current(attempt) {
                return;
            }
            match r {
                Ok(sign_in) => {
                    let (url, code) = (sign_in.url.clone(), sign_in.user_code.clone().map(Into::into));
                    this.services.pending = Some(Pending { service, qr: qr(&url), url: Some(url.into()), code });
                    this.poll_service(attempt, sign_in, cx);
                }
                Err(e) => {
                    this.services.pending = None;
                    this.services.error = Some((service, format!("Could not start sign-in: {e}").into()));
                }
            }
        });
        cx.notify();
    }

    fn poll_service(&mut self, attempt: u64, mut sign_in: SignIn, cx: &mut Context<Self>) {
        let (t, flag) = (self.services.trackers.clone(), self.services.attempt.clone());
        let service = sign_in.service;
        Self::bg(
            cx,
            move || {
                let (every, deadline) = (Duration::from_secs(sign_in.interval_secs), Instant::now() + Duration::from_secs(sign_in.expires_in_secs));
                poll_until(&flag, attempt, every, deadline, || Some(t.poll_sign_in(&mut sign_in)).filter(|r| !matches!(r, Poll::Pending))).unwrap_or_else(|| Poll::Failed("expired".into()))
            },
            move |this, r, _| {
                if !this.services.current(attempt) {
                    return;
                }
                this.services.pending = None;
                match r {
                    Poll::Failed(e) => this.services.error = Some((service, failure(&e).into())),
                    _ => log::info!("{} signed in", name(service)),
                }
            },
        );
    }

    fn set_tracker(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&Trackers) + Send + 'static) {
        let t = self.services.trackers.clone();
        Self::bg(cx, move || f(&t), |_, _, _| {});
    }

    fn render_service(&self, service: Service, cx: &mut Context<Self>) -> Div {
        let id = name(service);
        let st = self.services.trackers.status(service);
        let pending = self.services.pending.as_ref().filter(|p| p.service == service && !st.signed_in);
        let (status, button) = match (st.configured, st.signed_in, pending.is_some()) {
            (false, ..) => ("Not configured", None),
            (_, true, _) => ("Signed in", Some("Disconnect")),
            (_, _, true) => ("Not signed in", Some("Cancel")),
            _ => ("Not signed in", Some("Connect")),
        };
        let account = row((id, 0usize), Some(Icon::Person), "Account", status).when_some(button, |d, label| {
            d.child(pill((id, 1usize), label, None, label == "Connect").on_click(cx.listener(move |this, _, _, cx| this.service_button(service, cx))))
        });
        let error = self.services.error.as_ref().filter(|(s, _)| *s == service).map(|(_, e)| div().px(px(20.)).text_size(px(13.)).text_color(theme::error()).child(e.clone()));
        let scrobble_detail = match service {
            Service::Trakt => "Marks movies and episodes watched on Trakt",
            _ => "Updates your anime list when an episode is watched",
        };
        section(id)
            .child(account)
            .children(error)
            .when(st.signed_in, |d| {
                d.child(toggle_row((id, 3usize), "Scrobble", scrobble_detail, st.scrobble).on_click(cx.listener(move |this, _, _, cx| {
                    this.set_tracker(cx, move |t| t.set_scrobble(service, !st.scrobble));
                })))
            })
            .when(st.signed_in && service == Service::Trakt, |d| {
                let on = self.services.trackers.trakt_progress();
                d.child(toggle_row((id, 4usize), "Scrobble progress", "Shows what you're watching on Trakt while it plays", on).on_click(cx.listener(move |this, _, _, cx| {
                    this.set_tracker(cx, move |t| t.set_trakt_progress(!on));
                })))
            })
    }

    pub fn render_sign_in(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.services.pending.as_ref()?;
        let title = name(p.service);
        let close = cx.listener(|this, _, _, cx| {
            this.services.leave();
            cx.notify();
        });
        let waiting = |label: &'static str| div().flex().items_center().gap(px(10.)).text_size(px(13.)).text_color(theme::muted()).child(spinner(16., theme::muted())).child(label);
        let body = match &p.url {
            None => div().h(px(QR_SIZE)).flex().items_center().justify_center().child(waiting("Connecting…")),
            Some(url) => {
                let open = url.clone();
                div()
                    .flex()
                    .gap(px(28.))
                    .child(div().flex_none().p(px(18.)).rounded(px(12.)).bg(gpui::white()).children(p.qr.clone().map(qr_code)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .justify_center()
                            .gap(px(14.))
                            .child(div().text_size(px(14.)).text_color(theme::muted()).child("Scan this QR code or open the link on any device."))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(SharedString::from(url.trim_start_matches("https://").to_owned())))
                            .when_some(p.code.clone(), |d, code| d.child(div().text_size(px(32.)).font_weight(FontWeight::BOLD).child(code)))
                            .child(pill("sign-in-open", format!("Open {title} to sign in"), Some(Icon::OpenInNew), true).on_click(move |_, _, _| open_url(&open)))
                            .child(waiting("Waiting for authorization…")),
                    )
            }
        };
        let modal = floating("sign-in")
            .w(px(640.))
            .p(px(28.))
            .rounded(px(16.))
            .bg(theme::bg())
            .border_1()
            .border_color(theme::outline())
            .shadow(shadow(32., 0.6))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(div().text_size(px(22.)).font_weight(FontWeight::BOLD).child(format!("Sign in to {title}")))
            .child(body)
            .child(div().flex().justify_end().child(pill("sign-in-cancel", "Cancel", None, false).on_click(cx.listener(|this, _, _, cx| {
                this.services.leave();
                cx.notify();
            }))));
        Some(backdrop("sign-in-backdrop", close).bg(theme::black(0.55)).flex().items_center().justify_center().child(modal).into_any_element())
    }

    pub fn render_services(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(note("Anime is matched to MyAnimeList by its TVDB, TMDB or AniDB ID."))
            .children(Service::ALL.map(|s| self.render_service(s, cx)))
    }
}
