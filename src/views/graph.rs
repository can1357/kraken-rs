use chrono::{DateTime, Local, Utc};
use num_traits::ToPrimitive;

use crate::{
    app::state::{AppState, FocusField, MainView, RefDrag},
    git::models::{CommitBranchRef, RefKind, RefLabel, WorktreeInfo},
    graph::avatars,
    ui::{
        Color, FontFace, RADIUS_LG, RADIUS_SM, RADIUS_XL, Rect, Scene, Theme,
        action::{CursorHint, ResizeTarget, ScrollTarget, UiAction},
        geometry::{COMMIT_HEADER_HEIGHT as HEADER_HEIGHT, COMMIT_ROW_HEIGHT as ROW_HEIGHT},
        icons,
        widgets::{modal_text_input, scrollbar, truncated_text},
    },
};

const MESSAGE_COLUMN_MINIMUM: f32 = 220.0;
const MESSAGE_COLUMN_DRAG_MINIMUM: f32 = 80.0;
const REF_COLUMN_FLOOR: f32 = 100.0;
const GRAPH_COLUMN_FLOOR: f32 = 120.0;
const GRAPH_COLUMN_DRAG_FLOOR: f32 = 60.0;
const DATE_COLUMN_FLOOR: f32 = 110.0;
const DATE_COLUMN_WIDTH: f32 = 165.0;
const FULL_TIMESTAMP_MINIMUM_WIDTH: f32 = 150.0;
const SHA_COLUMN_WIDTH: f32 = 82.0;
const GRAPH_LANE_ORIGIN: f32 = 24.0;
const GRAPH_LANE_SPACING: f32 = 22.0;
const GRAPH_LANE_END_PADDING: f32 = 32.0;
const GRAPH_TRAIL_LIFT_WIDTH: f32 = 32.0;
const GRAPH_TRAIL_SHADOW_WIDTH: f32 = 12.0;
/// Diameter of an avatar commit node on its lane.
const AVATAR_NODE_DIAMETER: f32 = 20.0;
/// Diameter of a merge-commit (or avatar-less) lane dot.
const MERGE_DOT_DIAMETER: f32 = 8.0;

