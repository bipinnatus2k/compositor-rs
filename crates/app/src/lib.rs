//! Compositor's application crate. The modules here hold the editor's
//! window-side logic (state, tools, sheets, canvas and panels); `main.rs`
//! only opens the window and wires them up. Keeping the logic in a library
//! target lets unit tests exercise it without a GPUI window.

pub mod canvas;
pub mod flows;
pub mod color_picker_ui;
pub mod keys;
pub mod layers_panel;
pub mod menu;
pub mod panels;
pub mod panels_ui;
pub mod project;
pub mod settings;
pub mod sheets;
pub mod sheets_ui;
pub mod shortcuts;
pub mod state;
pub mod tabs;
pub mod tools;
pub mod ui;
pub mod workspace;
