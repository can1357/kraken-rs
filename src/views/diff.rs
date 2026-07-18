use std::{path::Path, sync::LazyLock};

use num_traits::ToPrimitive;
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

use crate::{
    app::state::AppState,
    git::models::{DiffLineSelection, DiffRow, DiffRowKind, DiffScope},
    ui::{
        Color, FontFace, RADIUS_LG, RADIUS_MD, RADIUS_SM, Rect, Scene, Theme,
        action::{CursorHint, ScrollTarget, UiAction},
        icons,
        widgets::{divider, scrollbar, truncated_text},
    },
};

const HEADER_HEIGHT: f32 = 78.0;
pub(crate) const ROW_HEIGHT: f32 = 20.0;

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.window);
    let header = Rect::new(rect.x, rect.y, rect.width, HEADER_HEIGHT);
    scene.rect(1, header, rect, theme.panel);
    divider(
        scene,
        Rect::new(header.x, header.bottom() - 1.0, header.width, 1.0),
        theme,
    );
    let path = state.selected_file.as_ref().map_or_else(
        || "Diff".to_owned(),
        |request| request.path.display().to_string(),
    );
    truncated_text(
        scene,
        &path,
        [header.x + 14.0, header.y + 11.0],
        Rect::new(header.x + 10.0, header.y, header.width - 390.0, 34.0),
        rect,
        theme.text,
        13.0,
        17.0,
        FontFace::Monospace,
    );
    let close = Rect::new(header.right() - 34.0, header.y + 6.0, 28.0, 27.0);
    ghost_button(
        scene,
        close,
        icons::CLOSE,
        UiAction::CloseDiff,
        state.hover(),
        theme,
        true,
        Some("Close diff"),
    );
    if state.focus == crate::app::state::FocusField::DiffSearch {
        draw_diff_search(scene, state, theme, header);
    }

    if let Some(request) = &state.selected_file {
        let (label, action) = match request.scope {
            DiffScope::Staged => ("Unstage File", UiAction::UnstageFile(request.path.clone())),
            DiffScope::Unstaged => ("Stage File", UiAction::StageFile(request.path.clone())),
            DiffScope::Commit(_) => ("", UiAction::DismissOverlay),
        };
        if !label.is_empty() {
            let action_rect = Rect::new(header.right() - 146.0, header.y + 6.0, 102.0, 27.0);
            outline_green_button(scene, action_rect, label, action, state.hover(), theme);
        }
    }
    if state.focus != crate::app::state::FocusField::DiffSearch {
        scene.text(
            "UTF-8",
            [header.right() - 202.0, header.y + 12.0],
            Rect::new(header.right() - 206.0, header.y, 56.0, 33.0),
            theme.text_muted,
            11.0,
            14.0,
            FontFace::Sans,
        );
    }

    let controls_y = header.y + 40.0;
    let mut left_x = header.x + 10.0;
    if state
        .selected_file
        .as_ref()
        .is_some_and(|request| !matches!(request.scope, DiffScope::Commit(_)))
    {
        let scope_label = match state.selected_file.as_ref().map(|request| &request.scope) {
            Some(DiffScope::Staged) => format!("Staged  {}", icons::CHEVRON_DOWN),
            _ => format!("Unstaged  {}", icons::CHEVRON_DOWN),
        };
        let scope = Rect::new(left_x, controls_y, 88.0, 28.0);
        scene.rounded_rect(
            2,
            scope,
            header,
            theme.accent_soft,
            theme.accent.with_alpha(0.35),
            RADIUS_SM,
            1.0,
        );
        scene.text(
            scope_label.to_uppercase(),
            [scope.x + 8.0, scope.y + 7.0],
            scope,
            theme.accent_active,
            10.0,
            14.0,
            FontFace::Sans,
        );
        scene.hit(
            scope,
            UiAction::ToggleDiffScope,
            CursorHint::Pointer,
            Some("Toggle staged and unstaged diff"),
        );
        left_x += scope.width + 8.0;
    }
    let file_tab = Rect::new(left_x, controls_y, 67.0, 28.0);
    let diff_tab = Rect::new(left_x + 67.0, controls_y, 75.0, 28.0);
    let tab_track = Rect::new(
        file_tab.x,
        file_tab.y,
        file_tab.width + diff_tab.width,
        file_tab.height,
    );
    scene.rounded_rect(2, tab_track, header, theme.panel_alt, theme.border, RADIUS_MD, 1.0);
    segmented_tab(
        scene,
        file_tab,
        "File View",
        state.diff_file_view,
        UiAction::ShowFileView,
        state.hover(),
        theme,
    );
    segmented_tab(
        scene,
        diff_tab,
        "Diff View",
        !state.diff_file_view,
        UiAction::ShowDiffView,
        state.hover(),
        theme,
    );

    ghost_button(
        scene,
        Rect::new(header.right() - 326.0, controls_y, 54.0, 28.0),
        "Blame",
        UiAction::DismissOverlay,
        state.hover(),
        theme,
        false,
        Some("Blame is available after history indexing"),
    );
    ghost_button(
        scene,
        Rect::new(header.right() - 272.0, controls_y, 62.0, 28.0),
        "History",
        UiAction::ToggleFileHistory,
        state.hover(),
        theme,
        false,
        Some("File History"),
    );
    ghost_button(
        scene,
        Rect::new(header.right() - 202.0, controls_y, 36.0, 28.0),
        icons::ARROW_UP,
        UiAction::PreviousHunk,
        state.hover(),
        theme,
        !state.diff_file_view,
        Some("Previous change"),
    );
    ghost_button(
        scene,
        Rect::new(header.right() - 166.0, controls_y, 36.0, 28.0),
        icons::ARROW_DOWN,
        UiAction::NextHunk,
        state.hover(),
        theme,
        !state.diff_file_view,
        Some("Next change"),
    );
    let unified = Rect::new(header.right() - 122.0, controls_y, 46.0, 28.0);
    let split = Rect::new(header.right() - 76.0, controls_y, 46.0, 28.0);
    let layout_track = Rect::new(unified.x, unified.y, unified.width + split.width, unified.height);
    scene.rounded_rect(
        2,
        layout_track,
        header,
        theme.panel_alt,
        theme.border,
        RADIUS_MD,
        1.0,
    );
    layout_icon_button(
        scene,
        unified,
        icons::LIST,
        !state.diff_split,
        state.hover(),
        theme,
        Some("Unified diff layout"),
    );
    layout_icon_button(
        scene,
        split,
        icons::SPLIT_VERTICAL,
        state.diff_split,
        state.hover(),
        theme,
        Some("Split diff layout"),
    );

    let canvas = Rect::new(
        rect.x,
        header.bottom(),
        rect.width,
        rect.height - HEADER_HEIGHT,
    );
    scene.rounded_rect(0, canvas, rect, theme.input, theme.border, RADIUS_LG, 1.0);
    scene.rect(
        1,
        Rect::new(canvas.x + 1.0, canvas.y + 1.0, canvas.width - 2.0, 4.0),
        canvas,
        theme.accent_soft.with_alpha(0.55),
    );
    if state.file_history {
        build_history_shell(scene, state, theme, canvas);
    } else if let Some(diff) = &state.diff {
        if state.diff_file_view {
            if diff.binary {
                centered_message(
                    scene,
                    theme,
                    canvas,
                    "Binary file — textual view unavailable",
                );
            } else if let Some(content) = &diff.content {
                draw_file_content(scene, state, theme, canvas, &diff.path, content);
            } else {
                centered_message(
                    scene,
                    theme,
                    canvas,
                    "File is absent in the selected revision",
                );
            }
        } else if diff.binary {
            centered_message(
                scene,
                theme,
                canvas,
                "Binary file — textual diff unavailable",
            );
        } else if diff.rows.is_empty() {
            centered_message(scene, theme, canvas, "No changes in the selected scope");
        } else {
            draw_diff_rows(scene, state, theme, canvas, diff);
        }
    } else {
        centered_message(
            scene,
            theme,
            canvas,
            format!("{}  Loading real file diff…", icons::LOADING),
        );
    }
}

