//! Offline page composition. Slots contain only trusted bundled assets; graph
//! JSON is escaped and inserted last so repository text cannot become a slot.

pub(crate) enum ExplorerView {
    Dependencies,
    Calls,
}

pub(crate) fn render(view: ExplorerView, data: &str) -> String {
    let calls = matches!(view, ExplorerView::Calls);
    let (style, navigator, canvas, details, script) = if calls {
        (
            include_str!("call_viewer.css"),
            include_str!("call_viewer.navigator.html"),
            include_str!("call_viewer.canvas.html"),
            include_str!("call_viewer.details.html"),
            format!(
                "{}\n{}",
                include_str!("call_source.js"),
                include_str!("call_viewer.js")
            ),
        )
    } else {
        (
            include_str!("dependency_viewer.css"),
            include_str!("dependency_viewer.navigator.html"),
            include_str!("dependency_viewer.canvas.html"),
            include_str!("dependency_viewer.details.html"),
            format!(
                "{}\n{}",
                include_str!("dependency_explorer.js"),
                include_str!("dependency_viewer.js")
            )
            .replace(
                "__CODEGRAIDE_DEPENDENCY_CONTROLS__",
                include_str!("dependency_controls.js"),
            ),
        )
    };
    let mut page = include_str!("explorer_shell.html").to_owned();
    for (slot, dependency, call) in [
        ("TITLE", "Dependency Explorer", "Call Graph Explorer"),
        ("TOOL", "dependencies", "calls"),
        ("DETAILS_OPEN", "true", "false"),
        ("DETAILS_ACTION", "Hide", "Show"),
        ("APP_CLASS", "app", ""),
        ("WORKSPACE_CLASS", "workspace", "layout"),
        ("LEFT_CLASS", "navigator", "pane left"),
        ("CENTER_CLASS", "graph-center", "center"),
        ("RIGHT_CLASS", "sidebar", "pane right"),
        ("LEFT_TOGGLE", "toggle-nav", "leftPaneToggle"),
        ("RIGHT_TOGGLE", "toggle-details", "rightPaneToggle"),
        (
            "SEARCH_LABEL",
            "Find a file or module",
            "Find a file, class, function, module, or architecture group",
        ),
        ("BACK", "back", "backButton"),
        ("FOCUS_TITLE", "focus-title", "graphTitle"),
        ("DEPTH_SLIDER", "depth-slider", "depthSlider"),
        ("DEPTH_INPUT", "depth", "depthInput"),
        ("MIN_DEPTH", "0", "1"),
        ("MAX_DEPTH", "20", "10"),
        ("ZOOM_OUT", "zoom-out", "zoomOut"),
        ("ZOOM_IN", "zoom-in", "zoomIn"),
        ("RECENTER", "fit", "resetView"),
        ("RESET", "reset-layout", "resetLayout"),
    ] {
        page = page.replace(
            &format!("__{slot}__"),
            if calls { call } else { dependency },
        );
    }
    for (slot, content) in [
        ("VIEW_STYLE", style),
        ("SHARED_STYLE", include_str!("explorer_controls.css")),
        ("NAVIGATOR", navigator),
        ("CANVAS", canvas),
        ("DETAILS", details),
        ("VIEW_SCRIPT", script.as_str()),
    ] {
        page = page.replace(&format!("__{slot}__"), content);
    }
    page = page.replace(
        "__SHARED_SCRIPT__",
        &format!(
            "{}\n{}\n{}",
            include_str!("explorer_interactions.js"),
            include_str!("explorer_runtime.js"),
            include_str!("explorer_languages.js")
        ),
    );
    page.replace("__CODEGRAIDE_GRAPH_DATA__", data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_tools_use_one_offline_shell_without_unexpanded_slots() {
        for view in [ExplorerView::Dependencies, ExplorerView::Calls] {
            let html = render(view, "{}");
            assert_eq!(html.matches("class=\"explorer-header\"").count(), 1);
            assert_eq!(html.matches("class=\"graph-toolbar\"").count(), 1);
            assert!(html.contains("function bindResizer"));
            assert!(html.contains("function bindDepth"));
            assert!(!html.contains("__CODEGRAIDE_"));
            assert!(!html.contains("__NAVIGATOR__"));
            assert!(!html.contains("<script src="));
            assert!(!html.contains("fetch("));
        }
    }

    #[test]
    fn graph_text_is_inserted_after_asset_slots() {
        let payload = r#"{"name":"__VIEW_SCRIPT__ __TITLE__"}"#;
        let html = render(ExplorerView::Calls, payload);
        assert!(html.contains(payload));
    }
}