#[derive(Clone, Copy)]
pub(crate) struct GraphColumnLayout {
    pub(crate) refs: Rect,
    pub(crate) graph: Rect,
    pub(crate) message: Rect,
    date: Rect,
    sha: Rect,
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
        | ResizeTarget::SidebarSection(_)
        | ResizeTarget::DetailPanel
        | ResizeTarget::TerminalPane
        | ResizeTarget::DetailMessage => {
            unreachable!("not a graph column")
        }
    }
}

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.window);
    let columns = column_layout(state, rect);
    let ref_rect = columns.refs;
    let graph_rect = columns.graph;
    let message_rect = columns.message;
    let date_rect = columns.date;
    let sha_rect = columns.sha;
    let date_width = date_rect.width;
    let sha_width = sha_rect.width;

    let body = Rect::new(
        rect.x,
        rect.y + HEADER_HEIGHT,
        rect.width,
        rect.height - HEADER_HEIGHT,
    );
    if let Some(snapshot) = &state.snapshot {
        let current_wip = usize::from(snapshot.working.is_dirty());
        let wip_offset = snapshot.wip_rows();
        let mut wip_index = 0;
        if current_wip == 1 {
            draw_wip_row(scene, state, theme, body, graph_rect, snapshot, wip_index);
            wip_index += 1;
        }
        for worktree in snapshot
            .worktrees
            .iter()
            .filter(|worktree| worktree.changes > 0)
        {
            draw_worktree_wip_row(
                scene, state, theme, body, ref_rect, graph_rect, snapshot, worktree, wip_index,
            );
            wip_index += 1;
        }
        let commit_scroll =
            (state.graph_scroll - wip_offset.to_f32().unwrap_or(0.0) * ROW_HEIGHT).max(0.0);
        let visible = state
            .graph
            .visible_range(commit_scroll, body.height, ROW_HEIGHT);
        let first_commit = visible.start.saturating_sub(1);
        let last_commit = visible.end;
        let (lane_origin, lane_spacing) = lane_geometry(graph_rect);
        let graph_clip = graph_rect.intersection(body).unwrap_or(graph_rect);
        let search_results = state.search_results();
        let hovered_reach = hovered_branch_reach(
            state,
            snapshot,
            body,
            ref_rect,
            first_commit..last_commit,
            wip_offset,
        );

        for index in first_commit..last_commit {
            let Some(commit) = snapshot.commits.get(index) else {
                continue;
            };
            let Some(layout) = state.graph.rows.get(index) else {
                continue;
            };
            let display_index = index.saturating_add(wip_offset);
            let y =
                body.y + display_index.to_f32().unwrap_or(0.0) * ROW_HEIGHT - state.graph_scroll;
            let row = Rect::new(body.x, y, body.width, ROW_HEIGHT);
            if row.bottom() < body.y || row.y > body.bottom() {
                continue;
            }
            let selected = state.selected_commit.as_deref() == Some(commit.id.as_str());
            let matched = search_results.binary_search(&index).is_ok();
            let current_match = search_results
                .get(state.search_cursor)
                .is_some_and(|result| *result == index);
            let node_x = lane_origin + layout.lane.to_f32().unwrap_or(0.0) * lane_spacing;
            let node_color = theme.graph_lanes[layout.color % theme.graph_lanes.len()];
            let band = Rect::new(
                message_rect.x,
                row.y,
                (body.right() - message_rect.x).max(0.0),
                row.height,
            );
            let trail = graph_trail(row, graph_clip, node_x);
            if selected {
                scene.rect(1, row, body, theme.row_selected);
                // The selection cursor carries a white leading edge.
                scene.rect(
                    2,
                    Rect::new(row.x, row.y, 2.0, row.height),
                    body,
                    theme.accent,
                );
            }
            if current_match {
                scene.rect(1, band, body, theme.yellow_muted);
            } else if matched {
                scene.rect(1, band, body, theme.yellow_muted.with_alpha(0.6));
            } else if !selected && row.contains(state.hover()) {
                scene.rect(1, band, body, theme.row_hover);
            }
            draw_graph_trail(scene, trail, graph_clip, node_color);
            if hovered_reach
                .as_ref()
                .is_some_and(|reach| reach.contains(&index))
            {
                scene.rect(1, trail, graph_clip, theme.accent.with_alpha(0.018));
            }

            let from_y = y + ROW_HEIGHT * 0.5;
            let to_y = from_y + ROW_HEIGHT;
            scene.rect(
                1,
                Rect::new(
                    message_rect.x - 1.0,
                    from_y - ROW_HEIGHT * 0.36,
                    3.5,
                    ROW_HEIGHT * 0.72,
                ),
                body,
                node_color,
            );
            for segment in &layout.segments {
                let from_x = lane_origin + segment.from.to_f32().unwrap_or(0.0) * lane_spacing;
                let to_x = lane_origin + segment.to.to_f32().unwrap_or(0.0) * lane_spacing;
                let color = theme.graph_lanes[segment.color % theme.graph_lanes.len()];
                if segment.from == segment.to {
                    scene.line(1, [from_x, from_y], [to_x, to_y], 1.5, color, graph_clip);
                } else {
                    scene.rounded_elbow(
                        1,
                        [from_x, from_y],
                        [to_x, to_y],
                        5.0,
                        1.5,
                        color,
                        graph_clip,
                    );
                }
            }
            // Every non-merge commit carries its author avatar; merge commits
            // and the avatars-off setting use a small solid lane dot.
            let avatar_node = state.settings.show_commit_author && commit.parents.len() < 2;
            let diameter = if avatar_node {
                AVATAR_NODE_DIAMETER
            } else {
                MERGE_DOT_DIAMETER
            };
            let node = Rect::new(
                node_x - diameter * 0.5,
                from_y - diameter * 0.5,
                diameter,
                diameter,
            );
            if avatar_node {
                scene.rounded_rect(
                    2,
                    node,
                    graph_clip,
                    theme.panel,
                    node_color,
                    diameter * 0.5,
                    2.0,
                );
                scene.image(
                    2,
                    node.inset(2.0),
                    graph_clip,
                    avatars::request(&commit.email),
                );
            } else {
                scene.rounded_rect(
                    2,
                    node,
                    graph_clip,
                    node_color,
                    node_color,
                    diameter * 0.5,
                    0.0,
                );
            }

            scene.hit_clipped(
                row,
                body,
                UiAction::SelectCommit(commit.id.clone()),
                CursorHint::Pointer,
                None,
            );
            draw_ref_chips(
                scene,
                theme,
                body,
                row,
                ref_rect,
                &commit.branch_refs,
                &commit.refs,
                state.hover(),
                state.dragging_ref(),
            );
            // The node replaced the in-column avatar; the message hugs the
            // column edge and reveals its full text on hover when truncated.
            let subject_color = if selected {
                theme.accent
            } else {
                theme.text_muted
            };
            let message_bounds = column_text_bounds(message_rect, row, body);
            truncated_text(
                scene,
                &commit.subject,
                [message_rect.x + 8.0, row.y + 5.0],
                message_bounds,
                body,
                subject_color,
                11.5,
                ROW_HEIGHT - 2.0,
                FontFace::Sans,
            );
            // GitKraken appends the first body line dimmed after the subject.
            if !commit.description.is_empty() {
                let subject_advance =
                    commit.subject.chars().count().to_f32().unwrap_or(0.0) * 11.5 * 0.52;
                let description_x = message_rect.x + 8.0 + subject_advance + 14.0;
                if description_x < message_bounds.right() - 24.0 {
                    truncated_text(
                        scene,
                        &commit.description,
                        [description_x, row.y + 6.0],
                        Rect::new(
                            description_x,
                            message_bounds.y,
                            (message_bounds.right() - description_x).max(0.0),
                            message_bounds.height,
                        ),
                        body,
                        theme.text_dim,
                        10.5,
                        ROW_HEIGHT - 2.0,
                        FontFace::Sans,
                    );
                }
            }
            if date_width > 0.0 {
                let date_bounds = column_text_bounds(date_rect, row, body);
                scene.text(
                    format_time_for_width(commit.authored_seconds, date_bounds.width),
                    [date_rect.x + 7.0, row.y + 5.0],
                    date_bounds,
                    theme.text_dim,
                    10.5,
                    ROW_HEIGHT - 2.0,
                    FontFace::Monospace,
                );
            }
            if sha_width > 0.0 {
                scene.text(
                    &commit.short_id,
                    [sha_rect.x + 7.0, row.y + 5.0],
                    column_text_bounds(sha_rect, row, body),
                    theme.text_dim,
                    10.5,
                    ROW_HEIGHT - 2.0,
                    FontFace::Monospace,
                );
            }
        }
        if snapshot.commits.is_empty() {
            empty_state(scene, theme, body, "This repository has no commits");
        } else if state.loading_history {
            scene.text(
                format!("{}  Loading older commits…", icons::LOADING),
                [body.x + body.width * 0.5 - 80.0, body.bottom() - 30.0],
                Rect::new(body.x, body.bottom() - 34.0, body.width, 28.0),
                theme.accent,
                12.0,
                16.0,
                FontFace::Sans,
            );
        }
        let content_height = state
            .graph
            .rows
            .len()
            .saturating_add(wip_offset)
            .to_f32()
            .unwrap_or(f32::MAX)
            * ROW_HEIGHT;
        scrollbar(
            scene,
            body,
            content_height,
            state.graph_scroll,
            ScrollTarget::Graph,
            theme,
        );
    } else {
        let center = [body.x + body.width * 0.5, body.y + body.height * 0.5 - 8.0];
        scene.rounded_rect(
            1,
            Rect::new(center[0] - 38.0, center[1] - 38.0, 76.0, 76.0),
            body,
            theme.panel_alt,
            theme.accent_soft,
            RADIUS_XL,
            2.0,
        );
        scene.rounded_rect(
            2,
            Rect::new(center[0] - 26.0, center[1] - 26.0, 52.0, 52.0),
            body,
            theme.window,
            theme.accent,
            RADIUS_LG,
            2.0,
        );
        scene.text(
            icons::REPOSITORY,
            [center[0] - 13.0, center[1] - 17.0],
            Rect::new(center[0] - 22.0, center[1] - 22.0, 44.0, 44.0),
            theme.accent,
            25.0,
            30.0,
            FontFace::Sans,
        );
        empty_state(scene, theme, body, "Opening repo");
    }
    // Draw the header after virtualized rows so its opaque background clips any
    // scrolled graph content before it can enter the column-label strip.
    build_header(
        scene,
        state,
        theme,
        rect,
        ref_rect,
        graph_rect,
        message_rect,
        date_rect,
        sha_rect,
    );
}