fn outline_green_button(
    scene: &mut Scene,
    rect: Rect,
    label: &str,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
) {
    let hovered = rect.contains(mouse);
    scene.rounded_rect(
        2,
        rect,
        scene.viewport(),
        if hovered {
            theme.green_muted
        } else {
            theme.input
        },
        theme.green,
        RADIUS_MD,
        1.0,
    );
    scene.text(
        label,
        [rect.x + 10.0, rect.y + 7.0],
        rect,
        theme.green,
        11.0,
        15.0,
        FontFace::Sans,
    );
    scene.hit(rect, action, CursorHint::Pointer, Some(label));
}

fn segmented_tab(
    scene: &mut Scene,
    rect: Rect,
    label: &str,
    active: bool,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
) {
    if active {
        scene.rounded_rect(
            2,
            rect.inset(2.0),
            scene.viewport(),
            theme.surface_3,
            theme.border_strong,
            RADIUS_MD - 2.0,
            1.0,
        );
    } else if rect.contains(mouse) {
        scene.rounded_rect(
            2,
            rect.inset(2.0),
            scene.viewport(),
            theme.row_hover,
            theme.row_hover,
            RADIUS_MD - 2.0,
            0.0,
        );
    }
    scene.text(
        label,
        [rect.x + 8.0, rect.y + 7.0],
        rect,
        if active { theme.text } else { theme.text_muted },
        11.0,
        15.0,
        FontFace::Sans,
    );
    scene.hit(rect, action, CursorHint::Pointer, Some(label));
}
#[allow(clippy::too_many_arguments)]
fn ghost_button(
    scene: &mut Scene,
    rect: Rect,
    label: &str,
    action: UiAction,
    mouse: [f32; 2],
    theme: &Theme,
    enabled: bool,
    tooltip: Option<&str>,
) {
    let hovered = enabled && rect.contains(mouse);
    if hovered {
        scene.rounded_rect(
            2,
            rect,
            scene.viewport(),
            theme.row_hover,
            theme.row_hover,
            RADIUS_SM,
            0.0,
        );
    }
    let is_icon = label
        .chars()
        .next()
        .is_some_and(|first| ('\u{E000}'..='\u{F8FF}').contains(&first));
    scene.text(
        label,
        [rect.x + 8.0, rect.y + if is_icon { 6.0 } else { 7.0 }],
        rect,
        if !enabled {
            theme.text_disabled
        } else if hovered {
            theme.text
        } else {
            theme.text_muted
        },
        if is_icon { 13.0 } else { 11.0 },
        if is_icon { 16.0 } else { 15.0 },
        if is_icon {
            FontFace::Monospace
        } else {
            FontFace::Sans
        },
    );
    scene.hit(rect, action, CursorHint::Pointer, tooltip);
}

