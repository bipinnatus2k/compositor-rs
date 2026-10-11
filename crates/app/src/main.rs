//! Compositor: a Photoshop-like image editor. The binary opens one dark
//! window whose root is the `ProjectWorkspaceView` entity (the workspace's
//! session plus viewport, tools and sheets). All application logic lives in
//! the library target (`lib.rs`), so the binary only wires up the window.

use compositor_app::{keys, state};
use gpui_kit::AppContext;
use gpui_kit::assets::AllAssets;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::{Root, TitleBar};
use gpui_kit::{Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, px, size};
use state::ProjectWorkspaceView;

fn main() {
    gpui_kit::application().with_assets(AllAssets).run(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys(keys::bindings());
        Theme::change(ThemeMode::Dark, None, cx);
        // Panels and sheets paint a hand-picked 0x2d2d2d; the default dark
        // theme's `input` border — the unchecked checkbox indicator and
        // input outlines — is nearly the same gray and disappears on it.
        cx.global_mut::<Theme>().input = gpui_kit::rgb(0x5f5f5f).into();

        cx.spawn(async move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(40.0), px(30.0)),
                    size(px(1560.0), px(920.0)),
                ))),
                // The component's base brings `appears_transparent` and the
                // macOS traffic-light position its 80px left padding assumes;
                // the title stays for the taskbar. The in-window bar is
                // rendered by the root view (content_view.rs).
                titlebar: Some(TitlebarOptions {
                    title: Some("Compositor".into()),
                    ..TitleBar::title_bar_options()
                }),
                window_min_size: Some(size(px(980.0), px(640.0))),
                ..TitleBar::window_options()
            };
            let window = cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| ProjectWorkspaceView::new(window, cx));
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
