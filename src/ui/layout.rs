//! Panel-rect and column/section layout math shared by the slab constraint
//! projection and repo automation.

use num_traits::ToPrimitive;

use crate::{
    app::state::{AppState, MainView},
    git::models::RefKind,
    ui::{
        Rect,
        action::ResizeTarget,
        geometry::{CHROME_HEIGHT, CONTENT_TOP, STATUS_BAR_HEIGHT},
        px,
    },
};

/// Width of the collapsed sidebar icon rail.
pub(crate) const SIDEBAR_RAIL_WIDTH: f32 = 44.0;

/// Persistent workspace panel rectangles derived from splitter state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Layout {
    pub(crate) sidebar: Rect,
    pub(crate) center: Rect,
    pub(crate) terminal: Option<Rect>,
    pub(crate) detail: Option<Rect>,
}

impl Layout {
    /// Resolves shell regions from current splitter positions and view state.
    pub(crate) fn for_state(state: &AppState) -> Self {
        let width = px(state.width);
        let height = px(state.height);
        let chrome_height = CHROME_HEIGHT;
        let status_height = STATUS_BAR_HEIGHT;
        // Pane preferences survive window shrinks (state keeps the dragged
        // extents); clamp them here so the effective layout always fits the
        // live viewport and growing the window restores the preference.
        let sidebar_width = if state.settings.sidebar_collapsed {
            SIDEBAR_RAIL_WIDTH
        } else {
            state.sidebar_width.min(width * 0.45)
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
                .min(width * 0.55)
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
            sidebar: Rect::new(0.0, content_top, sidebar_width, content_height),
            center: Rect::new(center_x, content_top, center_width, center_height),
            detail,
            terminal,
        }
    }
}

/// True when the right detail panel hosts the WIP staging panel rather than
/// the committed-detail view; mirrors the authored panel choice so input
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

const MESSAGE_COLUMN_MINIMUM: f32 = 220.0;
const MESSAGE_COLUMN_DRAG_MINIMUM: f32 = 80.0;
const REF_COLUMN_FLOOR: f32 = 100.0;
const GRAPH_COLUMN_FLOOR: f32 = 120.0;
const GRAPH_COLUMN_DRAG_FLOOR: f32 = 60.0;
const DATE_COLUMN_FLOOR: f32 = 110.0;
const DATE_COLUMN_WIDTH: f32 = 165.0;
const SHA_COLUMN_WIDTH: f32 = 82.0;
/// Horizontal inset from the graph column's left edge to the first lane.
pub(crate) const GRAPH_LANE_ORIGIN: f32 = 24.0;
/// Horizontal distance between adjacent graph lanes.
pub(crate) const GRAPH_LANE_SPACING: f32 = 22.0;
const GRAPH_LANE_END_PADDING: f32 = 32.0;

/// Solved rectangles of the five commit-table columns.
#[derive(Clone, Copy)]
pub(crate) struct GraphColumnLayout {
    pub(crate) refs: Rect,
    pub(crate) graph: Rect,
    pub(crate) message: Rect,
    pub(crate) date: Rect,
    pub(crate) sha: Rect,
    ref_cap: f32,
}

#[derive(Clone, Copy)]
struct GraphColumnWidths {
    refs: f32,
    graph: f32,
    message: f32,
    date: f32,
    sha: f32,
}

#[derive(Clone, Copy)]
struct GraphColumnInput {
    table_width: f32,
    ref_preference: f32,
    ref_content_width: f32,
    graph_preference: f32,
    graph_content_width: f32,
    show_date: bool,
    show_sha: bool,
    message_preference: f32,
    explicit_drag: bool,
    graph_explicit: bool,
}

fn graph_column_widths(input: GraphColumnInput) -> GraphColumnWidths {
    let GraphColumnInput {
        table_width,
        ref_preference,
        ref_content_width,
        graph_preference,
        graph_content_width,
        show_date,
        show_sha,
        message_preference,
        explicit_drag,
        graph_explicit,
    } = input;
    let graph_floor = if explicit_drag || graph_explicit {
        GRAPH_COLUMN_DRAG_FLOOR
    } else {
        GRAPH_COLUMN_FLOOR
    };
    let mut refs = ref_preference.min(ref_content_width).max(REF_COLUMN_FLOOR);
    let mut graph = if explicit_drag || graph_explicit {
        graph_preference.max(graph_floor)
    } else {
        graph_preference
            .min(graph_content_width)
            .max(GRAPH_COLUMN_FLOOR)
    };
    let mut date = if show_date { DATE_COLUMN_WIDTH } else { 0.0 };
    let sha = if show_sha { SHA_COLUMN_WIDTH } else { 0.0 };
    let requested_message = if message_preference > 0.0 {
        message_preference.max(MESSAGE_COLUMN_DRAG_MINIMUM)
    } else {
        MESSAGE_COLUMN_MINIMUM
    };

    let available = table_width - refs - graph - date - sha;
    if available < requested_message {
        if explicit_drag || graph_explicit {
            if show_date {
                date = DATE_COLUMN_FLOOR;
            }
        } else {
            graph = GRAPH_COLUMN_FLOOR;
            refs = REF_COLUMN_FLOOR;
            if show_date {
                date = DATE_COLUMN_FLOOR;
            }
        }
    }
    let available = table_width - refs - graph - date - sha;

    GraphColumnWidths {
        refs,
        graph,
        message: requested_message.min(available.max(0.0)),
        date,
        sha,
    }
}

