use num_traits::ToPrimitive;

use crate::{
    app::state::{AppState, FocusField},
    git::models::{ChangeKind, WorkingFile},
    ui::{
        FontFace, RADIUS_MD, RADIUS_SM, Rect, Scene, Theme,
        action::{CursorHint, ScrollTarget, UiAction},
        icons,
        widgets::{button, checkbox, divider, scrollbar},
    },
};

/// Fixed height of the commit form pinned to the panel bottom.
const COMMIT_FORM_HEIGHT: f32 = 274.0;
/// Height of one file row in either list.
const ROW_HEIGHT: f32 = 24.0;
/// Height of one pinned section header.
const HEADER_HEIGHT: f32 = 28.0;
/// Content height reserved for an empty section's placeholder line.
const EMPTY_CONTENT_HEIGHT: f32 = 29.0;

/// Fixed geometry for the two independently scrolled WIP file lists.
pub(crate) struct SectionLayout {
    pub(crate) unstaged_header: Rect,
    pub(crate) unstaged_view: Rect,
    pub(crate) unstaged_content: f32,
    pub(crate) staged_header: Rect,
    pub(crate) staged_view: Rect,
    pub(crate) staged_content: f32,
}

/// Splits the list area between the Unstaged and Staged sections.
///
/// Rule: each list is capped at half the available height; a list whose
/// content needs less than half keeps its natural height and cedes the
/// surplus to the other, so a tiny section never wastes list space.
pub(crate) fn section_layout(state: &AppState, panel: Rect) -> SectionLayout {
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
    let commit_height = COMMIT_FORM_HEIGHT.min(panel.height * 0.43);
    let list = Rect::new(
        panel.x + 1.0,
        panel.y + 60.0,
        panel.width - 2.0,
        (panel.height - 60.0 - commit_height).max(0.0),
    );
    let content = |count: usize| {
        if count == 0 {
            EMPTY_CONTENT_HEIGHT
        } else {
            count.to_f32().unwrap_or(0.0) * ROW_HEIGHT
        }
    };
    let unstaged_content = content(unstaged_count);
    let staged_content = content(staged_count);
    // 9px top padding + two pinned headers + 8px gap between the sections.
    let available = (list.height - 2.0 * HEADER_HEIGHT - 17.0).max(0.0);
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
    let unstaged_header = Rect::new(list.x, list.y + 9.0, list.width, HEADER_HEIGHT);
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
        HEADER_HEIGHT,
    );
    let staged_view = Rect::new(list.x, staged_header.bottom(), list.width, staged_height);
    SectionLayout {
        unstaged_header,
        unstaged_view,
        unstaged_content,
        staged_header,
        staged_view,
        staged_content,
    }
}

