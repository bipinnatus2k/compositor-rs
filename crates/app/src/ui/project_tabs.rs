//! Port of `ProjectTabs.swift`: the project workspace's tab strip
//! (`ProjectTabStrip`) — one pill per tab with title, unsaved dot and close
//! button, plus the "New" slot — and the pill-width math it shares.

use gpui_kit::{Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, px};

use crate::project::Step;
use crate::state::AppState;

/// `projectTabLabelWidth` (ProjectTabs.swift:59-64): the title's rendered
/// width plus the 10px unsaved dot when it shows, clamped to 35...155. The
/// port estimates the title width at 7px per character (the window's 12px
/// UI face) — gpui lays the label out itself, so the number drives the
/// pill's width cap rather than an exact frame.
pub(crate) fn project_tab_label_width(title: &str, modified: bool) -> f32 {
    let title_width = title.chars().count() as f32 * 7.0;
    let dot_width = if modified { 10.0 } else { 0.0 };
    155.0f32.min(35.0f32.max((title_width + dot_width).ceil()))
}

/// `projectTabPillWidth` (:66-69): the label plus 11px leading, 8px
/// trailing, the 16px close button and the 5px gap after it.
pub(crate) fn project_tab_pill_width(title: &str, modified: bool) -> f32 {
    project_tab_label_width(title, modified) + 40.0
}

impl AppState {
    /// The 34px project tab strip (`ProjectTabs.swift`'s `ProjectTabStrip`):
    /// one pill per tab — title, unsaved dot, close button — plus a "New"
    /// slot. Clicking selects (`workspace.select`, refitting the viewport);
    /// tabs and close buttons disable while `canSwitch` is false.
    pub(crate) fn render_tab_bar(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        use gpui_kit::ExternalPaths;
        use gpui_kit::component::button::{Button, ButtonVariants};
        use gpui_kit::component::tab::{Tab, TabBar};
        use gpui_kit::component::{Disableable, Selectable};

        let can_switch = self.workspace.can_switch(cx);
        let selected = self.workspace.selected_id();
        let tabs: Vec<(crate::project::TabId, String, bool)> = self
            .workspace
            .tabs()
            .iter()
            .map(|tab| (tab.id, tab.title(cx), tab.session.read(cx).is_modified()))
            .collect();

        let mut bar = TabBar::new("project-tabs")
            .pill()
            .max_width(px(project_tab_pill_width("x".repeat(60).as_str(), true)));
        for (id, title, modified) in tabs {
            let active = id == selected;
            let mut tab = Tab::new()
                .label(title.clone())
                .selected(active)
                .disabled(!can_switch && !active)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.workspace.select(id, cx) {
                        this.sync_session_observer(cx);
                        this.touch(cx);
                    }
                }));
            if modified {
                tab = tab.prefix(crate::flows::unsaved_dot());
            }
            tab = tab.suffix(
                Button::new(SharedString::from(format!("tab-close-{}", id)))
                    .label("\u{d7}")
                    .ghost()
                    .compact()
                    .disabled(!can_switch)
                    // Swift's close button carries `help("Close \(title)")`
                    // (ProjectTabs.swift:242); the port's icon-only button
                    // says the same to assistive tech.
                    .accessibility_label(format!("Close {title}"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        match this.workspace.close(id, cx) {
                            Step::Pending(pending) => this.present_pending(pending, window, cx),
                            Step::Done(()) => {
                                this.sync_session_observer(cx);
                                this.touch(cx);
                            }
                        }
                    })),
            );
            bar = bar.child(tab);
        }
        // The "New" slot (`NewTabDropSlot`, ProjectTabs.swift:200-216) is a
        // drop target in Swift; v1 makes it a plain New-tab button (the
        // workspace design's deviation #3).
        bar = bar.suffix(
            Button::new("tab-new")
                .label("+ New")
                .ghost()
                .compact()
                .disabled(!can_switch)
                .on_click(cx.listener(|this, _, _, cx| {
                    if this.workspace.can_switch(cx) {
                        this.workspace.add_tab(true, cx);
                        this.sync_session_observer(cx);
                        this.touch(cx);
                    }
                })),
        );

        gpui_kit::div()
            .id("tab-strip")
            .flex()
            .items_center()
            .h(px(34.0))
            .px_1()
            .flex_shrink_0()
            .bg(crate::ui::panel_bg())
            .border_b_1()
            .border_color(gpui_kit::black().opacity(0.4))
            // Dropping files on the strip opens them (into=nil: images get
            // their own tabs, packages load) — `ProjectTabs`' strip target.
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                style.bg(gpui_kit::rgb(crate::ui::BG_DROP_STRIP))
            })
            .on_drop::<ExternalPaths>(cx.listener(|this, dropped: &ExternalPaths, _window, cx| {
                let paths = dropped.paths().to_vec();
                let failures = this.workspace.receive(&paths, None, None, cx);
                this.sync_session_observer(cx);
                if !failures.is_empty() {
                    this.set_status(failures.join("; "), cx);
                }
                this.touch(cx);
            }))
            .child(bar)
            .into_any_element()
    }
}

#[cfg(test)]
mod tab_width_tests {
    use super::{project_tab_label_width, project_tab_pill_width};

    /// The clamp from ProjectTabs.swift:59-64.
    #[test]
    fn tab_label_width_clamps_like_swift() {
        // Short titles bottom out at 35.
        assert_eq!(project_tab_label_width("", false), 35.0);
        assert_eq!(project_tab_label_width("Untitled", false), 56.0);
        // The unsaved dot adds 10 (5px dot and 5px gap).
        assert_eq!(project_tab_label_width("Untitled", true), 66.0);
        // Long titles cap at 155.
        let long = "a".repeat(40);
        assert_eq!(project_tab_label_width(&long, true), 155.0);
    }

    /// The pill adds 40 for padding + the close button (:66-69).
    #[test]
    fn tab_pill_width_adds_forty() {
        assert_eq!(project_tab_pill_width("Untitled", false), 96.0);
        assert_eq!(project_tab_pill_width("Untitled", true), 106.0);
        assert_eq!(project_tab_pill_width(&"x".repeat(60), false), 195.0);
    }
}