/// Natural width of the lane graph for the deepest lane in the snapshot.
fn graph_content_width(max_lanes: usize) -> f32 {
    let lanes = max_lanes.max(1).to_f32().unwrap_or(1.0);
    GRAPH_LANE_ORIGIN + (lanes - 1.0) * GRAPH_LANE_SPACING + GRAPH_LANE_END_PADDING
}

/// Splits the commit table into ref, graph, message, date, and SHA columns.
pub(crate) fn column_layout(state: &AppState, rect: Rect) -> GraphColumnLayout {
    let ref_content_width = state.snapshot.as_ref().map_or(140.0, |snapshot| {
        snapshot
            .commits
            .iter()
            .map(|commit| {
                let branch_width = commit
                    .branch_refs
                    .iter()
                    .filter(|reference| !reference.is_tag)
                    .map(|reference| {
                        reference
                            .branch_short_name
                            .chars()
                            .count()
                            .to_f32()
                            .unwrap_or(0.0)
                            * 6.2
                            + 42.0
                            + reference.remote_names.len().to_f32().unwrap_or(0.0) * 11.0
                    })
                    .max_by(f32::total_cmp)
                    .unwrap_or(0.0);
                let tag_width = commit
                    .refs
                    .iter()
                    .filter(|label| matches!(label.kind, RefKind::Tag | RefKind::Worktree))
                    .map(|label| label.name.chars().count().to_f32().unwrap_or(0.0) * 6.2 + 24.0)
                    .sum::<f32>();
                branch_width + tag_width
            })
            .fold(0.0, f32::max)
            .clamp(140.0, 280.0)
    });
    let graph_content_width =
        graph_content_width(state.graph.max_lanes).clamp(GRAPH_COLUMN_FLOOR, 320.0);
    let explicit_drag = matches!(
        state.drag,
        Some(ResizeTarget::RefColumn | ResizeTarget::GraphColumn | ResizeTarget::MessageColumn)
    );
    let columns = graph_column_widths(GraphColumnInput {
        table_width: rect.width,
        ref_preference: state.ref_column_width,
        ref_content_width,
        graph_preference: state.graph_column_width,
        graph_content_width,
        show_date: state.settings.show_commit_date,
        show_sha: state.settings.show_commit_sha,
        message_preference: state.message_column_width,
        explicit_drag,
        graph_explicit: state.graph_column_explicit,
    });
    let refs = Rect::new(rect.x, rect.y, columns.refs, rect.height);
    let graph = Rect::new(refs.right(), rect.y, columns.graph, rect.height);
    let message = Rect::new(graph.right(), rect.y, columns.message, rect.height);
    let date = Rect::new(message.right(), rect.y, columns.date, rect.height);
    let sha = Rect::new(date.right(), rect.y, columns.sha, rect.height);
    GraphColumnLayout {
        refs,
        graph,
        message,
        date,
        sha,
        ref_cap: ref_content_width,
    }
}

/// Clamps a live column drag to the width budget the table can actually cede.
pub(crate) fn resize_preference(
    state: &AppState,
    table: Rect,
    target: ResizeTarget,
    edge_x: f32,
) -> f32 {
    let layout = column_layout(state, table);
    match target {
        ResizeTarget::RefColumn => {
            let maximum = (table.right()
                - table.x
                - layout.graph.width
                - layout.date.width
                - layout.sha.width
                - MESSAGE_COLUMN_DRAG_MINIMUM)
                .clamp(REF_COLUMN_FLOOR, layout.ref_cap);
            (edge_x - table.x).clamp(REF_COLUMN_FLOOR, maximum)
        }
        ResizeTarget::GraphColumn => {
            let maximum = (table.right()
                - layout.refs.right()
                - layout.date.width
                - layout.sha.width
                - MESSAGE_COLUMN_DRAG_MINIMUM)
                .max(GRAPH_COLUMN_DRAG_FLOOR);
            (edge_x - layout.refs.right()).clamp(GRAPH_COLUMN_DRAG_FLOOR, maximum)
        }
        ResizeTarget::MessageColumn => {
            let maximum =
                (table.right() - layout.graph.right() - layout.date.width - layout.sha.width)
                    .max(MESSAGE_COLUMN_DRAG_MINIMUM);
            (edge_x - layout.graph.right()).clamp(MESSAGE_COLUMN_DRAG_MINIMUM, maximum)
        }
        ResizeTarget::Sidebar
        | ResizeTarget::DetailPanel
        | ResizeTarget::TerminalPane
        | ResizeTarget::DetailMessage => {
            unreachable!("not a graph column")
        }
    }
}