/// Returns the WIP list section under the pointer, headers included, so the
/// wheel scrolls the hovered section instead of the surface behind the panel.
pub(crate) fn scroll_target_at(state: &AppState, mouse: [f32; 2]) -> Option<ScrollTarget> {
    let panel = super::Layout::for_state(state).detail?;
    let sections = section_layout(state, panel);
    let zone = |header: Rect, view: Rect| {
        Rect::new(
            header.x,
            header.y,
            header.width,
            (view.bottom() - header.y).max(0.0),
        )
    };
    if zone(sections.unstaged_header, sections.unstaged_view).contains(mouse) {
        Some(ScrollTarget::WipUnstaged)
    } else if zone(sections.staged_header, sections.staged_view).contains(mouse) {
        Some(ScrollTarget::WipStaged)
    } else {
        None
    }
}

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.panel);
    divider(scene, Rect::new(rect.x, rect.y, 1.0, rect.height), theme);
    let Some(snapshot) = &state.snapshot else {
        scene.text(
            format!("{}  Loading working changes…", icons::LOADING),
            [rect.x + 28.0, rect.y + 54.0],
            rect.inset(20.0),
            theme.accent,
            13.0,
            18.0,
            FontFace::Sans,
        );
        return;
    };

    let file_count = snapshot.working.files.len();
    let header = Rect::new(rect.x, rect.y, rect.width, 60.0);
    let live_pixel = Rect::new(header.x + 14.0, header.y + 12.0, 6.0, 6.0);
    scene.rounded_rect(1, live_pixel, header, theme.purple, theme.purple, 3.0, 0.0);
    scene.text(
        "WORKING TREE · LIVE",
        [live_pixel.right() + 6.0, header.y + 8.0],
        Rect::new(header.x + 14.0, header.y + 4.0, header.width - 28.0, 18.0),
        theme.purple,
        11.0,
        14.0,
        FontFace::Sans,
    );
    scene.text(
        format!(
            "{file_count} file {} on {}",
            if file_count == 1 { "change" } else { "changes" },
            snapshot.head
        ),
        [header.x + 14.0, header.y + 30.0],
        Rect::new(header.x + 10.0, header.y + 22.0, header.width - 124.0, 30.0),
        theme.text,
        15.0,
        20.0,
        FontFace::SansMedium,
    );
    let toggle = Rect::new(header.right() - 104.0, header.y + 25.0, 90.0, 26.0);
    scene.rounded_rect(2, toggle, header, theme.panel_alt, theme.border, RADIUS_MD, 1.0);
    let path_rect = Rect::new(toggle.x, toggle.y, 45.0, toggle.height);
    let tree_rect = Rect::new(path_rect.right(), toggle.y, 45.0, toggle.height);
    let active_rect = if state.path_tree {
        tree_rect
    } else {
        path_rect
    };
    scene.rounded_rect(
        2,
        Rect::new(
            active_rect.x + 2.0,
            active_rect.y + 2.0,
            active_rect.width - 4.0,
            active_rect.height - 4.0,
        ),
        toggle,
        theme.surface_3,
        theme.border_strong,
        RADIUS_MD - 2.0,
        1.0,
    );
    scene.text(
        "PATH",
        [path_rect.x + 9.0, path_rect.y + 6.0],
        path_rect,
        if state.path_tree {
            theme.text_muted
        } else {
            theme.text
        },
        11.0,
        14.0,
        FontFace::Sans,
    );
    scene.text(
        "TREE",
        [tree_rect.x + 9.0, tree_rect.y + 6.0],
        tree_rect,
        if state.path_tree {
            theme.text
        } else {
            theme.text_muted
        },
        11.0,
        14.0,
        FontFace::Sans,
    );
    scene.hit(
        toggle,
        UiAction::TogglePathTree,
        CursorHint::Pointer,
        Some("Toggle Path and Tree"),
    );

    let commit_height = COMMIT_FORM_HEIGHT.min(rect.height * 0.43);
    let commit_rect = Rect::new(
        rect.x + 1.0,
        rect.bottom() - commit_height,
        rect.width - 2.0,
        commit_height,
    );
    let list_rect = Rect::new(
        rect.x + 1.0,
        header.bottom(),
        rect.width - 2.0,
        (commit_rect.y - header.bottom()).max(0.0),
    );
    scene.rect(0, list_rect, rect, theme.panel);
    let unstaged = snapshot
        .working
        .files
        .iter()
        .filter(|file| file.unstaged.is_some())
        .collect::<Vec<_>>();
    let staged = snapshot
        .working
        .files
        .iter()
        .filter(|file| file.staged.is_some())
        .collect::<Vec<_>>();
    let sections = section_layout(state, rect);

    let unstaged_scroll = state
        .wip_unstaged_scroll
        .min((sections.unstaged_content - sections.unstaged_view.height).max(0.0));
    section_header(
        scene,
        state,
        theme,
        sections.unstaged_header,
        "Unstaged Files",
        &unstaged,
        false,
    );
    file_rows(
        scene,
        state,
        theme,
        sections.unstaged_view,
        &unstaged,
        false,
        unstaged_scroll,
    );
    scrollbar(
        scene,
        sections.unstaged_view,
        sections.unstaged_content,
        unstaged_scroll,
        ScrollTarget::WipUnstaged,
        theme,
    );

    let staged_scroll = state
        .wip_staged_scroll
        .min((sections.staged_content - sections.staged_view.height).max(0.0));
    section_header(
        scene,
        state,
        theme,
        sections.staged_header,
        "Staged Files",
        &staged,
        true,
    );
    file_rows(
        scene,
        state,
        theme,
        sections.staged_view,
        &staged,
        true,
        staged_scroll,
    );
    scrollbar(
        scene,
        sections.staged_view,
        sections.staged_content,
        staged_scroll,
        ScrollTarget::WipStaged,
        theme,
    );

    build_commit_form(
        scene,
        state,
        theme,
        commit_rect,
        staged.len(),
        unstaged.len(),
    );
}

