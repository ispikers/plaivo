#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod clock;
mod images;
mod logging;
mod mpv;
mod player;
mod shaders;
mod ui;
mod video_host;
mod watch;

use gpui::{
    App, AppContext, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, px, size,
};
use plaivo_core::{downloads, plex, prefs, settings, store};
use std::sync::Arc;

actions!(plaivo, [Quit]);

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
                cx.on_action(|_: &Quit, cx| cx.quit());
                cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
                cx.set_menus(vec![
                    Menu::new("Plaivo").items([MenuItem::action("Quit Plaivo", Quit)]),
                ]);
            }
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1440.), px(900.)),
                    cx,
                ))),
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
                    cx.new(|cx| app::Plaivo::new(window, cx, account, settings, client, queue, rx))
                })
                .expect("open window");
            if let Some(target) = std::env::args().nth(1) {
                window
                    .update(cx, |app, _, cx| {
                        if let Err(e) =
                            app.launch(&target, "", &target, "", 0.0, None, Vec::new(), cx)
                        {
                            app.toast(e, cx);
                        }
                    })
                    .ok();
            }
            cx.activate(true);
        });
}
