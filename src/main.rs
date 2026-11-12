#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clock;
mod images;
mod logging;
mod menu;
mod mpv;
mod placement;
mod player;
mod shaders;
mod ui;
mod video_host;
mod watch;

use gpui::{App, AppContext, TitlebarOptions, WindowBackgroundAppearance, WindowOptions, px, size};
use plaivo_core::{downloads, plex, prefs, settings, store};
use std::sync::Arc;

fn main() {
    let settings = settings::Settings::load();
    logging::init(settings.debug_logging);
    let account = plaivo_core::Account::load();
    let client = plex::Client::new(account.client_id.clone());
    let queue = Arc::new(images::Queue::default());
    let (tx, rx) = futures::channel::mpsc::unbounded();
    images::spawn_workers(queue.clone(), client.clone(), move |key, img| {
        let _ = tx.unbounded_send((key, img));
    });
    std::thread::spawn(|| store::prune_cache(plaivo_core::DISK_CACHE_BYTES));

    gpui_platform::application()
        .with_assets(ui::theme::Assets)
        .run(move |cx: &mut App| {
            ui::input::bind_keys(cx);
            if cfg!(target_os = "macos") {
                menu::install(cx);
            }
            let (bounds, display_id) = placement::restore(placement::load().as_ref(), cx);
            let options = WindowOptions {
                window_bounds: Some(bounds),
                display_id,
                titlebar: Some(TitlebarOptions {
                    title: Some("Plaivo".into()),
                    ..Default::default()
                }),
                window_background: WindowBackgroundAppearance::Transparent,
                window_min_size: Some(size(px(720.), px(480.))),
                draw_above_child_windows: true,
                ..Default::default()
            };
            let window = cx
                .open_window(options, |window, cx| {
                    let app = cx.new(|cx| {
                        app::Plaivo::new(window, cx, account, settings, client, queue, rx)
                    });
                    cx.new(|cx| ui::shell::Root::new(app, cx))
                })
                .expect("open window");
            if let Some(target) = std::env::args().nth(1) {
                window
                    .update(cx, |root, _, cx| {
                        root.app.update(cx, |app, cx| {
                            if let Err(e) =
                                app.launch(&target, "", &target, "", 0.0, None, Vec::new(), "", cx)
                            {
                                app.toast(e, cx);
                            }
                        })
                    })
                    .ok();
            }
            cx.activate(true);
        });
}