fn layout_icon_button(
    scene: &mut Scene,
    rect: Rect,
    icon: &str,
    active: bool,
    mouse: [f32; 2],
    theme: &Theme,
    tooltip: Option<&str>,
) {
    if active {
        scene.rounded_rect(
            2,
            rect.inset(2.0),
            scene.viewport(),
            theme.surface_3,
            theme.border_strong,
            RADIUS_MD - 2.0,
            1.0,
        );
    } else if rect.contains(mouse) {
        scene.rounded_rect(
            2,
            rect.inset(2.0),
            scene.viewport(),
            theme.row_hover,
            theme.row_hover,
            RADIUS_MD - 2.0,
            0.0,
        );
    }
    scene.text(
        icon,
        [rect.x + 17.0, rect.y + 6.0],
        rect,
        if active { theme.text } else { theme.text_muted },
        13.0,
        16.0,
        FontFace::Monospace,
    );
    scene.hit(
        rect,
        UiAction::ToggleDiffLayout,
        CursorHint::Pointer,
        tooltip,
    );
}

fn draw_file_content(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    canvas: Rect,
    path: &Path,
    content: &str,
) {
    truncated_text(
        scene,
        &path.display().to_string(),
        [canvas.x + 48.0, canvas.y + 5.0],
        Rect::new(canvas.x + 45.0, canvas.y, canvas.width - 50.0, 22.0),
        canvas,
        theme.text_dim,
        9.5,
        13.0,
        FontFace::Monospace,
    );
    let code = Rect::new(
        canvas.x,
        canvas.y + 23.0,
        canvas.width,
        canvas.height - 23.0,
    );
    let content_height = content.lines().count().to_f32().unwrap_or(0.0) * ROW_HEIGHT;
    let scroll = state
        .diff_scroll
        .min((content_height - code.height).max(0.0));
    let first = (scroll / ROW_HEIGHT)
        .floor()
        .to_usize()
        .unwrap_or(0)
        .saturating_sub(1);
    let count = (code.height / ROW_HEIGHT)
        .ceil()
        .to_usize()
        .unwrap_or(0)
        .saturating_add(3);
    for (offset, line) in content.lines().skip(first).take(count).enumerate() {
        let index = first.saturating_add(offset);
        let y = code.y + index.to_f32().unwrap_or(0.0) * ROW_HEIGHT - scroll;
        let row = Rect::new(code.x, y, code.width, ROW_HEIGHT);
        if row.bottom() <= code.y || row.y >= code.bottom() {
            continue;
        }
        if index % 2 == 1 {
            scene.rect(0, row, code, theme.panel_alt.with_alpha(0.6));
        }
        draw_number(
            scene,
            row,
            u32::try_from(index.saturating_add(1)).ok(),
            theme.text_dim,
            code,
        );
        draw_code(
            scene,
            path,
            line,
            row.x + 48.0,
            row.y + 4.0,
            Rect::new(row.x + 45.0, row.y, row.width - 45.0, row.height),
            code,
            theme,
        );
    }
    scrollbar(
        scene,
        code,
        content_height,
        scroll,
        ScrollTarget::Diff,
        theme,
    );
}

