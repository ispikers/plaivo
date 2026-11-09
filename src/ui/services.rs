use crate::app::{Plaivo, open_url, poll_until};
use crate::plex::Item;
use crate::ui::settings::{note, toggle_row};
use crate::ui::theme::{self, Icon};
use crate::ui::widgets::{pill, row, section};
use gpui::{Context, Div, FontWeight, SharedString, div, prelude::*, px};
use plaivo_core::trackers::{Media, Poll, Service, SignIn, Trackers};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const WATCHED_PCT: f64 = 90.0;

struct Pending {
    service: Service,
    url: Option<SharedString>,
    code: Option<SharedString>,
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
        s.pending = Some(Pending { service, url: None, code: None });
        let (attempt, t) = (s.attempt.load(Ordering::SeqCst), s.trackers.clone());
        Self::bg(cx, move || t.begin_sign_in(service), move |this, r, cx| {
            if !this.services.current(attempt) {
                return;
            }
            match r {
                Ok(sign_in) => {
                    open_url(&sign_in.url);
                    this.services.pending = Some(Pending { service, url: Some(sign_in.url.clone().into()), code: sign_in.user_code.clone().map(Into::into) });
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
        let waiting = pending.map(|p| {
            let reopen = p.url.clone().map(|url| pill((id, 2usize), "Open again", Some(Icon::OpenInNew), false).on_click(move |_, _, _| open_url(&url)));
            let text = match (&p.url, &p.code) {
                (None, _) => "Connecting…".to_owned(),
                (Some(url), Some(_)) => format!("Go to {url} and enter this code. Waiting for approval…"),
                (Some(_), None) => "Finish signing in in your browser. Waiting for approval…".to_owned(),
            };
            div()
                .flex()
                .items_center()
                .gap(px(16.))
                .px(px(20.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .when_some(p.code.clone(), |d, code| d.child(div().text_size(px(32.)).font_weight(FontWeight::BOLD).child(code)))
                        .child(div().text_size(px(13.)).text_color(theme::muted()).child(text)),
                )
                .children(reopen)
        });
        let error = self.services.error.as_ref().filter(|(s, _)| *s == service).map(|(_, e)| div().px(px(20.)).text_size(px(13.)).text_color(theme::error()).child(e.clone()));
        let scrobble_detail = match service {
            Service::Trakt => "Marks movies and episodes watched on Trakt",
            _ => "Updates your anime list when an episode is watched",
        };
        section(id)
            .child(account)
            .children(waiting)
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

    pub fn render_services(&mut self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(note("Anime is matched to MyAnimeList by its TVDB, TMDB or AniDB ID."))
            .children(Service::ALL.map(|s| self.render_service(s, cx)))
    }
}