#[allow(clippy::too_many_arguments)]
fn build_header(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    rect: Rect,
    refs: Rect,
    graph: Rect,
    message: Rect,
    date: Rect,
    sha: Rect,
) {
    let header = Rect::new(rect.x, rect.y, rect.width, HEADER_HEIGHT);
    scene.rect(1, header, rect, theme.panel_alt);
    scene.text(
        "BRANCH / TAG",
        [refs.x + 8.0, refs.y + 6.0],
        refs,
        theme.text_dim,
        11.0,
        13.0,
        FontFace::SansMedium,
    );
    scene.text(
        "GRAPH",
        [graph.x + 8.0, graph.y + 6.0],
        graph,
        theme.text_dim,
        11.0,
        13.0,
        FontFace::SansMedium,
    );
    scene.text(
        "COMMIT MESSAGE",
        [message.x + 8.0, message.y + 6.0],
        message,
        theme.text_dim,
        11.0,
        13.0,
        FontFace::SansMedium,
    );
    if date.width > 0.0 {
        scene.text(
            "COMMIT DATE / TIME",
            [date.x + 8.0, date.y + 6.0],
            date,
            theme.text_dim,
            11.0,
            13.0,
            FontFace::SansMedium,
        );
    }
    if sha.width > 0.0 {
        scene.text(
            format!("SHA   {}", icons::GEAR),
            [sha.x + 8.0, sha.y + 6.0],
            sha,
            theme.text_dim,
            11.0,
            13.0,
            FontFace::SansMedium,
        );
    }
    scene.rect(
        1,
        Rect::new(header.x, header.bottom() - 1.0, header.width, 1.0),
        rect,
        theme.border,
    );
    for (x, target) in [
        (refs.right(), ResizeTarget::RefColumn),
        (graph.right(), ResizeTarget::GraphColumn),
        (message.right(), ResizeTarget::MessageColumn),
    ] {
        let divider = Rect::new(x - 2.0, rect.y, 4.0, rect.height);
        if divider.contains(state.hover()) {
            scene.rect(3, divider, rect, theme.accent);
        }
        scene.hit(
            divider,
            UiAction::BeginResize(target),
            CursorHint::ResizeHorizontal,
            None,
        );
    }
    if state.focus == FocusField::Search || !state.search.is_empty() {
        let search = Rect::new(
            (rect.right() - 332.0).max(rect.x + 10.0),
            header.bottom() + 8.0,
            324.0,
            32.0,
        );
        let input = Rect::new(
            search.x + 8.0,
            search.y,
            search.width - 140.0,
            search.height,
        );
        let result_label = Rect::new(search.right() - 128.0, search.y, 54.0, search.height);
        let previous = Rect::new(search.right() - 66.0, search.y + 4.0, 18.0, 24.0);
        let next = Rect::new(search.right() - 44.0, search.y + 4.0, 18.0, 24.0);
        let close = Rect::new(search.right() - 22.0, search.y + 4.0, 18.0, 24.0);

        // This panel intentionally owns the graph body beneath it: row labels,
        // row hits, and graph nodes must not bleed through its opaque surface.
        scene.mask_hits(search);
        scene.mask_text(search);
        scene.shadow(3, search, scene.viewport(), RADIUS_LG);
        scene.rounded_rect(3, search, rect, theme.surface_3, theme.border, RADIUS_LG, 1.0);

        let results = state.search_results();
        modal_text_input(
            scene,
            input,
            &state.search,
            "Search commits",
            state.focus == FocusField::Search,
            UiAction::FocusSearch,
            state.hover(),
            theme,
            false,
        );
        scene.text(
            format!("{} RESULTS", results.len()),
            [result_label.x, search.y + 9.0],
            result_label,
            theme.text_dim,
            9.5,
            14.0,
            FontFace::Monospace,
        );
        for (button, label) in [
            (previous, icons::ARROW_UP),
            (next, icons::ARROW_DOWN),
            (close, icons::CLOSE),
        ] {
            if button.contains(state.hover()) {
                scene.rounded_rect(
                    3,
                    button,
                    search,
                    theme.row_hover,
                    theme.row_hover,
                    RADIUS_SM,
                    0.0,
                );
            }
            scene.text(
                label,
                [button.x + 5.0, button.y + 3.0],
                button,
                theme.text_muted,
                13.0,
                18.0,
                FontFace::Sans,
            );
        }
        scene.hit(
            input,
            UiAction::FocusSearch,
            CursorHint::Text,
            Some("Search commits"),
        );
        scene.hit(
            previous,
            UiAction::PreviousSearchResult,
            CursorHint::Pointer,
            Some("Previous result"),
        );
        scene.hit(
            next,
            UiAction::NextSearchResult,
            CursorHint::Pointer,
            Some("Next result"),
        );
        scene.hit(
            close,
            UiAction::CloseSearch,
            CursorHint::Pointer,
            Some("Close search"),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_wip_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    body: Rect,
    graph: Rect,
    snapshot: &crate::git::models::RepoSnapshot,
    index: usize,
) {
    let y = body.y + index.to_f32().unwrap_or(0.0) * ROW_HEIGHT - state.graph_scroll;
    let row = Rect::new(body.x, y, body.width, ROW_HEIGHT);
    if row.bottom() < body.y || row.y > body.bottom() {
        return;
    }
    let message_x = graph.right();
    let selected = state.main_view == MainView::Wip && state.selected_commit.is_none();
    if selected {
        scene.rect(1, row, body, theme.row_selected);
    }
    let (lane_origin, lane_spacing) = lane_geometry(graph);
    let head_layout = snapshot.head_id.as_ref().and_then(|head_id| {
        snapshot
            .commits
            .iter()
            .position(|commit| commit.id == *head_id)
            .and_then(|commit| state.graph.rows.get(commit))
    });
    let lane = head_layout.map_or(0, |layout| layout.lane);
    let node_color = head_layout.map_or(theme.graph_lanes[0], |layout| {
        theme.graph_lanes[layout.color % theme.graph_lanes.len()]
    });
    let node_x = lane_origin + lane.to_f32().unwrap_or(0.0) * lane_spacing;
    let graph_clip = graph.intersection(body).unwrap_or(graph);
    let center_y = y + ROW_HEIGHT * 0.5;
    let trail = graph_trail(row, graph_clip, node_x);
    draw_graph_trail(scene, trail, graph_clip, node_color);
    scene.line(
        1,
        [node_x, center_y],
        [message_x, center_y],
        1.5,
        node_color.with_alpha(0.9),
        body,
    );
    // Uncommitted work hangs above HEAD: dashed lane-colored stem plus a
    // dashed hollow circle node (GitKraken's WIP marker).
    let mut stem_y = center_y + 10.0;
    while stem_y < y + ROW_HEIGHT * 1.5 {
        scene.line(
            1,
            [node_x, stem_y],
            [node_x, (stem_y + 5.0).min(y + ROW_HEIGHT * 1.5)],
            1.5,
            node_color,
            graph_clip,
        );
        stem_y += 9.0;
    }
    let radius = 10.0;
    let dashes = 12;
    for dash in 0..dashes {
        if dash % 2 == 1 {
            continue;
        }
        let start =
            (dash.to_f32().unwrap_or(0.0)) / dashes.to_f32().unwrap_or(1.0) * std::f32::consts::TAU;
        let end = start + std::f32::consts::TAU / dashes.to_f32().unwrap_or(1.0);
        scene.line(
            1,
            [
                node_x + radius * start.cos(),
                center_y + radius * start.sin(),
            ],
            [node_x + radius * end.cos(), center_y + radius * end.sin()],
            2.0,
            node_color,
            graph_clip,
        );
    }
    // Boxed `// WIP` placeholder chip plus change-kind counters.
    let chip = Rect::new(message_x + 8.0, y + 3.5, 82.0, ROW_HEIGHT - 7.0);
    scene.rounded_rect(
        2,
        chip,
        body,
        theme.purple_muted,
        theme.purple.with_alpha(0.35),
        RADIUS_SM,
        1.0,
    );
    scene.text(
        "// WIP",
        [chip.x + 10.0, chip.y + 4.0],
        clipped_text_bounds(chip.inset(2.0), body),
        theme.purple,
        11.0,
        14.0,
        FontFace::Monospace,
    );
    let mut modified = 0usize;
    let mut added = 0usize;
    for file in &snapshot.working.files {
        let kind = file.staged.or(file.unstaged);
        if matches!(kind, Some(crate::git::models::ChangeKind::Added)) {
            added += 1;
        } else if kind.is_some() {
            modified += 1;
        }
    }
    let mut counter_x = chip.right() + 14.0;
    if modified > 0 {
        scene.text(
            format!("{} {modified}", icons::DIFF_MODIFIED),
            [counter_x, y + 5.0],
            clipped_text_bounds(Rect::new(counter_x, y, 64.0, ROW_HEIGHT), body),
            theme.text_dim,
            11.5,
            15.0,
            FontFace::Sans,
        );
        counter_x += 46.0;
    }
    if added > 0 {
        scene.text(
            format!("{} {added}", icons::DIFF_ADDED),
            [counter_x, y + 5.0],
            clipped_text_bounds(Rect::new(counter_x, y, 64.0, ROW_HEIGHT), body),
            theme.text_dim,
            11.5,
            15.0,
            FontFace::Sans,
        );
    }
    scene.hit_clipped(row, body, UiAction::SelectWip, CursorHint::Pointer, None);
}

#[allow(clippy::too_many_arguments)]
fn draw_worktree_wip_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    body: Rect,
    refs: Rect,
    graph: Rect,
    snapshot: &crate::git::models::RepoSnapshot,
    worktree: &WorktreeInfo,
    index: usize,
) {
    let y = body.y + index.to_f32().unwrap_or(0.0) * ROW_HEIGHT - state.graph_scroll;
    let row = Rect::new(body.x, y, body.width, ROW_HEIGHT);
    if row.bottom() < body.y || row.y > body.bottom() {
        return;
    }
    let selected = state.selected_commit.as_deref() == worktree.target.as_deref();
    if selected {
        scene.rect(1, row, body, theme.row_selected);
    }
    scene.rect(1, row, body, theme.purple.with_alpha(0.10));
    let chip = Rect::new(refs.x + 8.0, y + 4.0, (refs.width - 16.0).min(190.0), 18.0);
    scene.rounded_rect(
        2,
        chip,
        body,
        theme.purple_muted,
        theme.purple.with_alpha(0.35),
        RADIUS_SM,
        1.0,
    );
    scene.text(
        format!(
            "{}  {}",
            icons::WORKSPACE,
            worktree
                .branch
                .as_deref()
                .unwrap_or(worktree.name.as_str())
                .to_uppercase()
        ),
        [chip.x + 6.0, chip.y + 3.0],
        clipped_text_bounds(chip.inset(2.0), body),
        theme.purple,
        10.0,
        12.0,
        FontFace::Monospace,
    );
    let (lane_origin, lane_spacing) = lane_geometry(graph);
    let lane = worktree
        .target
        .as_ref()
        .and_then(|target| {
            snapshot
                .commits
                .iter()
                .position(|commit| commit.id == *target)
                .and_then(|commit| state.graph.rows.get(commit))
        })
        .map_or(0, |layout| layout.lane);
    let node_x = lane_origin + lane.to_f32().unwrap_or(0.0) * lane_spacing;
    let graph_clip = graph.intersection(body).unwrap_or(graph);
    scene.line(
        1,
        [node_x, y + ROW_HEIGHT * 0.5],
        [node_x, y + ROW_HEIGHT * 1.5],
        1.5,
        theme.purple,
        graph_clip,
    );
    scene.rounded_rect(
        2,
        Rect::new(node_x - 6.0, y + 7.0, 12.0, 12.0),
        graph_clip,
        theme.purple,
        theme.purple,
        3.0,
        0.0,
    );
    scene.text(
        format!(
            "// WIP  {} {}  ·  {}",
            icons::DIFF_ADDED,
            worktree.changes,
            worktree.name
        ),
        [graph.x + 48.0, y + 5.0],
        clipped_text_bounds(
            Rect::new(graph.x + 44.0, y, graph.width - 48.0, ROW_HEIGHT),
            body,
        ),
        theme.purple,
        11.0,
        15.0,
        FontFace::Monospace,
    );
    if let Some(target) = &worktree.target {
        scene.hit_clipped(
            row,
            body,
            UiAction::SelectCommit(target.clone()),
            CursorHint::Pointer,
            Some("View linked worktree HEAD"),
        );
    }
}
fn graph_content_width(max_lanes: usize) -> f32 {
    let lanes = max_lanes.max(1).to_f32().unwrap_or(1.0);
    GRAPH_LANE_ORIGIN + (lanes - 1.0) * GRAPH_LANE_SPACING + GRAPH_LANE_END_PADDING
}

fn lane_geometry(graph: Rect) -> (f32, f32) {
    (graph.x + GRAPH_LANE_ORIGIN, GRAPH_LANE_SPACING)
}

fn graph_trail(row: Rect, graph: Rect, node_x: f32) -> Rect {
    Rect::new(
        node_x,
        row.y + 2.0,
        (graph.right() - node_x).max(0.0),
        (row.height - 4.0).max(0.0),
    )
}

fn draw_graph_trail(scene: &mut Scene, trail: Rect, graph: Rect, color: Color) {
    let lift_x = (trail.right() - GRAPH_TRAIL_LIFT_WIDTH).max(trail.x);
    let base = Rect::new(trail.x, trail.y, lift_x - trail.x, trail.height);
    let lifted = Rect::new(lift_x, trail.y, trail.right() - lift_x, trail.height);
    scene.rect(1, base, graph, color.with_alpha(0.05));
    scene.rect(1, lifted, graph, color.with_alpha(0.06));

    // The short terminal slab sits above the long run. Its left edge casts
    // a narrow shadow back across the lower slab before the colored end cap.
    let shadow_width = GRAPH_TRAIL_SHADOW_WIDTH.min(base.width);
    scene.gradient_rect_h(
        1,
        Rect::new(lift_x - shadow_width, trail.y, shadow_width, trail.height),
        graph,
        Color::rgb(0, 0, 0).with_alpha(0.0),
        Color::rgb(0, 0, 0).with_alpha(0.28),
    );
    scene.rect(
        1,
        Rect::new(lift_x, trail.y, 1.0, trail.height),
        graph,
        Color::rgb(0, 0, 0).with_alpha(0.16),
    );
    scene.rect(
        1,
        Rect::new(graph.right() - 2.0, trail.y, 2.0, trail.height),
        graph,
        color.with_alpha(0.9),
    );
}

fn clipped_text_bounds(bounds: Rect, body: Rect) -> Rect {
    bounds
        .intersection(body)
        .unwrap_or(Rect::new(bounds.x, bounds.y, 0.0, 0.0))
}

fn column_text_bounds(column: Rect, row: Rect, body: Rect) -> Rect {
    Rect::new(
        column.x + 4.0,
        row.y,
        (column.width - 8.0).max(0.0),
        row.height,
    )
    .intersection(body)
    .unwrap_or(Rect::new(column.x, row.y, 0.0, 0.0))
}

/// Geometry and semantics of a row's leading branch chip, shared by drawing
/// and branch-hover detection.
struct PrimaryChip {
    badge: Rect,
    label: String,
    /// Checkout-ready name: local short name or "remote/name".
    target: String,
    is_local: bool,
    remote_count: usize,
    /// Branch refs on this commit beyond the leading one.
    extra: usize,
}

fn primary_branch_chip(
    clip: Rect,
    row: Rect,
    column: Rect,
    branch_refs: &[CommitBranchRef],
) -> Option<PrimaryChip> {
    let column_clip = column.intersection(clip)?;
    let mut branches = branch_refs
        .iter()
        .filter(|reference| !reference.is_tag)
        .collect::<Vec<_>>();
    branches.sort_by(|left, right| {
        right
            .is_head
            .cmp(&left.is_head)
            .then_with(|| left.branch_short_name.cmp(&right.branch_short_name))
    });
    let branch = *branches.first()?;
    let label = if branch.is_head {
        format!("{} {}", icons::CHECK, branch.branch_short_name)
    } else {
        branch.branch_short_name.clone()
    };
    let icon_count = usize::from(branch.is_local) + branch.remote_names.len();
    let x = column.x + 7.0;
    let width = (label.chars().count().to_f32().unwrap_or(0.0) * 6.2
        + icon_count.to_f32().unwrap_or(0.0) * 11.0
        + 12.0)
        .clamp(38.0, 180.0)
        .min(column_clip.right() - x - 4.0);
    if width <= 4.0 {
        return None;
    }
    let target = if branch.is_local {
        branch.branch_short_name.clone()
    } else {
        branch.remote_names.first().map_or_else(
            || branch.branch_short_name.clone(),
            |remote| format!("{remote}/{}", branch.branch_short_name),
        )
    };
    Some(PrimaryChip {
        badge: Rect::new(x, row.y + 4.0, width, 18.0),
        label,
        target,
        is_local: branch.is_local,
        remote_count: branch.remote_names.len(),
        extra: branches.len() - 1,
    })
}

/// Finds the branch chip under the pointer among visible rows and returns the
/// indices of every commit reachable from that branch tip.
fn hovered_branch_reach(
    state: &AppState,
    snapshot: &crate::git::models::RepoSnapshot,
    body: Rect,
    refs: Rect,
    range: std::ops::Range<usize>,
    wip_offset: usize,
) -> Option<std::collections::HashSet<usize>> {
    if !refs
        .intersection(body)
        .is_some_and(|clip| clip.contains(state.hover()))
    {
        return None;
    }
    let mut tip = None;
    for index in range {
        let Some(commit) = snapshot.commits.get(index) else {
            continue;
        };
        let display_index = index.saturating_add(wip_offset);
        let y = body.y + display_index.to_f32().unwrap_or(0.0) * ROW_HEIGHT - state.graph_scroll;
        let row = Rect::new(body.x, y, body.width, ROW_HEIGHT);
        let Some(chip) = primary_branch_chip(body, row, refs, &commit.branch_refs) else {
            continue;
        };
        if chip
            .badge
            .intersection(body)
            .is_some_and(|badge| badge.contains(state.hover()))
        {
            tip = Some(index);
            break;
        }
    }
    let tip = tip?;
    let index_of = snapshot
        .commits
        .iter()
        .enumerate()
        .map(|(index, commit)| (commit.id.as_str(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut reach = std::collections::HashSet::new();
    let mut queue = vec![tip];
    while let Some(index) = queue.pop() {
        if !reach.insert(index) {
            continue;
        }
        for parent in &snapshot.commits[index].parents {
            if let Some(&parent) = index_of.get(parent.as_str()) {
                queue.push(parent);
            }
        }
    }
    Some(reach)
}

#[allow(clippy::too_many_arguments)]
fn draw_ref_chips(
    scene: &mut Scene,
    theme: &Theme,
    clip: Rect,
    row: Rect,
    column: Rect,
    branch_refs: &[CommitBranchRef],
    refs: &[RefLabel],
    mouse: [f32; 2],
    drag: Option<&RefDrag>,
) {
    let Some(column_clip) = column.intersection(clip) else {
        return;
    };
    let mut x = column.x + 7.0;
    let mut extra = 0;
    let mut any_head = false;
    if let Some(chip) = primary_branch_chip(clip, row, column, branch_refs) {
        extra = chip.extra;
        any_head = branch_refs
            .iter()
            .any(|reference| !reference.is_tag && reference.is_head);
        let hovered = chip
            .badge
            .intersection(column_clip)
            .is_some_and(|badge| badge.contains(mouse));
        let droppable = drag.is_some_and(|drag| drag.source != chip.target);
        let icon_count = usize::from(chip.is_local) + chip.remote_count;
        let full_width = chip.label.chars().count().to_f32().unwrap_or(0.0) * 6.2
            + icon_count.to_f32().unwrap_or(0.0) * 11.0
            + 12.0;
        // A truncated chip expands to its full name on hover, floating above
        // the neighbouring columns.
        let expand = hovered && full_width > chip.badge.width + 0.5;
        let (badge, badge_clip, layer) = if expand {
            (
                Rect::new(
                    chip.badge.x,
                    chip.badge.y,
                    full_width.min(clip.right() - chip.badge.x - 4.0),
                    18.0,
                ),
                clip,
                3,
            )
        } else {
            (chip.badge, column_clip, 2)
        };
        draw_ref_chip(
            scene,
            theme,
            badge,
            badge_clip,
            &chip.label,
            theme.accent,
            theme.border_strong,
            Some(UiAction::BranchClick(chip.target.clone())),
            hovered,
            droppable,
            layer,
        );
        let mut icon_x = badge.x + 6.0 + chip.label.chars().count().to_f32().unwrap_or(0.0) * 6.2;
        if chip.is_local {
            scene.text(
                icons::BRANCH,
                [icon_x, badge.y + 3.5],
                clipped_text_bounds(badge.inset(2.0), badge_clip),
                theme.text_dim,
                9.5,
                11.0,
                FontFace::Terminal,
            );
            icon_x += 11.0;
        }
        for _ in 0..chip.remote_count {
            scene.text(
                icons::REMOTE,
                [icon_x, badge.y + 3.5],
                clipped_text_bounds(badge.inset(2.0), badge_clip),
                theme.text_dim,
                9.5,
                11.0,
                FontFace::Terminal,
            );
            icon_x += 11.0;
        }
        x = chip.badge.right() + 4.0;
    }
    if extra > 0 {
        let label = format!("+{extra}");
        let width = 24.0f32.min(column_clip.right() - x - 4.0);
        if width > 4.0 {
            draw_ref_chip(
                scene,
                theme,
                Rect::new(x, row.y + 4.0, width, 18.0),
                column_clip,
                &label,
                theme.text_muted,
                theme.border,
                None,
                false,
                false,
                2,
            );
            x += width + 4.0;
        }
    }

    let detached_head = refs.iter().any(|label| label.kind == RefKind::Head) && !any_head;
    if detached_head {
        let width = 40.0f32.min(column_clip.right() - x - 4.0);
        if width > 4.0 {
            draw_ref_chip(
                scene,
                theme,
                Rect::new(x, row.y + 4.0, width, 18.0),
                column_clip,
                "HEAD",
                theme.accent,
                theme.accent,
                None,
                false,
                false,
                2,
            );
            x += width + 4.0;
        }
    }
    for label in refs
        .iter()
        .filter(|label| matches!(label.kind, RefKind::Tag | RefKind::Worktree))
    {
        let full_width = label.name.chars().count().to_f32().unwrap_or(0.0) * 6.2 + 24.0;
        let width = full_width
            .clamp(36.0, 160.0)
            .min(column_clip.right() - x - 4.0);
        if width <= 4.0 {
            break;
        }
        let (color, border, icon) = match label.kind {
            RefKind::Tag => (theme.orange, theme.orange_muted, icons::TAG),
            RefKind::Worktree => (theme.purple, theme.purple_muted, icons::WORKSPACE),
            _ => unreachable!("only tag and worktree labels are rendered here"),
        };
        let action = (label.kind == RefKind::Tag).then(|| UiAction::TagClick(label.name.clone()));
        let anchored = Rect::new(x, row.y + 4.0, width, 18.0);
        let hovered = action.is_some()
            && anchored
                .intersection(column_clip)
                .is_some_and(|badge| badge.contains(mouse));
        let droppable = action.is_some() && drag.is_some_and(|drag| drag.source != label.name);
        let expand = hovered && full_width > width + 0.5;
        let (badge, badge_clip, layer) = if expand {
            (
                Rect::new(x, row.y + 4.0, full_width.min(clip.right() - x - 4.0), 18.0),
                clip,
                3,
            )
        } else {
            (anchored, column_clip, 2)
        };
        draw_ref_chip(
            scene, theme, badge, badge_clip, "", color, border, action, hovered, droppable, layer,
        );
        scene.text(
            icon,
            [badge.x + 5.0, badge.y + 3.5],
            clipped_text_bounds(badge.inset(2.0), badge_clip),
            theme.text_dim,
            9.5,
            11.0,
            FontFace::Terminal,
        );
        truncated_text(
            scene,
            &label.name,
            [badge.x + 16.0, badge.y + 3.0],
            clipped_text_bounds(badge.inset(2.0), badge_clip),
            badge_clip,
            color,
            10.5,
            12.0,
            FontFace::Sans,
        );
        x += width + 4.0;
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_ref_chip(
    scene: &mut Scene,
    theme: &Theme,
    badge: Rect,
    clip: Rect,
    label: &str,
    color: crate::ui::Color,
    border: crate::ui::Color,
    action: Option<UiAction>,
    hovered: bool,
    droppable: bool,
    layer: usize,
) {
    // Square chips with hairline outlines; while a ref drag is in flight
    // every valid drop target fills soft yellow, brightening under the
    // pointer.
    let (fill, border, border_width) = if droppable {
        (
            theme.yellow_muted,
            theme.yellow,
            if hovered { 1.6 } else { 1.0 },
        )
    } else if hovered {
        (theme.panel_alt, color, 1.4)
    } else {
        (theme.panel, border, 1.0)
    };
    scene.rounded_rect(layer, badge, clip, fill, border, RADIUS_SM, border_width);
    if !label.is_empty() {
        truncated_text(
            scene,
            label,
            [badge.x + 6.0, badge.y + 3.0],
            clipped_text_bounds(badge.inset(2.0), clip),
            clip,
            theme.text,
            10.5,
            12.0,
            FontFace::Sans,
        );
    }
    if let Some(action) = action {
        scene.hit_clipped(badge, clip, action, CursorHint::Pointer, None);
    }
}

fn empty_state(scene: &mut Scene, theme: &Theme, rect: Rect, message: &str) {
    scene.text(
        message,
        [
            rect.x + rect.width * 0.5 - 74.0,
            rect.y + rect.height * 0.5 + 45.0,
        ],
        Rect::new(rect.x, rect.y + rect.height * 0.5 + 40.0, rect.width, 30.0),
        theme.text_muted,
        13.0,
        18.0,
        FontFace::Sans,
    );
}

fn format_time(seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(DateTime::<Local>::from)
        .map_or_else(
            || "unknown".to_owned(),
            |time| time.format("%m/%d/%Y @ %-I:%M %p").to_string(),
        )
}

fn format_time_for_width(seconds: i64, width: f32) -> String {
    if width < FULL_TIMESTAMP_MINIMUM_WIDTH {
        DateTime::<Utc>::from_timestamp(seconds, 0)
            .map(DateTime::<Local>::from)
            .map_or_else(
                || "unknown".to_owned(),
                |time| time.format("%m/%d/%Y").to_string(),
            )
    } else {
        format_time(seconds)
    }
}

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

    #[test]
    fn narrow_dates_use_a_single_date_label() {
        let timestamp = 1_783_461_600;
        assert_eq!(format_time_for_width(timestamp, 149.0).len(), 10);
        assert!(format_time_for_width(timestamp, 150.0).contains(" @ "));
    }

    #[test]
    fn lanes_keep_readable_spacing_and_avatar_nodes_clear_neighbors() {
        let (_, spacing) = lane_geometry(Rect::new(0.0, 0.0, 120.0, 26.0));
        assert!((spacing - 22.0).abs() < f32::EPSILON);
        // Neighboring lane lines must pass outside the avatar node's ring.
        assert!(AVATAR_NODE_DIAMETER * 0.5 < spacing);
        assert!(MERGE_DOT_DIAMETER < AVATAR_NODE_DIAMETER);
    }
}