/// Fixed height of the commit form pinned to the WIP panel bottom.
pub(crate) const WIP_COMMIT_FORM_HEIGHT: f32 = 274.0;
/// Height of one file row in either WIP list.
pub(crate) const WIP_ROW_HEIGHT: f32 = 24.0;
/// Height of one pinned WIP section header.
pub(crate) const WIP_HEADER_HEIGHT: f32 = 28.0;
/// Content height reserved for an empty WIP section's placeholder line.
pub(crate) const WIP_EMPTY_CONTENT_HEIGHT: f32 = 29.0;

/// Fixed geometry for the two independently scrolled WIP file lists.
pub(crate) struct WipSectionLayout {
    pub(crate) unstaged_view: Rect,
    pub(crate) staged_view: Rect,
}

/// Splits the list area between the Unstaged and Staged sections.
///
/// Rule: each list is capped at half the available height; a list whose
/// content needs less than half keeps its natural height and cedes the
/// surplus to the other, so a tiny section never wastes list space.
pub(crate) fn wip_section_layout(state: &AppState, panel: Rect) -> WipSectionLayout {
    let (unstaged_count, staged_count) = state.snapshot.as_ref().map_or((0, 0), |snapshot| {
        (
            snapshot
                .working
                .files
                .iter()
                .filter(|file| file.unstaged.is_some())
                .count(),
            snapshot
                .working
                .files
                .iter()
                .filter(|file| file.staged.is_some())
                .count(),
        )
    });
    let commit_height = WIP_COMMIT_FORM_HEIGHT.min(panel.height * 0.43);
    let list = Rect::new(
        panel.x + 1.0,
        panel.y + 60.0,
        panel.width - 2.0,
        (panel.height - 60.0 - commit_height).max(0.0),
    );
    let content = |count: usize| {
        if count == 0 {
            WIP_EMPTY_CONTENT_HEIGHT
        } else {
            count.to_f32().unwrap_or(0.0) * WIP_ROW_HEIGHT
        }
    };
    let unstaged_content = content(unstaged_count);
    let staged_content = content(staged_count);
    // 9px top padding + two pinned headers + 8px gap between the sections.
    let available = (list.height - 2.0 * WIP_HEADER_HEIGHT - 17.0).max(0.0);
    let half = (available * 0.5).floor();
    let (unstaged_height, staged_height) = if unstaged_content <= half && staged_content <= half {
        (unstaged_content, staged_content)
    } else if unstaged_content <= half {
        (
            unstaged_content,
            (available - unstaged_content).min(staged_content),
        )
    } else if staged_content <= half {
        (
            (available - staged_content).min(unstaged_content),
            staged_content,
        )
    } else {
        (half, half)
    };
    let unstaged_header = Rect::new(list.x, list.y + 9.0, list.width, WIP_HEADER_HEIGHT);
    let unstaged_view = Rect::new(
        list.x,
        unstaged_header.bottom(),
        list.width,
        unstaged_height,
    );
    let staged_header = Rect::new(
        list.x,
        unstaged_view.bottom() + 8.0,
        list.width,
        WIP_HEADER_HEIGHT,
    );
    let staged_view = Rect::new(list.x, staged_header.bottom(), list.width, staged_height);
    WipSectionLayout {
        unstaged_view,
        staged_view,
    }
}

/// Height of one rendered diff row in every diff presentation.
pub(crate) const DIFF_ROW_HEIGHT: f32 = 20.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_column_keeps_priority_in_narrow_detail_layout() {
        let narrow = graph_column_widths(GraphColumnInput {
            table_width: 650.0,
            ref_preference: 440.0,
            ref_content_width: 140.0,
            graph_preference: 410.0,
            graph_content_width: 120.0,
            show_date: true,
            show_sha: true,
            message_preference: 0.0,
            explicit_drag: false,
            graph_explicit: false,
        });
        assert!((narrow.graph - GRAPH_COLUMN_FLOOR).abs() < f32::EPSILON);
        assert!((narrow.refs - REF_COLUMN_FLOOR).abs() < f32::EPSILON);
        assert!((narrow.date - DATE_COLUMN_FLOOR).abs() < f32::EPSILON);
        assert!(narrow.message >= MESSAGE_COLUMN_MINIMUM);

        let wide = graph_column_widths(GraphColumnInput {
            table_width: 1340.0,
            ref_preference: 440.0,
            ref_content_width: 140.0,
            graph_preference: 410.0,
            graph_content_width: 120.0,
            show_date: true,
            show_sha: true,
            message_preference: 0.0,
            explicit_drag: false,
            graph_explicit: false,
        });
        assert!(wide.message >= MESSAGE_COLUMN_MINIMUM);
    }

    #[test]
    fn dragged_message_column_never_rests_below_its_minimum() {
        let columns = graph_column_widths(GraphColumnInput {
            table_width: 650.0,
            ref_preference: 440.0,
            ref_content_width: 140.0,
            graph_preference: 410.0,
            graph_content_width: 120.0,
            show_date: true,
            show_sha: true,

            message_preference: 20.0,
            explicit_drag: false,
            graph_explicit: false,
        });
        assert!(columns.message >= MESSAGE_COLUMN_DRAG_MINIMUM);
    }
}