fn draw_diff_rows(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    canvas: Rect,
    diff: &crate::git::models::DiffDocument,
) {
    const MAP_WIDTH: f32 = 20.0;
    let content_height = diff.rows.len().to_f32().unwrap_or(0.0) * ROW_HEIGHT;
    let code_top = canvas.y + 23.0;
    // The minimap owns a reserved gutter at the right edge; rows never run
    // beneath it.
    let map = Rect::new(
        canvas.right() - MAP_WIDTH,
        code_top,
        MAP_WIDTH,
        canvas.height - 23.0,
    );
    let code_clip = Rect::new(
        canvas.x,
        code_top,
        canvas.width - MAP_WIDTH,
        canvas.height - 23.0,
    );
    let split = canvas.x + code_clip.width * 0.5;

    let gutter_width = 46.0;
    if state.diff_split {
        scene.rect(
            0,
            Rect::new(canvas.x, code_clip.y, gutter_width, code_clip.height),
            code_clip,
            theme.panel,
        );
        scene.rect(
            0,
            Rect::new(split, code_clip.y, gutter_width, code_clip.height),
            code_clip,
            theme.panel,
        );
    } else {
        scene.rect(
            0,
            Rect::new(canvas.x, code_clip.y, gutter_width + 16.0, code_clip.height),
            code_clip,
            theme.panel,
        );
    }
    if state.diff_split {
        scene.rect(
            1,
            Rect::new(split, canvas.y, 1.0, canvas.height),
            canvas,
            theme.border_strong,
        );
        scene.text(
            &diff.old_label,
            [canvas.x + 48.0, canvas.y + 5.0],
            Rect::new(canvas.x + 45.0, canvas.y, canvas.width * 0.5 - 50.0, 22.0),
            theme.text_dim,
            9.5,
            13.0,
            FontFace::Monospace,
        );
        scene.text(
            &diff.new_label,
            [split + 48.0, canvas.y + 5.0],
            Rect::new(split + 45.0, canvas.y, canvas.width * 0.5 - 50.0, 22.0),
            theme.text_dim,
            9.5,
            13.0,
            FontFace::Monospace,
        );
    }
    let scroll = state
        .diff_scroll
        .min((content_height - code_clip.height).max(0.0));
    let first = (scroll / ROW_HEIGHT)
        .floor()
        .to_usize()
        .unwrap_or(0)
        .saturating_sub(1);
    let count = (code_clip.height / ROW_HEIGHT)
        .ceil()
        .to_usize()
        .unwrap_or(0)
        .saturating_add(3);
    let end = first.saturating_add(count).min(diff.rows.len());
    for index in first..end {
        let row = &diff.rows[index];
        let y = code_top + index.to_f32().unwrap_or(0.0) * ROW_HEIGHT - scroll;
        let row_rect = Rect::new(code_clip.x, y, code_clip.width, ROW_HEIGHT);
        if row_rect.bottom() <= code_clip.y || row_rect.y >= code_clip.bottom() {
            continue;
        }
        if row.kind == DiffRowKind::Hunk {
            scene.rect(1, row_rect, code_clip, theme.panel_alt);
            scene.rect(
                1,
                Rect::new(row_rect.x, row_rect.y, row_rect.width, 1.0),
                code_clip,
                theme.border,
            );
            scene.rect(
                1,
                Rect::new(row_rect.x, row_rect.bottom() - 1.0, row_rect.width, 1.0),
                code_clip,
                theme.border,
            );
            if let Some(text_bounds) = row_rect.inset(3.0).clipped(code_clip) {
                scene.text(
                    row.new_text.to_uppercase(),
                    [row_rect.x + 9.0, row_rect.y + 4.0],
                    text_bounds,
                    theme.text_muted,
                    12.0,
                    15.0,
                    FontFace::Monospace,
                );
            }
            continue;
        }
        if state.diff_selected_rows.contains(&index) {
            scene.rect(1, row_rect, code_clip, theme.row_selected);
            scene.rect(
                1,
                Rect::new(row_rect.x, row_rect.y, 2.0, row_rect.height),
                code_clip,
                theme.accent,
            );
        }
        let inline_side = u8::from(row.kind != DiffRowKind::Deleted);
        draw_text_marks(
            scene,
            state,
            theme,
            code_clip,
            row_rect,
            index,
            if state.diff_split { 0 } else { inline_side },
            row_rect.x + if state.diff_split { 48.0 } else { 53.0 },
        );
        if state.diff_split {
            draw_text_marks(
                scene,
                state,
                theme,
                code_clip,
                row_rect,
                index,
                1,
                split + 48.0,
            );
        }
        if state.diff_split {
            draw_split_row(scene, theme, code_clip, row_rect, split, row, &diff.path);
        } else {
            draw_inline_row(scene, theme, code_clip, row_rect, row, &diff.path);
        }
        draw_line_gutter_action(scene, state, theme, code_clip, row_rect, split, index, row);
        let (text_rect, side) = if state.diff_split {
            if state.hover()[0] < split {
                (
                    Rect::new(
                        row_rect.x + 48.0,
                        row_rect.y,
                        split - row_rect.x - 48.0,
                        row_rect.height,
                    ),
                    0,
                )
            } else {
                (
                    Rect::new(
                        split + 48.0,
                        row_rect.y,
                        code_clip.right() - split - 48.0,
                        row_rect.height,
                    ),
                    1,
                )
            }
        } else {
            (
                Rect::new(
                    row_rect.x + 53.0,
                    row_rect.y,
                    row_rect.width - 53.0,
                    row_rect.height,
                ),
                inline_side,
            )
        };
        if let Some(text_rect) = text_rect.clipped(code_clip) {
            let column = ((state.hover()[0] - text_rect.x) / 7.2)
                .max(0.0)
                .floor()
                .to_usize()
                .unwrap_or(0);
            scene.hit_clipped(
                text_rect,
                code_clip,
                UiAction::BeginDiffTextSelection {
                    row: index,
                    side,
                    column,
                    clicks: 1,
                },
                CursorHint::Text,
                None,
            );
        }
        scene.rect(
            1,
            Rect::new(row_rect.x, row_rect.bottom() - 1.0, row_rect.width, 1.0),
            code_clip,
            theme.border.with_alpha(0.35),
        );
    }
    draw_minimap(
        scene,
        theme,
        map,
        code_clip.height,
        diff,
        scroll,
        state.hover(),
    );
    let content_height = diff.rows.len().to_f32().unwrap_or(0.0) * ROW_HEIGHT;
    scrollbar(
        scene,
        code_clip,
        content_height,
        scroll,
        ScrollTarget::Diff,
        theme,
    );
}