/// Draws one pinned section header with its bulk staging button.
fn section_header(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    header: Rect,
    title: &str,
    files: &[&WorkingFile],
    staged: bool,
) {
    let inner = Rect::new(header.x + 9.0, header.y, header.width - 18.0, header.height);
    scene.rect(1, header, scene.viewport(), theme.panel_alt);
    scene.rect(
        1,
        Rect::new(header.x, header.bottom() - 1.0, header.width, 1.0),
        scene.viewport(),
        theme.border,
    );
    let label = title.to_uppercase();
    let label_width = label.chars().count() as f32 * 6.4;
    scene.text(
        &label,
        [inner.x + 3.0, inner.y + 8.0],
        Rect::new(inner.x, inner.y, inner.width - 132.0, inner.height),
        theme.text_muted,
        12.0,
        16.0,
        FontFace::Sans,
    );
    let count = files.len().to_string();
    let pill = Rect::new(
        inner.x + 3.0 + label_width + 8.0,
        inner.y + 6.0,
        count.chars().count() as f32 * 6.0 + 12.0,
        16.0,
    );
    scene.rounded_rect(2, pill, header, theme.panel, theme.border, 8.0, 1.0);
    scene.text(
        &count,
        [pill.x + 6.0, pill.y + 2.0],
        pill,
        theme.text_muted,
        10.0,
        12.0,
        FontFace::Monospace,
    );
    let selected_count = state.selected_working_files.len();
    let (label, action) = if staged {
        ("Unstage All".to_owned(), UiAction::UnstageAll)
    } else if selected_count > 1 {
        (
            format!("Stage {selected_count} Files"),
            UiAction::StageSelection,
        )
    } else {
        ("Stage All Changes".to_owned(), UiAction::StageAll)
    };
    button(
        scene,
        Rect::new(inner.right() - 128.0, inner.y + 2.0, 126.0, 26.0),
        &label,
        action,
        state.hover(),
        theme,
        false,
        !files.is_empty(),
        None,
    );
}

