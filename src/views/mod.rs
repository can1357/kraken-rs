pub(crate) mod commit_detail;
pub(crate) mod diff;
pub(crate) mod graph;
mod overlays;
pub(crate) mod palette;
mod preferences;
pub(crate) mod shell;
mod terminal;
mod welcome;
pub(crate) mod wip;

use crate::{
    app::state::{AppState, MainView},
    ui::{
        Rect, Scene, Theme,
        geometry::{CONTENT_TOP, CHROME_HEIGHT, STATUS_BAR_HEIGHT},
        px,
    },
};

/// Persistent panel rectangles calculated once per immediate frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    pub(crate) tabs: Rect,
    pub(crate) toolbar: Rect,
    pub(crate) sidebar: Rect,
    pub(crate) center: Rect,
    pub(crate) terminal: Option<Rect>,
    pub(crate) detail: Option<Rect>,
    pub(crate) status: Rect,
}

impl Layout {
    /// Resolves shell regions from current splitter positions and view state.
    pub(crate) fn for_state(state: &AppState) -> Self {
        let width = px(state.width);
        let height = px(state.height);
        let chrome_height = CHROME_HEIGHT;
        let status_height = STATUS_BAR_HEIGHT;
        let sidebar_width = if state.settings.sidebar_collapsed {
            shell::SIDEBAR_RAIL_WIDTH
        } else {
            state.sidebar_width
        };
        let welcome = state
            .tabs
            .get(state.active_tab)
            .is_some_and(|tab| tab.path.is_none());
        let content_top = if welcome { chrome_height } else { CONTENT_TOP };
        let content_height = (height - content_top - status_height).max(0.0);
        let show_detail = !welcome
            && (state.selected_commit.is_some()
                || matches!(state.main_view, MainView::Wip | MainView::Diff));
        let detail_width = if show_detail {
            state
                .detail_width
                .min((width - sidebar_width - 320.0).max(0.0))
        } else {
            0.0
        };
        let detail = (detail_width > 0.0).then(|| {
            Rect::new(
                width - detail_width,
                content_top,
                detail_width,
                content_height,
            )
        });
        let center_x = if welcome { 0.0 } else { sidebar_width };
        let center_width = if welcome {
            width
        } else {
            (width - sidebar_width - detail_width).max(0.0)
        };
        let terminal = (!welcome && state.terminal_open).then(|| {
            let font_size = f32::from(state.settings.terminal_font_size.max(8));
            let minimum = (font_size * 1.2 * 3.0 + 24.0).min(content_height);
            let maximum = (content_height * 0.8).max(minimum);
            let pane_height =
                (content_height * state.terminal_height_fraction).clamp(minimum, maximum);
            Rect::new(
                center_x,
                content_top + content_height - pane_height,
                center_width,
                pane_height,
            )
        });
        let center_height = content_height - terminal.map_or(0.0, |rect| rect.height);
        Self {
            tabs: Rect::new(0.0, 0.0, width, chrome_height),
            // Zero-height anchor rect: popup menus position against its
            // bottom edge without the strip owning a second row.
            toolbar: Rect::new(0.0, chrome_height, width, 0.0),
            sidebar: Rect::new(0.0, content_top, sidebar_width, content_height),
            center: Rect::new(center_x, content_top, center_width, center_height),
            detail,
            terminal,
            status: Rect::new(0.0, height - status_height, width, status_height),
        }
    }
}

/// Builds one complete frame from immutable application state.
pub(crate) fn build_scene(state: &AppState, theme: &Theme) -> Scene {
    let mut scene = Scene::new(state.width, state.height);
    if state.preferences_open {
        preferences::build(&mut scene, state, theme);
        overlays::build_global_feedback(&mut scene, state, theme);
        return scene;
    }

    let layout = Layout::for_state(state);
    shell::build(&mut scene, state, theme, &layout);
    if state
        .tabs
        .get(state.active_tab)
        .is_some_and(|tab| tab.path.is_none())
    {
        welcome::build(&mut scene, state, theme, layout.center);
    } else {
        match state.main_view {
            MainView::Graph | MainView::Wip => {
                graph::build(&mut scene, state, theme, layout.center)
            }
            MainView::Diff => diff::build(&mut scene, state, theme, layout.center),
        }
        if let Some(terminal_rect) = layout.terminal {
            terminal::build(&mut scene, state, theme, terminal_rect);
        }
    }
    if let Some(detail_rect) = layout.detail {
        if detail_shows_wip(state) {
            wip::build(&mut scene, state, theme, detail_rect);
        } else {
            commit_detail::build(&mut scene, state, theme, detail_rect);
        }
    }
    overlays::build(&mut scene, state, theme, &layout);
    scene
}

/// True when the right detail panel hosts the WIP staging panel rather than
/// the committed-detail view; mirrors the `build_scene` panel choice so input
/// routing and rendering always agree.
pub(crate) fn detail_shows_wip(state: &AppState) -> bool {
    let working_diff = state.selected_file.as_ref().is_some_and(|request| {
        matches!(
            request.scope,
            crate::git::models::DiffScope::Staged | crate::git::models::DiffScope::Unstaged
        )
    });
    state.main_view == MainView::Wip || working_diff || state.selected_commit.is_none()
}