fn draw_text_marks(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    row_rect: Rect,
    row: usize,
    side: u8,
    text_x: f32,
) {
    for (match_index, (match_row, match_side, start, end)) in
        state.diff_search_results().iter().enumerate()
    {
        if *match_row == row && *match_side == side {
            let current = match_index == state.diff_search_cursor;
            let color = if current {
                theme.yellow_muted
            } else {
                theme.yellow_muted.with_alpha(0.6)
            };
            let mark = Rect::new(
                text_x + start.to_f32().unwrap_or(0.0) * 7.2,
                row_rect.y + 2.0,
                (end - start).to_f32().unwrap_or(0.0) * 7.2,
                row_rect.height - 4.0,
            );
            if current {
                scene.rounded_rect(2, mark, clip, color, theme.yellow, 3.0, 1.0);
            } else {
                scene.rounded_rect(2, mark, clip, color, color, 3.0, 0.0);
            }
        }
    }
    if let Some(((start_row, selected_side, start_column), (end_row, _, end_column))) =
        state.diff_text_selection
    {
        if selected_side != side || row < start_row.min(end_row) || row > start_row.max(end_row) {
            return;
        }
        let (first_row, first_column, last_row, last_column) = if start_row <= end_row {
            (start_row, start_column, end_row, end_column)
        } else {
            (end_row, end_column, start_row, start_column)
        };
        let start = if row == first_row { first_column } else { 0 };
        let end = if row == last_row { last_column } else { 10_000 };
        if end > start {
            scene.rect(
                2,
                Rect::new(
                    text_x + start.to_f32().unwrap_or(0.0) * 7.2,
                    row_rect.y + 2.0,
                    (end - start).min(10_000).to_f32().unwrap_or(0.0) * 7.2,
                    row_rect.height - 4.0,
                ),
                clip,
                theme.accent.with_alpha(0.22),
            );
        }
    }
}

fn draw_diff_search(scene: &mut Scene, state: &AppState, theme: &Theme, header: Rect) {
    let rect = Rect::new(header.right() - 420.0, header.y + 5.0, 212.0, 28.0);
    scene.rounded_rect(
        3,
        rect,
        header,
        theme.input,
        theme.accent,
        RADIUS_MD,
        1.0,
    );
    scene.text(
        state.diff_search.text(),
        [rect.x + 8.0, rect.y + 7.0],
        rect.inset(7.0),
        theme.text,
        11.0,
        15.0,
        FontFace::Monospace,
    );
    crate::ui::widgets::caret_overlay(
        scene,
        3,
        [rect.x + 8.0, rect.y + 7.0],
        rect.inset(2.0),
        &state.diff_search,
        6.6,
        15.0,
        theme,
    );
    let results = state.diff_search_results();
    let counter = format!(
        "{} of {}",
        if results.is_empty() {
            0
        } else {
            state.diff_search_cursor + 1
        },
        results.len()
    );
    scene.text(
        counter,
        [rect.right() + 8.0, rect.y + 7.0],
        Rect::new(rect.right() + 6.0, rect.y, 70.0, rect.height),
        theme.text_muted,
        10.0,
        14.0,
        FontFace::Sans,
    );
    let previous = Rect::new(rect.right() + 78.0, rect.y, 24.0, rect.height);
    let next = Rect::new(rect.right() + 103.0, rect.y, 24.0, rect.height);
    let close = Rect::new(rect.right() + 128.0, rect.y, 24.0, rect.height);
    for (button, label, action) in [
        (previous, icons::CHEVRON_LEFT, UiAction::PreviousDiffSearch),
        (next, icons::CHEVRON_RIGHT, UiAction::NextDiffSearch),
        (close, icons::CLOSE, UiAction::CloseDiffSearch),
    ] {
        scene.text(
            label,
            [button.x + 8.0, button.y + 6.0],
            button,
            theme.text_muted,
            13.0,
            16.0,
            FontFace::Monospace,
        );
        scene.hit(button, action, CursorHint::Pointer, None);
    }
}

