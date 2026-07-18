use num_traits::ToPrimitive;

use crate::{
    app::state::{AppState, FocusField, Overlay, add_remote_popup_rect},
    git::models::{DiffLineSelection, DiffRowKind, DiffScope},
    ui::{
        FontFace, Rect, Scene, Theme,
        action::{AddRemoteProvider, CursorHint, UiAction},
        geometry::px,
        icons,
        menu::{self, MenuRow},
        widgets::{button, divider, modal_button, modal_text_input, text_input},
    },
    views::Layout,
};

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, layout: &Layout) {
    match &state.overlay {
        Overlay::None => {}
        Overlay::Branches => build_branches(scene, state, theme),
        Overlay::Lfs => build_lfs(scene, state, theme, layout),
        Overlay::Actions => build_actions(scene, state, theme, layout),
        Overlay::CommitOptions => build_commit_options(scene, state, theme),
        Overlay::PullOptions => build_pull_options(scene, state, theme, layout),
        Overlay::DiffSelection => build_diff_selection(scene, state, theme),
        Overlay::Tabs => build_tabs(scene, state, theme),
        Overlay::Notifications => build_notifications(scene, state, theme),
        Overlay::CreateBranch => build_create_branch(scene, state, theme),
        Overlay::AddRemote => build_add_remote(scene, state, theme),
        Overlay::RenameBranch(branch) => build_rename_branch(scene, state, theme, branch),
        Overlay::CreateTag(target) => build_create_tag(scene, state, theme, target),
        Overlay::StashContext(_)
        | Overlay::TagContext(_)
        | Overlay::BranchContext(_)
        | Overlay::CommitContext(_)
        | Overlay::FileContext { .. }
        | Overlay::DropMenu { .. } => build_context_menu(scene, state, theme),
        Overlay::EditCommitMessage(_) => build_edit_commit_message(scene, state, theme),
        Overlay::Ai => build_ai(scene, state, theme),
        Overlay::CommandPalette => crate::views::palette::build(
            scene,
            state,
            theme,
            crate::views::palette::PaletteSkin::General,
        ),
        Overlay::EditorPalette => crate::views::palette::build(
            scene,
            state,
            theme,
            crate::views::palette::PaletteSkin::Editor,
        ),
    }
    build_global_feedback(scene, state, theme);
    if state.overlay == Overlay::None && state.error.is_none() {
        build_tooltip(scene, state, theme);
    }
    if let Some(drag) = state.dragging_ref() {
        draw_drag_ghost(scene, state, theme, drag);
    }
}

/// Floating chip that follows the pointer while a ref is being dragged.
fn draw_drag_ghost(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    drag: &crate::app::state::RefDrag,
) {
    let label = drag.source.as_str();
    let width = label.chars().count().to_f32().unwrap_or(0.0) * 6.2 + 32.0;
    let chip = Rect::new(
        (state.mouse[0] + 14.0).min(scene.width - width - 4.0),
        (state.mouse[1] + 12.0).min(scene.height - 26.0),
        width,
        20.0,
    );
    scene.rounded_rect(
        4,
        chip,
        scene.viewport(),
        theme.surface_3,
        theme.yellow,
        4.0,
        1.2,
    );
    scene.text(
        if drag.tag { icons::TAG } else { icons::BRANCH },
        [chip.x + 6.0, chip.y + 4.0],
        chip,
        theme.yellow,
        10.0,
        12.0,
        FontFace::Terminal,
    );
    scene.text(
        label,
        [chip.x + 19.0, chip.y + 3.5],
        chip,
        theme.text,
        11.0,
        14.0,
        FontFace::Sans,
    );
}

pub(super) fn build_global_feedback(scene: &mut Scene, state: &AppState, theme: &Theme) {
    if let Some(error) = &state.error {
        let backdrop = scene.viewport();
        scene.rect(4, backdrop, backdrop, theme.shadow.with_alpha(0.68));
        let modal = Rect::new(
            backdrop.width * 0.5 - 200.0,
            backdrop.height * 0.5 - 120.0,
            400.0,
            240.0,
        );
        popup_panel(scene, modal, theme);
        scene.text(
            "GIT OPERATION FAILED",
            [modal.x + 24.0, modal.y + 24.0],
            Rect::new(modal.x + 24.0, modal.y + 20.0, modal.width - 80.0, 24.0),
            theme.red,
            12.0,
            18.0,
            FontFace::Monospace,
        );
        divider(
            scene,
            Rect::new(modal.x + 24.0, modal.y + 56.0, modal.width - 48.0, 1.0),
            theme,
        );
        scene.text(
            error,
            [modal.x + 24.0, modal.y + 80.0],
            Rect::new(modal.x + 24.0, modal.y + 72.0, modal.width - 48.0, 96.0),
            theme.text_muted,
            12.0,
            17.0,
            FontFace::Monospace,
        );
        button(
            scene,
            Rect::new(modal.right() - 108.0, modal.bottom() - 52.0, 84.0, 32.0),
            "Dismiss",
            UiAction::DismissOverlay,
            state.mouse,
            theme,
            false,
            true,
            None,
        );
        let close_rect = Rect::new(modal.right() - 40.0, modal.y + 16.0, 24.0, 24.0);
        if close_rect.contains(state.mouse) {
            scene.rect(4, close_rect, modal, theme.panel_alt);
        }
        scene.text(
            icons::CLOSE,
            [close_rect.x + 8.0, close_rect.y + 2.0],
            close_rect,
            theme.text_muted,
            16.0,
            20.0,
            FontFace::Sans,
        );
        scene.hit(
            close_rect,
            UiAction::DismissOverlay,
            CursorHint::Pointer,
            Some("Dismiss"),
        );
        scene.hit(
            backdrop,
            UiAction::DismissOverlay,
            CursorHint::Default,
            None,
        );
    } else if let Some(toast) = &state.toast {
        let rect = Rect::new(24.0, scene.height - 64.0, 320.0, 40.0);
        popup_surface(scene, rect, theme);
        scene.text(
            format!("{}  {toast}", icons::CHECK),
            [rect.x + 12.0, rect.y + 12.0],
            rect.inset(8.0),
            theme.green,
            11.5,
            16.0,
            FontFace::Monospace,
        );
    }
}