/// Draws the scrolled file rows of one section, clipped to its viewport.
fn file_rows(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    view: Rect,
    files: &[&WorkingFile],
    staged: bool,
    scroll: f32,
) {
    if files.is_empty() {
        let empty = Rect::new(
            view.x + 16.0,
            view.y,
            view.width - 32.0,
            EMPTY_CONTENT_HEIGHT,
        );
        if let Some(visible_empty) = empty.intersection(view) {
            scene.text(
                if staged {
                    "No staged files"
                } else {
                    "Working directory clean"
                },
                [empty.x + 6.0, empty.y + 6.0],
                visible_empty,
                theme.text_muted,
                13.0,
                16.0,
                FontFace::Sans,
            );
        }
        return;
    }
    let mut y = view.y - scroll;
    for file in files {
        let row = Rect::new(view.x + 4.0, y, view.width - 8.0, ROW_HEIGHT);
        y += ROW_HEIGHT;
        let Some(visible_row) = row.intersection(view) else {
            continue;
        };
        let selected = state.selected_working_files.contains(&file.path);
        let open = state
            .selected_file
            .as_ref()
            .is_some_and(|request| request.path == file.path);
        let wash = Rect::new(row.x, row.y + 1.0, row.width, row.height - 2.0);
        if open || selected {
            scene.rounded_rect(
                1,
                wash,
                view,
                theme.row_selected,
                theme.row_selected,
                RADIUS_SM,
                0.0,
            );
        } else if row.contains(state.hover()) {
            scene.rounded_rect(1, wash, view, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
        }
        scene.hit_clipped(
            visible_row,
            view,
            UiAction::SelectFile {
                path: file.path.clone(),
                staged,
                commit: None,
            },
            CursorHint::Pointer,
            None,
        );
        let check = Rect::new(row.x + 4.0, row.y + 3.5, 17.0, 17.0);
        scene.rounded_rect(
            2,
            check,
            view,
            if selected { theme.accent } else { theme.input },
            if selected {
                theme.accent
            } else {
                theme.border_strong
            },
            5.0,
            1.0,
        );
        if selected {
            scene.text(
                icons::CHECK,
                [check.x + 3.0, check.y],
                check.intersection(view).unwrap_or(visible_row),
                theme.on_accent,
                12.0,
                15.0,
                FontFace::Sans,
            );
        }
        scene.hit_clipped(
            check,
            view,
            UiAction::ToggleFileSelection(file.path.clone()),
            CursorHint::Pointer,
            Some("Select for bulk staging"),
        );
        let kind = if staged { file.staged } else { file.unstaged }.unwrap_or(ChangeKind::Modified);
        let label = working_path_label(file, state.path_tree);
        let badge = Rect::new(row.x + 26.0, row.y + 4.0, 16.0, 16.0);
        let (kind_fg, kind_bg) = change_colors(kind, theme);
        scene.rounded_rect(
            2,
            badge,
            view,
            kind_bg,
            kind_bg,
            RADIUS_SM - 1.0,
            0.0,
        );
        scene.text(
            kind.marker(),
            [badge.x + 5.0, badge.y + 2.0],
            badge.intersection(view).unwrap_or(visible_row),
            kind_fg,
            10.0,
            12.0,
            FontFace::Monospace,
        );
        scene.text(
            label,
            [row.x + 46.0, row.y + 4.5],
            Rect::new(row.x + 44.0, row.y, row.width - 130.0, row.height)
                .intersection(view)
                .unwrap_or(visible_row),
            if open || selected {
                theme.accent
            } else {
                theme.text
            },
            13.0,
            16.0,
            FontFace::Sans,
        );
        let action_rect = Rect::new(row.right() - 78.0, row.y + 1.5, 72.0, 21.0);
        if (action_rect.contains(state.hover()) || row.contains(state.hover()))
            && action_rect.y >= view.y
            && action_rect.bottom() <= view.bottom()
        {
            button(
                scene,
                action_rect,
                if staged { "Unstage" } else { "Stage File" },
                if staged {
                    UiAction::UnstageFile(file.path.clone())
                } else {
                    UiAction::StageFile(file.path.clone())
                },
                state.hover(),
                theme,
                false,
                true,
                None,
            );
        }
    }
}

fn build_commit_form(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    rect: Rect,
    staged_count: usize,
    unstaged_count: usize,
) {
    scene.rect(1, rect, scene.viewport(), theme.panel);
    divider(scene, Rect::new(rect.x, rect.y, rect.width, 1.0), theme);
    checkbox(
        scene,
        Rect::new(rect.x + 12.0, rect.y + 9.0, rect.width - 24.0, 29.0),
        "Amend previous commit",
        state.amend,
        UiAction::ToggleAmend,
        state.hover(),
        theme,
    );

    // One unified bordered box: summary line on top, description beneath.
    let summary_focused = state.focus == FocusField::CommitSummary;
    let body_focused = state.focus == FocusField::CommitBody;
    let box_height = (rect.height - 134.0).clamp(60.0, 128.0);
    let unified = Rect::new(rect.x + 13.0, rect.y + 42.0, rect.width - 26.0, box_height);
    scene.rounded_rect(
        1,
        unified,
        scene.viewport(),
        theme.input,
        if summary_focused || body_focused {
            theme.accent
        } else {
            theme.border_strong
        },
        RADIUS_MD,
        1.0,
    );
    let summary = Rect::new(unified.x, unified.y, unified.width, 34.0);
    let description = Rect::new(
        unified.x,
        summary.bottom(),
        unified.width,
        unified.height - summary.height,
    );
    // The focused part gets the accent treatment: a slim accent bar inside the shared border.
    if summary_focused {
        scene.rect(
            2,
            Rect::new(summary.x + 1.5, summary.y + 5.0, 2.0, summary.height - 9.0),
            unified,
            theme.accent,
        );
    } else if body_focused {
        scene.rect(
            2,
            Rect::new(
                description.x + 1.5,
                description.y + 4.0,
                2.0,
                description.height - 9.0,
            ),
            unified,
            theme.accent,
        );
    }
    let summary_count = state.commit_summary.chars().count();
    scene.text(
        if state.commit_summary.is_empty() {
            "Commit summary"
        } else {
            state.commit_summary.text()
        },
        [summary.x + 9.0, summary.y + 8.0],
        Rect::new(
            summary.x + 2.0,
            summary.y,
            summary.width - 72.0,
            summary.height,
        ),
        if state.commit_summary.is_empty() {
            theme.text_dim
        } else {
            theme.text
        },
        13.0,
        17.0,
        FontFace::Sans,
    );
    if summary_focused {
        crate::ui::widgets::caret_overlay(
            scene,
            3,
            [summary.x + 9.0, summary.y + 8.0],
            Rect::new(
                summary.x + 2.0,
                summary.y,
                summary.width - 74.0,
                summary.height,
            ),
            &state.commit_summary,
            7.1,
            17.0,
            theme,
        );
    }
    scene.hit(
        Rect::new(summary.x, summary.y, summary.width - 68.0, summary.height),
        UiAction::FocusCommitSummary,
        CursorHint::Text,
        None,
    );
    // Remaining-characters counter and AI sparkle live inside the box, top-right.
    let remaining = 72_i64 - summary_count.to_i64().unwrap_or(i64::MAX);
    scene.text(
        remaining.to_string(),
        [summary.right() - 58.0, summary.y + 10.0],
        Rect::new(summary.right() - 66.0, summary.y, 36.0, summary.height),
        if remaining < 0 {
            theme.red
        } else {
            theme.text_dim
        },
        10.0,
        14.0,
        FontFace::Monospace,
    );
    let sparkle = Rect::new(summary.right() - 30.0, summary.y + 5.0, 24.0, 24.0);
    if sparkle.contains(state.hover()) {
        scene.rounded_rect(2, sparkle, unified, theme.panel_alt, theme.panel_alt, RADIUS_SM, 0.0);
    }
    scene.text(
        icons::SPARKLE,
        [sparkle.x + 5.0, sparkle.y + 3.0],
        sparkle,
        theme.accent,
        13.0,
        17.0,
        FontFace::Sans,
    );
    scene.hit(
        sparkle,
        UiAction::ShowAiStatus,
        CursorHint::Pointer,
        Some("Compose commits with AI"),
    );
    scene.text(
        if state.commit_body.is_empty() {
            "Description"
        } else {
            state.commit_body.text()
        },
        [description.x + 9.0, description.y + 6.0],
        description.inset(7.0),
        if state.commit_body.is_empty() {
            theme.text_dim
        } else {
            theme.text
        },
        13.0,
        19.0,
        FontFace::Sans,
    );
    if body_focused {
        crate::ui::widgets::caret_overlay(
            scene,
            3,
            [description.x + 9.0, description.y + 6.0],
            description.inset(2.0),
            &state.commit_body,
            7.1,
            19.0,
            theme,
        );
    }
    scene.hit(
        description,
        UiAction::FocusCommitBody,
        CursorHint::Text,
        None,
    );

    button(
        scene,
        Rect::new(rect.x + 13.0, unified.bottom() + 8.0, 126.0, 24.0),
        format!("Commit options  {}", icons::CHEVRON_DOWN),
        UiAction::ToggleCommitOptions,
        state.hover(),
        theme,
        false,
        true,
        Some("Commit options"),
    );

    // Full-width green-bordered commit button, GitKraken style.
    let primary = Rect::new(rect.x + 13.0, rect.bottom() - 58.0, rect.width - 26.0, 42.0);
    let (label, action, enabled) = if staged_count == 0 {
        (
            if unstaged_count == 0 {
                "Nothing to Commit".to_owned()
            } else {
                "Stage Changes to Commit".to_owned()
            },
            UiAction::StageAll,
            unstaged_count > 0,
        )
    } else if state.commit_summary.trim().is_empty() {
        (
            "Type a Message to Commit".to_owned(),
            UiAction::Commit,
            false,
        )
    } else {
        (
            format!(
                "Commit changes to {staged_count} file{}",
                if staged_count == 1 { "" } else { "s" }
            ),
            UiAction::Commit,
            true,
        )
    };
    let hovered = enabled && primary.contains(state.hover());
    scene.rounded_rect(
        2,
        primary,
        rect,
        if hovered {
            theme.green_muted
        } else {
            theme.panel
        },
        if enabled {
            theme.green
        } else {
            theme.border_strong
        },
        RADIUS_MD,
        1.0,
    );
    let label_width = label.chars().count().to_f32().unwrap_or(0.0) * 6.8;
    scene.text(
        &label,
        [
            primary.x + ((primary.width - label_width) * 0.5).max(9.0),
            primary.y + 13.0,
        ],
        primary.inset(4.0),
        if enabled { theme.green } else { theme.text_dim },
        13.0,
        16.0,
        FontFace::Sans,
    );
    if enabled {
        let commit_tooltip = format!("Commit staged changes ({})", icons::KEY_COMMAND_RETURN);
        scene.hit(primary, action, CursorHint::Pointer, Some(&commit_tooltip));
    }
    if state.selected_working_files.len() > 1 {
        let badge = Rect::new(primary.right() - 25.0, primary.y - 8.0, 22.0, 22.0);
        scene.rounded_rect(3, badge, rect, theme.red, theme.red, 11.0, 0.0);
        scene.text(
            state.selected_working_files.len().to_string(),
            [badge.x + 6.0, badge.y + 3.0],
            badge,
            theme.on_accent,
            10.0,
            13.0,
            FontFace::Monospace,
        );
    }
}

fn working_path_label(file: &WorkingFile, tree: bool) -> String {
    let format_path = |path: &std::path::Path| {
        if tree {
            path.components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join(icons::BREADCRUMB_SEPARATOR)
        } else {
            path.display().to_string()
        }
    };
    let path = format_path(&file.path);
    match &file.old_path {
        Some(old) => format!("{}  {}  {path}", format_path(old), icons::ARROW_RIGHT),
        None => path,
    }
}

fn change_colors(kind: ChangeKind, theme: &Theme) -> (crate::ui::Color, crate::ui::Color) {
    match kind {
        ChangeKind::Added => (theme.green, theme.green_muted),
        ChangeKind::Deleted | ChangeKind::Conflicted => (theme.red, theme.red_muted),
        ChangeKind::Renamed => (theme.text_muted, theme.panel_alt),
        ChangeKind::Modified | ChangeKind::TypeChanged => (theme.orange, theme.orange_muted),
    }
}