/// Draws the change minimap along the diff canvas's right edge.
///
/// Change runs render as proportional green/red/orange bands (GitKraken-style),
/// the visible range as an outlined window, and every band is a click target
/// that jumps the scroll to its first row — the fast path to conflicts.
fn draw_minimap(
    scene: &mut Scene,
    theme: &Theme,
    strip: Rect,
    viewport_height: f32,
    diff: &crate::git::models::DiffDocument,
    scroll: f32,
    mouse: [f32; 2],
) {
    let total = diff.rows.len();
    if total == 0 {
        return;
    }
    scene.rect(2, strip, strip, theme.top_bar);
    scene.rect(
        2,
        Rect::new(strip.x, strip.y, 1.0, strip.height),
        strip,
        theme.border,
    );
    let scale = strip.height / total.to_f32().unwrap_or(1.0);
    let mut index = 0;
    while index < total {
        if !is_change(diff.rows[index].kind) {
            index += 1;
            continue;
        }
        let start = index;
        let (mut added, mut deleted, mut changed) = (0usize, 0usize, 0usize);
        while index < total && is_change(diff.rows[index].kind) {
            match diff.rows[index].kind {
                DiffRowKind::Added => added += 1,
                DiffRowKind::Deleted => deleted += 1,
                _ => changed += 1,
            }
            index += 1;
        }
        let color = if changed > 0 || (added > 0 && deleted > 0) {
            theme.orange
        } else if added > 0 {
            theme.green
        } else {
            theme.red
        };
        let y = strip.y + start.to_f32().unwrap_or(0.0) * scale;
        let height = ((index - start).to_f32().unwrap_or(0.0) * scale).max(3.0);
        let band = Rect::new(strip.x + 3.0, y, strip.width - 6.0, height);
        scene.rect(3, band, strip, color.with_alpha(0.9));
        let target = Rect::new(strip.x, y - 3.0, strip.width, height + 6.0);
        if let Some(target) = target.clipped(strip) {
            scene.hit(
                target,
                UiAction::SeekDiffRow(start),
                CursorHint::Pointer,
                Some("Jump to change"),
            );
        }
    }
    let content_height = total.to_f32().unwrap_or(0.0) * ROW_HEIGHT;
    if content_height <= viewport_height {
        return;
    }
    let view_y = strip.y + (scroll / ROW_HEIGHT) * scale;
    let view_height = ((viewport_height / ROW_HEIGHT) * scale).max(14.0);
    let window = Rect::new(strip.x + 1.0, view_y, strip.width - 1.0, view_height);
    let hovered = strip.contains(mouse);
    scene.rounded_rect(
        3,
        window,
        strip,
        theme.accent.with_alpha(0.18),
        if hovered {
            theme.accent
        } else {
            theme.accent.with_alpha(0.5)
        },
        3.0,
        1.0,
    );
}

/// Rows that count as changes for minimap bands (hunk headers are separators).
fn is_change(kind: DiffRowKind) -> bool {
    matches!(
        kind,
        DiffRowKind::Added | DiffRowKind::Deleted | DiffRowKind::Changed
    )
}

#[allow(clippy::too_many_arguments)]
fn draw_line_gutter_action(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    row_rect: Rect,
    split: f32,
    index: usize,
    row: &DiffRow,
) {
    let gutter_rect = if state.diff_split {
        Rect::new(
            if row.new_number.is_some() {
                split
            } else {
                clip.x
            },
            row_rect.y,
            46.0,
            row_rect.height,
        )
    } else {
        Rect::new(clip.x, row_rect.y, 46.0, row_rect.height)
    };
    let Some(gutter) = gutter_rect.clipped(clip) else {
        return;
    };
    scene.hit_clipped(
        gutter,
        clip,
        UiAction::BeginDiffSelection(index),
        CursorHint::Pointer,
        Some("Drag to select diff lines"),
    );
    if matches!(row.kind, DiffRowKind::Context) {
        return;
    }
    let Some(request) = &state.selected_file else {
        return;
    };
    let staged = matches!(request.scope, DiffScope::Staged);
    if !matches!(request.scope, DiffScope::Staged | DiffScope::Unstaged)
        || !gutter.contains(state.hover())
    {
        return;
    }
    let affordance = Rect::new(gutter_rect.right() - 17.0, gutter_rect.y + 2.0, 15.0, 16.0);
    let Some(affordance_bounds) = affordance.clipped(clip) else {
        return;
    };
    let fill = if staged {
        theme.red_muted
    } else {
        theme.green_muted
    };
    scene.rounded_rect(3, affordance, clip, fill, fill, RADIUS_SM, 0.0);
    scene.text(
        if staged { icons::REMOVE } else { icons::ADD },
        [affordance.x + 4.0, affordance.y + 1.0],
        affordance_bounds,
        if staged { theme.red } else { theme.green },
        12.0,
        15.0,
        FontFace::Monospace,
    );
    let lines = vec![DiffLineSelection {
        old_line: row.old_number,
        new_line: row.new_number,
    }];
    scene.hit_clipped(
        affordance_bounds,
        clip,
        if staged {
            UiAction::UnstageDiffLines {
                path: request.path.clone(),
                lines,
            }
        } else {
            UiAction::StageDiffLines {
                path: request.path.clone(),
                lines,
            }
        },
        CursorHint::Pointer,
        Some(if staged {
            "Unstage selected line"
        } else {
            "Stage selected line"
        }),
    );
}