fn build_commit_options(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let actionable = state
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.working.staged_count() > 0)
        && !state.commit_summary.trim().is_empty();
    let rect = Rect::new(
        state.overlay_anchor[0].clamp(12.0, scene.width - 190.0),
        state.overlay_anchor[1].clamp(12.0, scene.height - 52.0),
        178.0,
        38.0,
    );
    popup_panel(scene, rect, theme);
    button(
        scene,
        rect.inset(4.0),
        "Commit and Push",
        UiAction::CommitAndPush,
        state.mouse,
        theme,
        false,
        actionable,
        None,
    );
}

fn build_diff_selection(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let Some(request) = &state.selected_file else {
        return;
    };
    let (lines, copied) = selected_diff_rows(state);
    if lines.is_empty() {
        return;
    }
    let staged = matches!(request.scope, DiffScope::Staged);
    let menu_height = if staged { 76.0 } else { 110.0 };
    let rect = Rect::new(
        state.overlay_anchor[0].clamp(8.0, scene.width - 210.0),
        state.overlay_anchor[1].clamp(8.0, scene.height - menu_height - 8.0),
        202.0,
        menu_height,
    );
    popup_panel(scene, rect, theme);
    let mut y = rect.y + 4.0;
    if staged {
        button(
            scene,
            Rect::new(rect.x + 4.0, y, rect.width - 8.0, 30.0),
            "Unstage selected lines",
            UiAction::UnstageDiffLines {
                path: request.path.clone(),
                lines: lines.clone(),
            },
            state.mouse,
            theme,
            false,
            true,
            None,
        );
        y += 34.0;
    } else {
        button(
            scene,
            Rect::new(rect.x + 4.0, y, rect.width - 8.0, 30.0),
            "Discard Selection",
            UiAction::DiscardDiffLines {
                path: request.path.clone(),
                lines: lines.clone(),
            },
            state.mouse,
            theme,
            false,
            true,
            None,
        );
        y += 34.0;
        button(
            scene,
            Rect::new(rect.x + 4.0, y, rect.width - 8.0, 30.0),
            "Stage selected lines",
            UiAction::StageDiffLines {
                path: request.path.clone(),
                lines: lines.clone(),
            },
            state.mouse,
            theme,
            false,
            true,
            None,
        );
        y += 34.0;
    }
    button(
        scene,
        Rect::new(rect.x + 4.0, y, rect.width - 8.0, 30.0),
        "Copy",
        UiAction::CopyDiffLines(copied),
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

fn selected_diff_rows(state: &AppState) -> (Vec<DiffLineSelection>, Vec<String>) {
    let Some(diff) = &state.diff else {
        return (Vec::new(), Vec::new());
    };
    let mut indices = state.diff_selected_rows.iter().copied().collect::<Vec<_>>();
    indices.sort_unstable();
    indices
        .into_iter()
        .filter_map(|index| diff.rows.get(index))
        .filter(|row| !matches!(row.kind, DiffRowKind::Context | DiffRowKind::Hunk))
        .map(|row| {
            (
                DiffLineSelection {
                    old_line: row.old_number,
                    new_line: row.new_number,
                },
                if row.new_text.is_empty() {
                    row.old_text.clone()
                } else {
                    row.new_text.clone()
                },
            )
        })
        .unzip()
}

fn build_branches(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = Rect::new(128.0, 48.0, 320.0, 520.0_f32.min(scene.height - 64.0));
    popup_panel(scene, rect, theme);
    let search = Rect::new(rect.x + 8.0, rect.y + 8.0, rect.width - 16.0, 32.0);
    text_input(
        scene,
        search,
        &state.branch_filter,
        "Search branches",
        state.focus == FocusField::BranchFilter,
        UiAction::FocusBranchFilter,
        state.mouse,
        theme,
        false,
    );
    let query = state.branch_filter.to_lowercase();
    let branches = state
        .snapshot
        .as_ref()
        .map(|snapshot| {
            snapshot
                .branches
                .iter()
                .filter(|branch| query.is_empty() || branch.name.to_lowercase().contains(&query))
                .take(20)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut y = search.bottom() + 12.0;
    scene.text(
        "BRANCHES",
        [rect.x + 16.0, y + 2.0],
        Rect::new(rect.x + 8.0, y, rect.width - 16.0, 16.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    y += 20.0;
    if branches.is_empty() {
        scene.text(
            "NO MATCHING BRANCHES",
            [rect.x + 16.0, y + 8.0],
            Rect::new(rect.x + 16.0, y, rect.width - 32.0, 30.0),
            theme.text_dim,
            10.0,
            14.0,
            FontFace::Monospace,
        );
    }
    for branch in branches {
        let row = Rect::new(rect.x + 8.0, y, rect.width - 16.0, 28.0);
        if branch.current {
            scene.rect(4, row, rect, theme.row_selected);
            scene.dither_rect(
                4,
                row,
                rect,
                theme.accent.with_alpha(0.25),
                crate::ui::scene::Pattern::Checker,
            );
            scene.rect(
                5,
                Rect::new(row.x, row.y, 2.0, row.height),
                rect,
                theme.accent,
            );
        } else if row.contains(state.mouse) {
            scene.rect(4, row, rect, theme.panel_alt);
        }
        scene.text(
            if branch.current {
                icons::CHECK
            } else if branch.remote {
                icons::REMOTE
            } else {
                icons::BRANCH
            },
            [row.x + 8.0, row.y + 6.0],
            row,
            if branch.current {
                theme.accent
            } else {
                theme.text_muted
            },
            12.0,
            16.0,
            FontFace::Monospace,
        );
        scene.text(
            &branch.name,
            [row.x + 28.0, row.y + 6.0],
            Rect::new(row.x + 28.0, row.y, row.width - 36.0, row.height),
            if branch.current {
                theme.accent
            } else if row.contains(state.mouse) {
                theme.text
            } else {
                theme.text_muted
            },
            13.0,
            16.0,
            FontFace::Sans,
        );
        scene.hit(
            row,
            UiAction::CheckoutBranch(branch.name.clone()),
            CursorHint::Pointer,
            None,
        );
        y += 28.0;
    }
}

fn build_lfs(scene: &mut Scene, state: &AppState, theme: &Theme, layout: &Layout) {
    let rect = Rect::new(
        layout.toolbar.right() - 280.0,
        layout.toolbar.bottom() + 4.0,
        240.0,
        168.0,
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "LFS COMMANDS",
        [rect.x + 16.0, rect.y + 16.0],
        rect.inset(10.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    divider(
        scene,
        Rect::new(rect.x + 8.0, rect.y + 40.0, rect.width - 16.0, 1.0),
        theme,
    );
    let entries = [
        ("Checkout all LFS files", UiAction::LfsCheckout),
        ("Pull all LFS files", UiAction::LfsPull),
        ("Push all LFS files", UiAction::LfsPush),
        ("Prune local LFS", UiAction::LfsPrune),
    ];
    let mut y = rect.y + 48.0;
    for (entry, action) in entries {
        let row = Rect::new(rect.x + 8.0, y, rect.width - 16.0, 28.0);
        if row.contains(state.mouse) {
            scene.rect(4, row, rect, theme.panel_alt);
        }
        scene.text(
            entry,
            [row.x + 8.0, row.y + 6.0],
            row,
            if row.contains(state.mouse) {
                theme.text
            } else {
                theme.text_muted
            },
            13.0,
            16.0,
            FontFace::Sans,
        );
        scene.hit(row, action, CursorHint::Pointer, None);
        y += 28.0;
    }
}

fn build_pull_options(scene: &mut Scene, state: &AppState, theme: &Theme, layout: &Layout) {
    let rect = Rect::new(
        layout.toolbar.x + 520.0_f32.min(layout.toolbar.width * 0.42) + 104.0,
        layout.toolbar.bottom() + 4.0,
        310.0,
        190.0,
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "Select a default pull/fetch operation to execute when clicking this button",
        [rect.x + 12.0, rect.y + 12.0],
        Rect::new(rect.x + 12.0, rect.y + 10.0, rect.width - 24.0, 42.0),
        theme.text,
        12.0,
        16.0,
        FontFace::Sans,
    );
    let entries = [
        ("Fetch All", crate::git::models::PullOperation::FetchAll),
        (
            "Pull (fast-forward if possible)",
            crate::git::models::PullOperation::FastForward,
        ),
        (
            "Pull (fast-forward only)",
            crate::git::models::PullOperation::FastForwardOnly,
        ),
        ("Pull (rebase)", crate::git::models::PullOperation::Rebase),
    ];
    let mut y = rect.y + 58.0;
    for (label, operation) in entries {
        let row = Rect::new(rect.x + 8.0, y, rect.width - 16.0, 30.0);
        if row.contains(state.mouse) {
            scene.rect(4, row, rect, theme.panel_alt);
        }
        let selected = state.settings.default_pull_operation == operation;
        scene.text(
            if selected {
                icons::CIRCLE_FILLED
            } else {
                icons::CIRCLE
            },
            [row.x + 8.0, row.y + 7.0],
            row,
            if selected {
                theme.accent
            } else {
                theme.text_muted
            },
            13.0,
            16.0,
            FontFace::Sans,
        );
        scene.text(
            label,
            [row.x + 30.0, row.y + 7.0],
            row,
            theme.text,
            12.0,
            16.0,
            FontFace::Sans,
        );
        scene.hit(
            row,
            UiAction::SetPullOperation(operation),
            CursorHint::Pointer,
            None,
        );
        y += 30.0;
    }
}

fn build_actions(scene: &mut Scene, state: &AppState, theme: &Theme, layout: &Layout) {
    let rect = Rect::new(
        layout.toolbar.right() - 240.0,
        layout.toolbar.bottom() + 4.0,
        224.0,
        164.0,
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "REPOSITORY ACTIONS",
        [rect.x + 16.0, rect.y + 16.0],
        rect.inset(10.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    divider(
        scene,
        Rect::new(rect.x + 8.0, rect.y + 40.0, rect.width - 16.0, 1.0),
        theme,
    );
    let actions = [
        (
            format!("{}  Fetch all remotes", icons::FETCH),
            UiAction::Fetch,
        ),
        (
            format!("{}  Create branch", icons::BRANCH),
            UiAction::ToggleCreateBranch,
        ),
        (
            format!("{}  Stash changes", icons::ARCHIVE),
            UiAction::Stash,
        ),
    ];
    let mut y = rect.y + 48.0;
    for (label, action) in actions {
        let row = Rect::new(rect.x + 8.0, y, rect.width - 16.0, 32.0);
        if row.contains(state.mouse) {
            scene.rect(4, row, rect, theme.panel_alt);
        }
        scene.text(
            label,
            [row.x + 8.0, row.y + 8.0],
            row,
            if row.contains(state.mouse) {
                theme.text
            } else {
                theme.text_muted
            },
            13.0,
            16.0,
            FontFace::Sans,
        );
        scene.hit(row, action, CursorHint::Pointer, None);
        y += 34.0;
    }
}

fn build_tabs(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = Rect::new(scene.width - 320.0, 48.0, 300.0, 240.0);
    popup_panel(scene, rect, theme);
    scene.text(
        "OPEN TABS",
        [rect.x + 16.0, rect.y + 16.0],
        rect.inset(10.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    let search = Rect::new(rect.x + 8.0, rect.y + 40.0, rect.width - 16.0, 32.0);
    text_input(
        scene,
        search,
        &state.tab_filter,
        "Search Tabs...",
        state.focus == FocusField::TabFilter,
        UiAction::FocusTabFilter,
        state.mouse,
        theme,
        false,
    );
    let mut y = rect.y + 84.0;
    for (index, tab) in state.tabs.iter().enumerate().filter(|(_, tab)| {
        state.tab_filter.is_empty()
            || tab
                .title
                .to_lowercase()
                .contains(&state.tab_filter.to_lowercase())
    }) {
        if y + 32.0 > rect.bottom() - 12.0 {
            break;
        }
        let row = Rect::new(rect.x + 8.0, y, rect.width - 16.0, 32.0);
        let active = index == state.active_tab;
        if active || row.contains(state.mouse) {
            scene.rect(
                4,
                row,
                rect,
                if active {
                    theme.accent_soft
                } else {
                    theme.panel_alt
                },
            );
        }
        if active {
            scene.rect(
                5,
                Rect::new(row.x, row.y, 2.0, row.height),
                rect,
                theme.accent,
            );
        }
        let icon = if tab.path.is_some() {
            icons::REPOSITORY
        } else {
            icons::HOME
        };
        scene.text(
            format!("{icon}  {}", tab.title),
            [row.x + 10.0, row.y + 8.0],
            Rect::new(row.x + 10.0, row.y, row.width - 40.0, row.height),
            if active { theme.accent } else { theme.text },
            13.0,
            16.0,
            FontFace::Sans,
        );
        let close = Rect::new(row.right() - 28.0, row.y + 4.0, 24.0, 24.0);
        scene.text(
            icons::CLOSE,
            [close.x + 8.0, close.y + 2.0],
            close,
            theme.text_muted,
            14.0,
            20.0,
            FontFace::Sans,
        );
        scene.hit(row, UiAction::SelectTab(index), CursorHint::Pointer, None);
        scene.hit(
            close,
            UiAction::CloseTab(index),
            CursorHint::Pointer,
            Some("Close tab"),
        );
        y += 34.0;
    }
    scene.hit(rect, UiAction::DismissOverlay, CursorHint::Default, None);
}

fn build_notifications(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = Rect::new(scene.width - 380.0, 48.0, 360.0, 400.0);
    popup_panel(scene, rect, theme);
    scene.text(
        "NOTIFICATIONS",
        [rect.x + 20.0, rect.y + 20.0],
        Rect::new(rect.x, rect.y, rect.width, 40.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    scene.text(
        format!("ALL  {}", icons::CHEVRON_DOWN),
        [rect.right() - 52.0, rect.y + 22.0],
        Rect::new(rect.right() - 64.0, rect.y, 64.0, 40.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    divider(
        scene,
        Rect::new(rect.x, rect.y + 52.0, rect.width, 1.0),
        theme,
    );
    let notices = [
        (
            "Kraken Native 0.1.0",
            "Native wgpu renderer, real graph, staging, and Git operations are available.",
        ),
        (
            "AI provider",
            "No AI usage is fabricated. Configure an endpoint and API key in the environment.",
        ),
        (
            "Repository",
            "Git operations report their real libgit2 result in the status bar.",
        ),
    ];
    let mut y = rect.y + 64.0;
    for (title, body) in notices {
        let card = Rect::new(rect.x + 12.0, y, rect.width - 24.0, 96.0);
        scene.rounded_rect(
            4,
            card,
            rect,
            if card.contains(state.mouse) {
                theme.surface_3
            } else {
                theme.panel_alt
            },
            theme.border_strong,
            0.0,
            1.0,
        );
        scene.text(
            title,
            [card.x + 16.0, card.y + 12.0],
            Rect::new(card.x + 16.0, card.y + 12.0, card.width - 32.0, 24.0),
            theme.text,
            12.0,
            18.0,
            FontFace::Sans,
        );
        scene.text(
            body,
            [card.x + 16.0, card.y + 36.0],
            Rect::new(card.x + 16.0, card.y + 36.0, card.width - 32.0, 60.0),
            theme.text_muted,
            11.5,
            16.0,
            FontFace::Sans,
        );
        y += 108.0;
    }
    scene.hit(rect, UiAction::DismissOverlay, CursorHint::Default, None);
}

fn build_create_branch(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = Rect::new(
        scene.width * 0.5 - 200.0,
        scene.height * 0.5 - 100.0,
        400.0,
        200.0,
    );
    scene.rect(
        3,
        scene.viewport(),
        scene.viewport(),
        theme.shadow.with_alpha(0.68),
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "CREATE A BRANCH",
        [rect.x + 24.0, rect.y + 24.0],
        Rect::new(rect.x + 24.0, rect.y + 24.0, rect.width - 48.0, 30.0),
        theme.text,
        12.0,
        18.0,
        FontFace::Monospace,
    );
    let input = Rect::new(rect.x + 24.0, rect.y + 68.0, rect.width - 48.0, 36.0);
    modal_text_input(
        scene,
        input,
        &state.new_branch,
        "Branch name",
        state.focus == FocusField::CreateBranch,
        UiAction::FocusCreateBranch,
        state.mouse,
        theme,
        false,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 132.0, rect.bottom() - 56.0, 108.0, 32.0),
        "Create Branch",
        UiAction::CreateBranch,
        state.mouse,
        theme,
        true,
        !state.new_branch.trim().is_empty(),
        None,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 224.0, rect.bottom() - 56.0, 80.0, 32.0),
        "Cancel",
        UiAction::DismissOverlay,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

/// Provider tab strip entries for the Add Remote form: provider, NF glyph,
/// and display label.
const ADD_REMOTE_PROVIDERS: [(AddRemoteProvider, &str, &str); 3] = [
    (AddRemoteProvider::Url, icons::GLOBE, "URL"),
    (AddRemoteProvider::GitHub, icons::GITHUB, "GitHub"),
    (AddRemoteProvider::Gitea, icons::GITEA, "Gitea"),
];

/// GitKraken-style Add Remote form: provider tabs (URL, GitHub, Gitea) over
/// per-provider fields; submits via [`UiAction::AddRemote`].
fn build_add_remote(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = add_remote_popup_rect(scene.width, scene.height);
    scene.rect(
        3,
        scene.viewport(),
        scene.viewport(),
        theme.shadow.with_alpha(0.68),
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "ADD REMOTE",
        [rect.x + 24.0, rect.y + 20.0],
        Rect::new(rect.x + 24.0, rect.y + 20.0, rect.width - 48.0, 24.0),
        theme.text,
        12.0,
        18.0,
        FontFace::Monospace,
    );
    let mut tab_x = rect.x + 24.0;
    for (provider, icon, label) in ADD_REMOTE_PROVIDERS {
        let tab = Rect::new(tab_x, rect.y + 50.0, 72.0, 52.0);
        tab_x += 76.0;
        let active = state.add_remote_provider == provider;
        let hovered = tab.contains(state.mouse);
        let color = if active {
            theme.text
        } else if hovered {
            theme.text_muted
        } else {
            theme.text_dim
        };
        scene.text(
            icon,
            [tab.x + (tab.width - 16.0 * 0.6) * 0.5, tab.y + 6.0],
            tab,
            color,
            16.0,
            20.0,
            FontFace::Icons,
        );
        let label_width = px(label.chars().count()) * 10.0 * 0.52;
        scene.text(
            label,
            [tab.x + (tab.width - label_width) * 0.5, tab.y + 30.0],
            tab,
            color,
            10.0,
            14.0,
            FontFace::Sans,
        );
        if active {
            scene.rect(
                4,
                Rect::new(tab.x + 12.0, tab.bottom() - 2.0, tab.width - 24.0, 2.0),
                rect,
                theme.accent,
            );
        }
        scene.hit(
            tab,
            UiAction::SelectAddRemoteProvider(provider),
            CursorHint::Pointer,
            None,
        );
    }
    scene.rect(
        4,
        Rect::new(rect.x + 1.0, rect.y + 106.0, rect.width - 2.0, 1.0),
        rect,
        theme.border,
    );
    let fields: Vec<(&str, &str, &crate::ui::TextField, FocusField, UiAction)> =
        match state.add_remote_provider {
            AddRemoteProvider::Url => vec![
                (
                    "Remote Name",
                    "origin",
                    &state.add_remote_name,
                    FocusField::AddRemoteName,
                    UiAction::FocusAddRemoteName,
                ),
                (
                    "Pull URL",
                    "https://host.xz/user/repo.git",
                    &state.add_remote_url,
                    FocusField::AddRemoteUrl,
                    UiAction::FocusAddRemoteUrl,
                ),
                (
                    "Push URL (optional)",
                    "Defaults to pull URL",
                    &state.add_remote_push_url,
                    FocusField::AddRemotePushUrl,
                    UiAction::FocusAddRemotePushUrl,
                ),
            ],
            AddRemoteProvider::GitHub => vec![
                (
                    "Repository",
                    "owner/repo",
                    &state.add_remote_repo,
                    FocusField::AddRemoteRepo,
                    UiAction::FocusAddRemoteRepo,
                ),
                (
                    "Remote Name",
                    "origin",
                    &state.add_remote_name,
                    FocusField::AddRemoteName,
                    UiAction::FocusAddRemoteName,
                ),
            ],
            AddRemoteProvider::Gitea => vec![
                (
                    "Host",
                    "https://gitea.example.com",
                    &state.add_remote_host,
                    FocusField::AddRemoteHost,
                    UiAction::FocusAddRemoteHost,
                ),
                (
                    "Repository",
                    "owner/repo",
                    &state.add_remote_repo,
                    FocusField::AddRemoteRepo,
                    UiAction::FocusAddRemoteRepo,
                ),
                (
                    "Remote Name",
                    "origin",
                    &state.add_remote_name,
                    FocusField::AddRemoteName,
                    UiAction::FocusAddRemoteName,
                ),
            ],
        };
    let mut y = rect.y + 122.0;
    for (label, placeholder, value, focus, action) in fields {
        scene.text(
            label,
            [rect.x + 24.0, y],
            Rect::new(rect.x + 24.0, y, rect.width - 48.0, 14.0),
            theme.text_muted,
            10.0,
            14.0,
            FontFace::Sans,
        );
        modal_text_input(
            scene,
            Rect::new(rect.x + 24.0, y + 16.0, rect.width - 48.0, 32.0),
            value,
            placeholder,
            state.focus == focus,
            action,
            state.mouse,
            theme,
            false,
        );
        y += 62.0;
    }
    modal_button(
        scene,
        Rect::new(rect.right() - 136.0, rect.bottom() - 52.0, 112.0, 32.0),
        "Add Remote",
        UiAction::AddRemote,
        state.mouse,
        theme,
        true,
        state.add_remote_submission().is_some(),
        None,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 228.0, rect.bottom() - 52.0, 80.0, 32.0),
        "Cancel",
        UiAction::DismissOverlay,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

fn build_rename_branch(scene: &mut Scene, state: &AppState, theme: &Theme, branch: &str) {
    let rect = Rect::new(
        scene.width * 0.5 - 200.0,
        scene.height * 0.5 - 100.0,
        400.0,
        200.0,
    );
    scene.rect(
        3,
        scene.viewport(),
        scene.viewport(),
        theme.shadow.with_alpha(0.68),
    );
    popup_panel(scene, rect, theme);
    scene.text(
        format!("RENAME {branch}"),
        [rect.x + 24.0, rect.y + 24.0],
        Rect::new(rect.x + 24.0, rect.y + 24.0, rect.width - 48.0, 30.0),
        theme.text,
        12.0,
        18.0,
        FontFace::Monospace,
    );
    modal_text_input(
        scene,
        Rect::new(rect.x + 24.0, rect.y + 68.0, rect.width - 48.0, 36.0),
        &state.renamed_branch,
        "New branch name",
        state.focus == FocusField::RenameBranch,
        UiAction::FocusRenameBranch,
        state.mouse,
        theme,
        false,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 132.0, rect.bottom() - 56.0, 108.0, 32.0),
        "Rename",
        UiAction::RenameBranch,
        state.mouse,
        theme,
        true,
        !state.renamed_branch.trim().is_empty(),
        None,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 224.0, rect.bottom() - 56.0, 80.0, 32.0),
        "Cancel",
        UiAction::DismissOverlay,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

fn build_create_tag(scene: &mut Scene, state: &AppState, theme: &Theme, target: &str) {
    let rect = Rect::new(
        scene.width * 0.5 - 200.0,
        scene.height * 0.5 - 128.0,
        400.0,
        256.0,
    );
    scene.rect(
        3,
        scene.viewport(),
        scene.viewport(),
        theme.shadow.with_alpha(0.68),
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "CREATE TAG",
        [rect.x + 24.0, rect.y + 20.0],
        Rect::new(rect.x + 24.0, rect.y + 20.0, rect.width - 48.0, 24.0),
        theme.text,
        12.0,
        18.0,
        FontFace::Monospace,
    );
    scene.text(
        target,
        [rect.x + 24.0, rect.y + 44.0],
        Rect::new(rect.x + 24.0, rect.y + 44.0, rect.width - 48.0, 20.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    modal_text_input(
        scene,
        Rect::new(rect.x + 24.0, rect.y + 76.0, rect.width - 48.0, 36.0),
        &state.tag_name,
        "Tag name",
        state.focus == FocusField::CreateTagName,
        UiAction::FocusCreateTagName,
        state.mouse,
        theme,
        false,
    );
    modal_text_input(
        scene,
        Rect::new(rect.x + 24.0, rect.y + 124.0, rect.width - 48.0, 36.0),
        &state.tag_message,
        "Message (optional)",
        state.focus == FocusField::CreateTagMessage,
        UiAction::FocusCreateTagMessage,
        state.mouse,
        theme,
        false,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 132.0, rect.bottom() - 56.0, 108.0, 32.0),
        "Create Tag",
        UiAction::CreateTag,
        state.mouse,
        theme,
        true,
        !state.tag_name.trim().is_empty(),
        None,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 224.0, rect.bottom() - 56.0, 80.0, 32.0),
        "Cancel",
        UiAction::DismissOverlay,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

fn build_ai(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = Rect::new(
        scene.width * 0.5 - 280.0,
        scene.height * 0.5 - 180.0,
        560.0,
        360.0,
    );
    scene.rect(
        3,
        scene.viewport(),
        scene.viewport(),
        theme.shadow.with_alpha(0.68),
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "GITKRAKEN AI",
        [rect.x + 24.0, rect.y + 24.0],
        Rect::new(rect.x + 24.0, rect.y + 24.0, rect.width - 48.0, 30.0),
        theme.text,
        12.0,
        18.0,
        FontFace::Monospace,
    );
    divider(
        scene,
        Rect::new(rect.x, rect.y + 60.0, rect.width, 1.0),
        theme,
    );
    let message = if state.ai_loading {
        format!("{}  Waiting for the configured provider…", icons::LOADING)
    } else {
        state.ai_message.clone().unwrap_or_else(|| {
            "No response. Configure a provider in Preferences > GitKraken AI.".to_owned()
        })
    };
    scene.text(
        message,
        [rect.x + 24.0, rect.y + 84.0],
        Rect::new(rect.x + 24.0, rect.y + 84.0, rect.width - 48.0, 220.0),
        if state.ai_loading {
            theme.accent
        } else {
            theme.text_muted
        },
        12.0,
        18.0,
        FontFace::Sans,
    );
    button(
        scene,
        Rect::new(rect.right() - 104.0, rect.bottom() - 52.0, 80.0, 32.0),
        "Close",
        UiAction::DismissOverlay,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
    let close_rect = Rect::new(rect.right() - 44.0, rect.y + 16.0, 28.0, 28.0);
    if close_rect.contains(state.mouse) {
        scene.rect(4, close_rect, rect, theme.panel_alt);
    }
    scene.text(
        icons::CLOSE,
        [close_rect.x + 10.0, close_rect.y + 4.0],
        close_rect,
        theme.text_muted,
        16.0,
        20.0,
        FontFace::Sans,
    );
    scene.hit(
        close_rect,
        UiAction::DismissOverlay,
        CursorHint::Pointer,
        Some("Close"),
    );
}

fn build_tooltip(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let Some(text) = state.tooltip() else {
        return;
    };
    if text.is_empty() {
        return;
    }
    let estimated = text.chars().count().to_f32().unwrap_or(0.0) * 6.6 + 24.0;
    let width = estimated.clamp(80.0, 560.0);
    let lines = (estimated / (width - 24.0)).ceil().max(1.0);
    let height = 16.0 + lines * 15.0;
    let x = (state.mouse[0] + 16.0).min(scene.width - width - 8.0);
    let y = (state.mouse[1] + 24.0).min(scene.height - height - 10.0);
    let rect = Rect::new(x, y, width, height);
    popup_surface(scene, rect, theme);
    scene.text(
        text,
        [rect.x + 12.0, rect.y + 8.0],
        rect.inset(4.0),
        theme.text,
        10.5,
        15.0,
        FontFace::Sans,
    );
}

/// Renders the active right-click menu from its state-derived spec: an opaque
/// panel with hover rows, disabled rows, separators, and a one-level submenu
/// that expands while the pointer rests on its parent row.
fn build_context_menu(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let Some(spec) = state.context_menu() else {
        return;
    };
    let layout = menu::layout(
        &spec,
        state.overlay_anchor,
        [scene.width, scene.height],
        state.mouse,
    );
    menu_panel(scene, layout.panel, theme);
    scene.text(
        layout.title,
        [layout.panel.x + 12.0, layout.panel.y + 7.0],
        Rect::new(
            layout.panel.x + 12.0,
            layout.panel.y + 4.0,
            layout.panel.width - 24.0,
            18.0,
        ),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    divider(
        scene,
        Rect::new(
            layout.panel.x + 6.0,
            layout.panel.y + 25.0,
            layout.panel.width - 12.0,
            1.0,
        ),
        theme,
    );
    for row in &layout.rows {
        match row {
            MenuRow::Separator { rect } => divider(
                scene,
                Rect::new(
                    rect.x + 2.0,
                    rect.y + rect.height * 0.5,
                    rect.width - 4.0,
                    1.0,
                ),
                theme,
            ),
            MenuRow::Item {
                rect,
                label,
                action,
                enabled,
            } => menu_row(
                scene,
                state,
                theme,
                *rect,
                label,
                Some((*action).clone()),
                *enabled,
            ),
            MenuRow::Parent { rect, label, open } => {
                if *open {
                    scene.rect(4, *rect, layout.panel, theme.panel_alt);
                }
                scene.text(
                    *label,
                    [rect.x + 8.0, rect.y + 5.0],
                    *rect,
                    if *open { theme.text } else { theme.text_muted },
                    12.5,
                    16.0,
                    FontFace::Sans,
                );
                scene.text(
                    icons::CHEVRON_RIGHT,
                    [rect.right() - 18.0, rect.y + 5.0],
                    *rect,
                    theme.text_dim,
                    12.0,
                    16.0,
                    FontFace::Sans,
                );
            }
        }
    }
    if let Some(submenu) = &layout.submenu {
        menu_panel(scene, submenu.panel, theme);
        for (rect, label, action) in &submenu.rows {
            menu_row(
                scene,
                state,
                theme,
                *rect,
                label,
                Some((*action).clone()),
                true,
            );
        }
    }
}

/// One hoverable, clickable (when enabled) context-menu row.
fn menu_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    rect: Rect,
    label: &str,
    action: Option<UiAction>,
    enabled: bool,
) {
    let hovered = enabled && rect.contains(state.mouse);
    if hovered {
        scene.rect(4, rect, scene.viewport(), theme.panel_alt);
    }
    scene.text(
        label,
        [rect.x + 8.0, rect.y + 5.0],
        rect,
        if !enabled {
            theme.text_disabled
        } else if hovered {
            theme.text
        } else {
            theme.text_muted
        },
        12.5,
        16.0,
        FontFace::Sans,
    );
    if enabled && let Some(action) = action {
        scene.hit(rect, action, CursorHint::Pointer, None);
    }
}

/// Opaque elevated surface for context menus; unlike the frosted popup panel
/// nothing bleeds through from the layers beneath.
fn menu_panel(scene: &mut Scene, rect: Rect, theme: &Theme) {
    scene.mask_hits(rect);
    scene.mask_text(rect);
    scene.rounded_rect(
        4,
        rect,
        scene.viewport(),
        theme.surface_3,
        theme.border_strong,
        6.0,
        1.0,
    );
}

/// Modal editor amending the HEAD commit's message.
fn build_edit_commit_message(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let rect = Rect::new(
        scene.width * 0.5 - 210.0,
        scene.height * 0.5 - 130.0,
        420.0,
        260.0,
    );
    scene.rect(
        3,
        scene.viewport(),
        scene.viewport(),
        theme.shadow.with_alpha(0.68),
    );
    popup_panel(scene, rect, theme);
    scene.text(
        "EDIT COMMIT MESSAGE",
        [rect.x + 24.0, rect.y + 22.0],
        Rect::new(rect.x + 24.0, rect.y + 22.0, rect.width - 48.0, 24.0),
        theme.text,
        12.0,
        18.0,
        FontFace::Monospace,
    );
    modal_text_input(
        scene,
        Rect::new(rect.x + 24.0, rect.y + 56.0, rect.width - 48.0, 34.0),
        &state.edit_summary,
        "Commit summary",
        state.focus == FocusField::EditMessageSummary,
        UiAction::FocusEditMessageSummary,
        state.mouse,
        theme,
        false,
    );
    modal_text_input(
        scene,
        Rect::new(rect.x + 24.0, rect.y + 98.0, rect.width - 48.0, 92.0),
        &state.edit_body,
        "Extended description (optional)",
        state.focus == FocusField::EditMessageBody,
        UiAction::FocusEditMessageBody,
        state.mouse,
        theme,
        true,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 176.0, rect.bottom() - 52.0, 152.0, 32.0),
        "Amend message",
        UiAction::ConfirmEditMessage,
        state.mouse,
        theme,
        true,
        !state.edit_summary.trim().is_empty(),
        None,
    );
    modal_button(
        scene,
        Rect::new(rect.right() - 264.0, rect.bottom() - 52.0, 80.0, 32.0),
        "Cancel",
        UiAction::DismissOverlay,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

pub(super) fn popup_panel(scene: &mut Scene, rect: Rect, theme: &Theme) {
    scene.mask_hits(rect);
    scene.mask_text(rect);
    popup_surface(scene, rect, theme);
}

fn popup_surface(scene: &mut Scene, rect: Rect, theme: &Theme) {
    scene.frosted_rounded_rect(
        4,
        rect,
        scene.viewport(),
        theme.surface_3,
        theme.border_strong,
        4.0,
        1.0,
    );
}
