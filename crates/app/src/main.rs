//! Compositor: a Photoshop-like image editor. The binary opens one dark
//! window (see `workspace.rs`) whose state is the `EditorSession` plus the
//! viewport, tools and sheets.

mod canvas;
mod keys;
mod layers_panel;
mod menu;
mod sheets;
mod sheets_ui;
mod state;
mod text_bitmap;
mod tools;
mod ui;
mod workspace;

use gpui_kit::{px, point, size, Bounds, TitlebarOptions, WindowBounds, WindowOptions};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{Root, TitleBar};
use gpui_kit::AppContext;

use state::AppState;

fn main() {
    gpui_kit::application().run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys(keys::bindings());
        Theme::change(ThemeMode::Dark, None, cx);

        cx.spawn(async move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(40.0), px(30.0)),
                    size(px(1560.0), px(920.0)),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Compositor".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(980.0), px(640.0))),
                ..TitleBar::window_options()
            };
            let window = cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| AppState::new(window, cx));
                let focus = view.read(cx).focus.clone();
                window.focus(&focus, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            if let Err(error) = window {
                eprintln!("failed to open the Compositor window: {error}");
            }
        })
        .detach();
    });
}