fn draw_split_row(
    scene: &mut Scene,
    theme: &Theme,
    clip: Rect,
    row_rect: Rect,
    split: f32,
    row: &DiffRow,
    path: &Path,
) {
    let half = row_rect.width * 0.5;
    let old_rect = Rect::new(row_rect.x, row_rect.y, half, row_rect.height);
    let new_rect = Rect::new(split, row_rect.y, half, row_rect.height);
    match row.kind {
        DiffRowKind::Changed => {
            scene.rect(1, old_rect, clip, theme.red_muted);
            scene.rect(1, new_rect, clip, theme.green_muted);
        }
        DiffRowKind::Added => {
            draw_hatch(scene, old_rect, clip, theme);
            scene.rect(1, new_rect, clip, theme.green_muted);
        }
        DiffRowKind::Deleted => {
            scene.rect(1, old_rect, clip, theme.red_muted);
            draw_hatch(scene, new_rect, clip, theme);
        }
        DiffRowKind::Context | DiffRowKind::Hunk => {}
    }
    draw_intraline(
        scene,
        theme.red.with_alpha(0.25),
        old_rect,
        clip,
        row.old_mark,
        &row.old_text,
    );
    draw_intraline(
        scene,
        theme.green.with_alpha(0.25),
        new_rect,
        clip,
        row.new_mark,
        &row.new_text,
    );
    let old_gutter = match row.kind {
        DiffRowKind::Deleted | DiffRowKind::Changed => theme.red,
        DiffRowKind::Added | DiffRowKind::Context | DiffRowKind::Hunk => theme.text_dim,
    };
    let new_gutter = match row.kind {
        DiffRowKind::Added | DiffRowKind::Changed => theme.green,
        DiffRowKind::Deleted | DiffRowKind::Context | DiffRowKind::Hunk => theme.text_dim,
    };
    draw_number(scene, old_rect, row.old_number, old_gutter, clip);
    draw_number(scene, new_rect, row.new_number, new_gutter, clip);
    draw_code(
        scene,
        path,
        &row.old_text,
        old_rect.x + 48.0,
        row_rect.y + 4.0,
        Rect::new(
            old_rect.x + 45.0,
            old_rect.y,
            old_rect.width - 45.0,
            old_rect.height,
        ),
        clip,
        theme,
    );
    draw_code(
        scene,
        path,
        &row.new_text,
        new_rect.x + 48.0,
        row_rect.y + 4.0,
        Rect::new(
            new_rect.x + 45.0,
            new_rect.y,
            new_rect.width - 45.0,
            new_rect.height,
        ),
        clip,
        theme,
    );
}

fn draw_inline_row(
    scene: &mut Scene,
    theme: &Theme,
    clip: Rect,
    row_rect: Rect,
    row: &DiffRow,
    path: &Path,
) {
    let (prefix, text, number, color, fill, mark) = match row.kind {
        DiffRowKind::Added => (
            icons::DIFF_ADDED,
            &row.new_text,
            row.new_number,
            theme.green,
            theme.green_muted,
            row.new_mark,
        ),
        DiffRowKind::Deleted => (
            icons::DIFF_REMOVED,
            &row.old_text,
            row.old_number,
            theme.red,
            theme.red_muted,
            row.old_mark,
        ),
        DiffRowKind::Changed => (
            icons::DIFF_MODIFIED,
            &row.new_text,
            row.new_number,
            theme.orange,
            theme.orange_muted,
            row.new_mark,
        ),
        DiffRowKind::Context | DiffRowKind::Hunk => (
            " ",
            &row.new_text,
            row.new_number,
            theme.text_muted,
            theme.input,
            None,
        ),
    };
    if row.kind != DiffRowKind::Context {
        scene.rect(1, row_rect, clip, fill);
    }
    draw_intraline(scene, color.with_alpha(0.25), row_rect, clip, mark, text);
    let gutter_color = match row.kind {
        DiffRowKind::Added => theme.green,
        DiffRowKind::Deleted => theme.red,
        DiffRowKind::Changed => theme.orange,
        DiffRowKind::Context | DiffRowKind::Hunk => theme.text_dim,
    };
    draw_number(scene, row_rect, number, gutter_color, clip);
    if let Some(prefix_bounds) = row_rect.clipped(clip) {
        scene.text(
            prefix,
            [row_rect.x + 39.0, row_rect.y + 4.0],
            prefix_bounds,
            color,
            10.0,
            14.0,
            FontFace::Monospace,
        );
    }
    draw_code(
        scene,
        path,
        text,
        row_rect.x + 53.0,
        row_rect.y + 4.0,
        Rect::new(
            row_rect.x + 50.0,
            row_rect.y,
            row_rect.width - 50.0,
            row_rect.height,
        ),
        clip,
        theme,
    );
}

fn draw_number(scene: &mut Scene, rect: Rect, number: Option<u32>, color: Color, clip: Rect) {
    let bounds = Rect::new(rect.x + 3.0, rect.y, 34.0, rect.height);
    if let Some(clipped_bounds) = bounds.clipped(clip) {
        scene.text(
            number.map_or_else(String::new, |number| number.to_string()),
            [rect.x + 6.0, rect.y + 4.0],
            clipped_bounds,
            color,
            10.0,
            13.0,
            FontFace::Monospace,
        );
    }
}

