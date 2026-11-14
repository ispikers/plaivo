use crate::app::Plaivo;
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{Context, SharedString};
use std::sync::OnceLock;
use std::time::Duration;
use velopack::sources::HttpSource;
use velopack::{UpdateCheck, UpdateInfo, UpdateManager, VelopackApp};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "https://github.com/ispikers/plaivo";
const FIRST_CHECK: Duration = Duration::from_secs(30);
const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

static RESTARTED_INTO: OnceLock<String> = OnceLock::new();

// Velopack's hooks exit the process; a normal launch returns.
pub fn run_installer_hooks() {
    VelopackApp::build()
        .on_restarted(|v| {
            let _ = RESTARTED_INTO.set(v.to_string());
        })
        .run();
}

pub enum Status {
    Unavailable(SharedString),
    Idle,
    Checking,
    Current,
    Downloading(Box<UpdateInfo>, u8),
    Ready(Box<UpdateInfo>),
    Installing,
    Failed(SharedString),
}

pub enum Event {
    Checked(Result<Option<Box<UpdateInfo>>, String>),
    Progress(u8),
    Downloaded(Box<UpdateInfo>, Result<(), String>),
}

pub struct Update {
    pub status: Status,
    manager: Option<UpdateManager>,
    tx: UnboundedSender<Event>,
}

pub fn version_of(info: &UpdateInfo) -> &str {
    &info.TargetFullRelease.Version
}

pub fn notes_url(info: &UpdateInfo) -> String {
    format!("{REPO}/releases/tag/v{}", version_of(info))
}

impl Update {
    pub fn start(cx: &mut Context<Plaivo>) -> Self {
        if let Some(v) = RESTARTED_INTO.get() {
            let msg = format!("Updated to Plaivo {v}");
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.toast(msg, cx)).ok();
            })
            .detach();
        }

        let (tx, mut rx) = unbounded();
        cx.spawn(async move |this, cx| {
            while let Some(ev) = rx.next().await {
                if this.update(cx, |this, cx| this.on_update(ev, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let manager = if cfg!(debug_assertions) {
            Err("Development builds don't update themselves.".to_owned())
        } else {
            UpdateManager::new(
                HttpSource::new(format!("{REPO}/releases/latest/download")),
                None,
                None,
            )
            .map_err(|e| {
                log::info!("updates unavailable: {e}");
                "This copy wasn't installed with the Plaivo installer, so it can't update itself."
                    .to_owned()
            })
        };
        let status = match &manager {
            Ok(_) => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(FIRST_CHECK).await;
                    while this
                        .update(cx, |this, cx| this.check_for_updates(cx))
                        .is_ok()
                    {
                        cx.background_executor().timer(CHECK_EVERY).await;
                    }
                })
                .detach();
                Status::Idle
            }
            Err(why) => Status::Unavailable(why.clone().into()),
        };
        Self {
            status,
            manager: manager.ok(),
            tx,
        }
    }

    pub fn attention(&self) -> bool {
        matches!(self.status, Status::Ready(_))
    }

    pub fn summary(&self) -> Option<&'static str> {
        match self.status {
            Status::Ready(_) => Some("Update ready"),
            _ => None,
        }
    }
}

impl Plaivo {
    pub fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        if self.settings.check_updates {
            self.check_for_updates_now(cx);
        }
    }

    pub fn check_for_updates_now(&mut self, cx: &mut Context<Self>) {
        let u = &mut self.updates;
        let idle = matches!(u.status, Status::Idle | Status::Current | Status::Failed(_));
        let Some(manager) = u.manager.clone() else {
            return;
        };
        if !idle {
            return;
        }
        u.status = Status::Checking;
        let tx = u.tx.clone();
        std::thread::spawn(move || {
            let found = match manager.check_for_updates() {
                Ok(UpdateCheck::UpdateAvailable(info)) => Ok(Some(info)),
                Ok(UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty) => Ok(None),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.unbounded_send(Event::Checked(found));
        });
        cx.notify();
    }

    fn download_update(&mut self, info: Box<UpdateInfo>, cx: &mut Context<Self>) {
        let u = &mut self.updates;
        let Some(manager) = u.manager.clone() else {
            return;
        };
        u.status = Status::Downloading(info.clone(), 0);
        let tx = u.tx.clone();
        std::thread::spawn(move || {
            let (progress, pct) = std::sync::mpsc::channel::<i16>();
            let forward = tx.clone();
            std::thread::spawn(move || {
                for p in pct {
                    let _ = forward.unbounded_send(Event::Progress(p.clamp(0, 100) as u8));
                }
            });
            let done = manager
                .download_updates(&info, Some(progress))
                .map_err(|e| e.to_string());
            let _ = tx.unbounded_send(Event::Downloaded(info, done));
        });
        cx.notify();
    }

    pub fn restart_to_update(&mut self, cx: &mut Context<Self>) {
        let u = &self.updates;
        let (Status::Ready(info), Some(manager)) = (&u.status, &u.manager) else {
            return;
        };
        if let Err(e) =
            manager.wait_exit_then_apply_updates(&**info, false, true, Vec::<String>::new())
        {
            log::error!("could not start the update: {e}");
            let msg = format!("Couldn't start the update: {e}");
            self.toast(msg.clone(), cx);
            self.updates.status = Status::Failed(msg.into());
            return cx.notify();
        }
        log::info!("quitting to install the update");
        self.updates.status = Status::Installing;
        #[cfg(target_os = "macos")]
        cx.hide();
        cx.quit();
    }

    fn on_update(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::Checked(Ok(Some(info))) => return self.download_update(info, cx),
            Event::Checked(Ok(None)) => self.updates.status = Status::Current,
            Event::Checked(Err(e)) => {
                log::warn!("update check failed: {e}");
                self.updates.status = Status::Failed(e.into());
            }
            Event::Progress(pct) => {
                if let Status::Downloading(_, shown) = &mut self.updates.status {
                    *shown = pct;
                }
            }
            Event::Downloaded(info, Ok(())) => {
                log::info!("update {} is ready", version_of(&info));
                self.toast(
                    format!(
                        "Plaivo {} is ready. Restart Plaivo from Settings → About to install it.",
                        version_of(&info)
                    ),
                    cx,
                );
                self.updates.status = Status::Ready(info);
            }
            Event::Downloaded(_, Err(e)) => {
                log::warn!("update download failed: {e}");
                self.updates.status = Status::Failed(e.into());
            }
        }
        cx.notify();
    }
}