fn draw_intraline(
    scene: &mut Scene,
    color: Color,
    pane: Rect,
    clip: Rect,
    mark: Option<(usize, usize)>,
    text: &str,
) {
    let Some((start, end)) = mark else {
        return;
    };
    let prefix = text.get(..start).unwrap_or_default().chars().count();
    let changed = text
        .get(start..end)
        .unwrap_or_default()
        .chars()
        .count()
        .max(1);
    let x = pane.x + 48.0 + prefix.to_f32().unwrap_or(0.0) * 7.2;
    let width = changed.to_f32().unwrap_or(1.0) * 7.2;
    scene.rounded_rect(
        2,
        Rect::new(x, pane.y + 2.0, width, pane.height - 4.0),
        clip,
        color,
        color,
        3.0,
        0.0,
    );
}

#[allow(clippy::too_many_arguments)]
fn draw_code(
    scene: &mut Scene,
    path: &Path,
    text: &str,
    mut x: f32,
    y: f32,
    bounds: Rect,
    clip: Rect,
    theme: &Theme,
) {
    for (token, color) in syntax_spans(path, text, theme) {
        if x >= bounds.right() {
            break;
        }
        let width = token.chars().count().to_f32().unwrap_or(0.0) * 7.2 + 1.0;
        let token_bounds = Rect::new(x, bounds.y, width.min(bounds.right() - x), bounds.height);
        if let Some(clipped_bounds) = token_bounds.clipped(clip) {
            scene.text(
                token,
                [x, y],
                clipped_bounds,
                color,
                12.0,
                16.0,
                FontFace::Monospace,
            );
        }
        x += width;
    }
}

fn syntax_spans(path: &Path, line: &str, theme: &Theme) -> Vec<(String, Color)> {
    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
    let syntaxes = &*SYNTAXES;
    let themes = &*THEMES;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("txt");
    let syntax = syntaxes
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let Some(syntax_theme) = themes
        .themes
        .get("base16-ocean.dark")
        .or_else(|| themes.themes.values().next())
    else {
        return vec![(line.to_owned(), theme.text_muted)];
    };
    let mut highlighter = HighlightLines::new(syntax, syntax_theme);
    highlighter.highlight_line(line, syntaxes).map_or_else(
        |_| vec![(line.to_owned(), theme.text_muted)],
        |ranges| {
            ranges
                .into_iter()
                .map(|(style, token)| {
                    (
                        token.to_owned(),
                        Color::rgba(
                            style.foreground.r,
                            style.foreground.g,
                            style.foreground.b,
                            style.foreground.a,
                        ),
                    )
                })
                .collect()
        },
    )
}

fn build_history_shell(scene: &mut Scene, state: &AppState, theme: &Theme, canvas: Rect) {
    let list = Rect::new(
        canvas.x,
        canvas.y,
        268.0_f32.min(canvas.width * 0.35),
        canvas.height,
    );
    scene.rect(1, list, canvas, theme.panel);
    divider(
        scene,
        Rect::new(list.right(), list.y, 1.0, list.height),
        theme,
    );
    scene.text(
        format!(
            "File History: {}",
            state.selected_file.as_ref().map_or_else(
                || "file".to_owned(),
                |request| request.path.display().to_string()
            )
        ),
        [list.x + 12.0, list.y + 12.0],
        Rect::new(list.x + 9.0, list.y + 7.0, list.width - 18.0, 35.0),
        theme.text,
        12.0,
        17.0,
        FontFace::Sans,
    );
    scene.text(
        "History is read from the repository.\nSelect History again to return to the diff.",
        [list.x + 14.0, list.y + 56.0],
        Rect::new(list.x + 12.0, list.y + 48.0, list.width - 24.0, 58.0),
        theme.text_muted,
        10.5,
        16.0,
        FontFace::Sans,
    );
    if let Some(diff) = &state.diff {
        let right = Rect::new(
            list.right() + 1.0,
            canvas.y,
            canvas.width - list.width - 1.0,
            canvas.height,
        );
        draw_diff_rows(scene, state, theme, right, diff);
    }
}

fn centered_message(scene: &mut Scene, theme: &Theme, rect: Rect, message: impl Into<String>) {
    scene.text(
        message,
        [
            rect.x + rect.width * 0.5 - 120.0,
            rect.y + rect.height * 0.5 - 10.0,
        ],
        Rect::new(
            rect.x + 20.0,
            rect.y + rect.height * 0.5 - 20.0,
            rect.width - 40.0,
            40.0,
        ),
        theme.text_muted,
        13.0,
        16.0,
        FontFace::Sans,
    );
}

fn draw_hatch(scene: &mut Scene, rect: Rect, clip: Rect, theme: &Theme) {
    let color = theme.border.with_alpha(0.2);
    let step = 8.0;
    let mut x = rect.x - rect.height;
    let offset = (rect.y - rect.x) % step;
    x -= offset;
    if let Some(intersection) = rect.clipped(clip) {
        while x < rect.right() {
            scene.line(
                1,
                [x, rect.bottom()],
                [x + rect.height, rect.y],
                1.0,
                color,
                intersection,
            );
            x += step;
        }
    }
}
