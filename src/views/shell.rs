use num_traits::ToPrimitive;
use std::collections::BTreeMap;

use crate::{
    app::state::{AppState, ToolbarOp},
    git::models::BranchInfo,
    ui::{
        Color, FontFace, RADIUS_MD, RADIUS_SM, Rect, Scene, Theme,
        action::{CursorHint, ResizeTarget, ScrollTarget, UiAction},
        icons,
        widgets::{divider, scrollbar, truncated_text},
    },
    views::Layout,
};

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, layout: &Layout) {
    scene.rect(0, scene.viewport(), scene.viewport(), theme.window);
    build_tabs(scene, state, theme, layout.tabs);
    if state
        .tabs
        .get(state.active_tab)
        .is_some_and(|tab| tab.path.is_none())
    {
        build_status(scene, state, theme, layout.status);
        return;
    }
    // The unified strip hosts the compact action cluster beside the tabs.
    build_toolbar(scene, state, theme, layout.tabs);
    build_sidebar(scene, state, theme, layout.sidebar);
    build_status(scene, state, theme, layout.status);

    if !state.settings.sidebar_collapsed {
        let splitter = Rect::new(
            layout.sidebar.right() - 2.0,
            layout.sidebar.y,
            4.0,
            layout.sidebar.height,
        );
        if splitter.contains(state.hover()) {
            scene.rect(3, splitter, scene.viewport(), theme.accent);
        } else {
            scene.rect(
                1,
                Rect::new(splitter.x + 1.0, splitter.y, 1.0, splitter.height),
                scene.viewport(),
                theme.border,
            );
        }
        scene.hit(
            splitter,
            UiAction::BeginResize(ResizeTarget::Sidebar),
            CursorHint::ResizeHorizontal,
            None,
        );
    }

    if let Some(detail) = layout.detail {
        let splitter = Rect::new(detail.x - 2.0, detail.y, 4.0, detail.height);
        scene.rect(
            3,
            splitter,
            scene.viewport(),
            if splitter.contains(state.hover()) {
                theme.accent
            } else {
                theme.border
            },
        );
        scene.hit(
            splitter,
            UiAction::BeginResize(ResizeTarget::DetailPanel),
            CursorHint::ResizeHorizontal,
            None,
        );
    }
}

fn build_tabs(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.top_bar);
    divider(
        scene,
        Rect::new(rect.x, rect.bottom() - 1.0, rect.width, 1.0),
        theme,
    );
    for (x, color) in [
        (14.0, Color::rgb(255, 95, 86)),
        (34.0, Color::rgb(255, 189, 46)),
        (54.0, Color::rgb(39, 201, 63)),
    ] {
        scene.rounded_rect(
            2,
            Rect::new(x, 16.0, 12.0, 12.0),
            rect,
            color,
            color,
            6.0,
            0.0,
        );
    }

    let tab_clip = Rect::new(90.0, 4.0, rect.right() - 314.0, 36.0);
    let mut x = 90.0;
    for (index, repo_tab) in state.tabs.iter().enumerate() {
        let tab = Rect::new(x, 4.0, 180.0, 36.0);
        let active = index == state.active_tab;
        let pill = Rect::new(tab.x + 2.0, rect.y + 5.0, tab.width - 4.0, rect.height - 11.0);
        if active {
            // Minimal underline tab: text carries the state, a white rule
            // anchors it to the strip's bottom hairline.
            let bar = Rect::new(tab.x + 10.0, rect.bottom() - 2.0, tab.width - 20.0, 2.0);
            scene.rect(2, bar, tab_clip, theme.accent);
        } else if tab.contains(state.hover()) {
            scene.rounded_rect(
                1,
                pill,
                tab_clip,
                theme.row_hover,
                theme.row_hover,
                RADIUS_MD,
                0.0,
            );
        }
        let icon = if repo_tab.path.is_some() {
            icons::REPOSITORY
        } else {
            icons::HOME
        };
        scene.text(
            format!("{icon}  {}", repo_tab.title),
            [tab.x + 12.0, tab.y + 11.0],
            Rect::new(tab.x + 12.0, tab.y, tab.width - 34.0, tab.height),
            if active { theme.text } else { theme.text_muted },
            13.0,
            18.0,
            FontFace::SansMedium,
        );
        let close = Rect::new(tab.right() - 26.0, tab.y + 5.0, 22.0, 24.0);
        if close.contains(state.hover()) {
            scene.rounded_rect(
                2,
                close,
                tab_clip,
                theme.row_hover,
                theme.row_hover,
                RADIUS_SM,
                0.0,
            );
        }
        scene.text(
            icons::CLOSE,
            [close.x + 7.0, close.y + 3.0],
            close,
            theme.text_muted,
            15.0,
            18.0,
            FontFace::Sans,
        );
        scene.hit_clipped(
            tab,
            tab_clip,
            UiAction::SelectTab(index),
            CursorHint::Pointer,
            None,
        );
        scene.hit_clipped(
            close,
            tab_clip,
            UiAction::CloseTab(index),
            CursorHint::Pointer,
            Some("Close tab"),
        );
        x += tab.width + 2.0;
    }
    let plus = Rect::new(x + 4.0, 8.0, 28.0, 28.0);
    if plus.contains(state.hover()) {
        scene.rounded_rect(1, plus, rect, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
    }
    scene.text(
        icons::ADD,
        [plus.x + 8.0, plus.y + 4.0],
        plus,
        theme.text_dim,
        18.0,
        20.0,
        FontFace::Sans,
    );
    scene.hit(plus, UiAction::NewTab, CursorHint::Pointer, Some("New tab"));

    let right = rect.right();
    let open_tabs_tooltip = format!("Open tabs ({})", icons::KEY_COMMAND_SHIFT_A);
    for (offset, icon, action, tooltip) in [
        (
            130.0,
            icons::CHEVRON_DOWN,
            UiAction::ToggleTabSwitcher,
            open_tabs_tooltip.as_str(),
        ),
        (
            94.0,
            icons::BELL,
            UiAction::ToggleNotifications,
            "Notifications",
        ),
        (58.0, icons::GEAR, UiAction::OpenPreferences, "Preferences"),
    ] {
        let target = Rect::new(right - offset, 6.0, 32.0, 32.0);
        if target.contains(state.hover()) {
            scene.rounded_rect(1, target, rect, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
        }
        scene.text(
            icon,
            [target.x + 9.0, target.y + 8.0],
            target,
            if target.contains(state.hover()) {
                theme.text
            } else {
                theme.text_muted
            },
            15.0,
            18.0,
            FontFace::Sans,
        );
        scene.hit(target, action, CursorHint::Pointer, Some(tooltip));
    }
    let profile = Rect::new(right - 180.0, 10.0, 32.0, 24.0);
    scene.rounded_rect(2, profile, rect, theme.green, theme.green, RADIUS_SM, 0.0);
    scene.text(
        "PRO",
        [profile.x + 6.0, profile.y + 4.0],
        profile,
        theme.on_accent,
        10.0,
        14.0,
        FontFace::Monospace,
    );
}

fn build_toolbar(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let repository = state
        .snapshot
        .as_ref()
        .map_or("repository", |snapshot| snapshot.name.as_str());
    let branch = state
        .snapshot
        .as_ref()
        .map_or("branch", |snapshot| snapshot.head.as_str());
    let tabs_end = 90.0 + 180.0 * state.tabs.len().to_f32().unwrap_or(0.0) + 40.0;
    let mut cursor = tabs_end + 12.0;
    // The repo/branch wells and PRs button yield to the action cluster below
    // 1150px; the sidebar and Branch action cover the same flows there.
    if rect.width >= 1150.0 {
        dropdown(
            scene,
            Rect::new(cursor, 7.0, 150.0, 30.0),
            "REPO",
            repository,
            None,
            state,
            theme,
        );
        dropdown(
            scene,
            Rect::new(cursor + 162.0, 7.0, 160.0, 30.0),
            "BRANCH",
            branch,
            Some(UiAction::ToggleBranchMenu),
            state,
            theme,
        );
        let pr = Rect::new(cursor + 334.0, 7.0, 60.0, 30.0);
        let pr_hovered = pr.contains(state.hover());
        if pr_hovered {
            scene.rounded_rect(1, pr, rect, theme.row_hover, theme.row_hover, RADIUS_MD, 0.0);
        }
        scene.text(
            format!("{}  PRs", icons::GIT_PULL_REQUEST),
            [pr.x + 11.0, pr.y + 7.0],
            pr,
            if pr_hovered {
                theme.text
            } else {
                theme.text_muted
            },
            11.0,
            16.0,
            FontFace::Sans,
        );
        cursor += 406.0;
    }

    let action_width = 34.0;
    let busy = |op| state.op_in_flight(op);
    let actions = [
        (
            icons::UNDO,
            "Undo",
            UiAction::Undo,
            state.can_undo() && !busy(ToolbarOp::Undo),
            busy(ToolbarOp::Undo),
        ),
        (
            icons::REDO,
            "Redo",
            UiAction::Redo,
            state.can_redo() && !busy(ToolbarOp::Redo),
            busy(ToolbarOp::Redo),
        ),
        (
            icons::REPOSITORY_PULL,
            "Pull",
            UiAction::Pull,
            !busy(ToolbarOp::Pull),
            busy(ToolbarOp::Pull),
        ),
        (
            icons::REPOSITORY_PUSH,
            "Push",
            UiAction::Push,
            !busy(ToolbarOp::Push),
            busy(ToolbarOp::Push),
        ),
        (
            icons::BRANCH,
            "Branch",
            UiAction::ToggleCreateBranch,
            true,
            false,
        ),
        (
            icons::ARCHIVE,
            "Stash",
            UiAction::Stash,
            !busy(ToolbarOp::Stash),
            busy(ToolbarOp::Stash),
        ),
        (
            icons::HISTORY,
            "Pop",
            UiAction::PopStash,
            !busy(ToolbarOp::Pop),
            busy(ToolbarOp::Pop),
        ),
        (
            icons::TERMINAL,
            "Terminal",
            UiAction::OpenTerminal,
            true,
            false,
        ),
    ];
    let actions_width = action_width * 8.0 + 14.0;
    let action_start = (cursor + 12.0)
        .max(rect.width * 0.42)
        .min(rect.right() - 300.0 - actions_width)
        .max(cursor + 12.0);
    for (index, (icon, label, action, enabled, busy)) in actions.into_iter().enumerate() {
        let x = action_start + index.to_f32().unwrap_or(0.0) * action_width;
        toolbar_action(
            scene,
            Rect::new(x, 7.0, action_width, 30.0),
            icon,
            label,
            action,
            enabled,
            busy,
            state,
            theme,
        );
    }
    let pull_chevron = Rect::new(action_start + 2.0 * action_width + 24.0, 7.0, 12.0, 30.0);
    scene.text(
        icons::CHEVRON_DOWN,
        [pull_chevron.x, pull_chevron.y + 9.0],
        pull_chevron,
        theme.text_muted,
        11.0,
        14.0,
        FontFace::Sans,
    );
    scene.hit(
        pull_chevron,
        UiAction::TogglePullOptions,
        CursorHint::Pointer,
        Some("Select pull operation"),
    );

    let right = rect.right();
    toolbar_action(
        scene,
        Rect::new(right - 330.0, 7.0, 46.0, 30.0),
        format!("LFS{}", icons::CHEVRON_DOWN),
        "LFS commands",
        UiAction::ToggleLfsMenu,
        true,
        false,
        state,
        theme,
    );
    toolbar_action(
        scene,
        Rect::new(right - 280.0, 7.0, 32.0, 30.0),
        icons::MENU,
        "Actions",
        UiAction::ToggleActionsMenu,
        true,
        false,
        state,
        theme,
    );
    toolbar_action(
        scene,
        Rect::new(right - 244.0, 7.0, 32.0, 30.0),
        icons::SEARCH,
        "Search",
        UiAction::ToggleSearch,
        true,
        false,
        state,
        theme,
    );
}

#[allow(clippy::too_many_arguments)]
fn dropdown(
    scene: &mut Scene,
    rect: Rect,
    caption: &str,
    value: &str,
    action: Option<UiAction>,
    state: &AppState,
    theme: &Theme,
) {
    let hovered = rect.contains(state.hover());
    scene.rounded_rect(
        1,
        rect,
        scene.viewport(),
        theme.panel,
        if hovered {
            theme.border_hard
        } else {
            theme.border_strong
        },
        RADIUS_MD,
        1.0,
    );
    // Single-line well: dim caption, value, chevron on one baseline.
    let caption_width = caption.len().to_f32().unwrap_or(0.0) * 5.2 + 6.0;
    scene.text(
        caption,
        [rect.x + 9.0, rect.y + 9.0],
        rect,
        theme.text_dim,
        9.0,
        12.0,
        FontFace::Sans,
    );
    scene.text(
        format!("{value}  {}", icons::CHEVRON_DOWN),
        [rect.x + 9.0 + caption_width, rect.y + 7.0],
        Rect::new(
            rect.x + 9.0 + caption_width,
            rect.y,
            (rect.width - caption_width - 18.0).max(0.0),
            rect.height,
        ),
        theme.text,
        12.0,
        16.0,
        FontFace::Sans,
    );
    if let Some(action) = action {
        scene.hit(rect, action, CursorHint::Pointer, None);
    }
}

#[allow(clippy::too_many_arguments)]
fn toolbar_action(
    scene: &mut Scene,
    rect: Rect,
    icon: impl Into<String>,
    label: &str,
    action: UiAction,
    enabled: bool,
    busy: bool,
    state: &AppState,
    theme: &Theme,
) {
    let hovered = enabled && rect.contains(state.hover());
    if hovered {
        scene.rounded_rect(
            1,
            rect,
            scene.viewport(),
            theme.row_hover,
            theme.row_hover,
            RADIUS_MD,
            0.0,
        );
    }
    let color = if enabled {
        if hovered {
            theme.text
        } else {
            theme.text_muted
        }
    } else {
        theme.text_disabled
    };
    if busy {
        spinner(
            scene,
            [rect.x + rect.width * 0.5, rect.y + rect.height * 0.5],
            6.5,
            state.animation_time(),
            theme.text_muted,
        );
    } else {
        scene.text(
            icon,
            [rect.x + (rect.width - 15.0) / 2.0, rect.y + 7.0],
            rect,
            color,
            16.0,
            20.0,
            FontFace::Sans,
        );
    }
    if enabled {
        let tooltip = (!label.is_empty()).then_some(label);
        scene.hit(rect, action, CursorHint::Pointer, tooltip);
    }
}

/// Rotating three-quarter arc shown in place of a toolbar icon while its Git job runs.
fn spinner(scene: &mut Scene, center: [f32; 2], radius: f32, time: f32, color: Color) {
    const SEGMENTS: usize = 16;
    let start = time * std::f32::consts::TAU * 1.1;
    let sweep = std::f32::consts::TAU * 0.75;
    let clip = scene.viewport();
    let mut previous = [
        center[0] + start.cos() * radius,
        center[1] + start.sin() * radius,
    ];
    for segment in 1..=SEGMENTS {
        let angle =
            start + sweep * segment.to_f32().unwrap_or(0.0) / SEGMENTS.to_f32().unwrap_or(1.0);
        let point = [
            center[0] + angle.cos() * radius,
            center[1] + angle.sin() * radius,
        ];
        scene.line(1, previous, point, 1.6, color, clip);
        previous = point;
    }
}

fn build_sidebar(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.panel);
    scene.rect(
        1,
        Rect::new(rect.right() - 1.0, rect.y, 1.0, rect.height),
        scene.viewport(),
        theme.border,
    );
    let clip = rect.inset(1.0);
    if state.settings.sidebar_collapsed {
        build_sidebar_rail(scene, state, theme, rect, clip);
        return;
    }
    let collapse = Rect::new(rect.x + 10.0, rect.y + 12.0, 26.0, 26.0);
    if collapse.contains(state.hover()) {
        scene.rounded_rect(1, collapse, clip, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
    }
    scene.text(
        icons::CHEVRON_LEFT,
        [collapse.x + 8.0, collapse.y + 4.0],
        collapse,
        theme.text_muted,
        13.0,
        17.0,
        FontFace::Sans,
    );
    scene.hit_clipped(
        collapse,
        clip,
        UiAction::ToggleSidebarCollapse,
        CursorHint::Pointer,
        Some("Collapse sidebar"),
    );
    let toggle = Rect::new(rect.x + 44.0, rect.y + 12.0, rect.width - 56.0, 26.0);
    scene.rounded_rect(1, toggle, clip, theme.panel_alt, theme.panel_alt, RADIUS_MD, 0.0);
    let list = Rect::new(
        toggle.x + 2.0,
        toggle.y + 2.0,
        toggle.width * 0.5 - 2.0,
        22.0,
    );
    scene.rounded_rect(2, list, clip, theme.surface_3, theme.border_strong, RADIUS_MD - 2.0, 1.0);
    scene.text(
        format!("{}  List", icons::LIST_TREE),
        [list.x + 32.0, list.y + 3.0],
        list,
        theme.text,
        11.0,
        15.0,
        FontFace::Sans,
    );
    scene.text(
        format!("{}  Agents", icons::ORGANIZATION),
        [list.right() + 32.0, list.y + 3.0],
        Rect::new(list.right(), list.y, list.width, list.height),
        theme.text_muted,
        11.0,
        15.0,
        FontFace::Sans,
    );

    let commit_count = state
        .snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.commits.len());
    scene.text(
        format!("Viewing {commit_count} commits"),
        [rect.x + 12.0, rect.y + 48.0],
        Rect::new(rect.x + 12.0, rect.y + 48.0, rect.width - 24.0, 18.0),
        theme.text_dim,
        11.0,
        15.0,
        FontFace::Sans,
    );
    let filter = sidebar_filter_rect(rect);
    scene.rounded_rect(1, filter, clip, theme.input, theme.border_strong, RADIUS_MD, 1.0);
    let filter_label = if state.branch_filter.is_empty() {
        format!("FILTER ({}+OPTION+F)", icons::KEY_COMMAND)
    } else {
        state.branch_filter.text().to_owned()
    };
    scene.text(
        filter_label,
        [filter.x + 8.0, filter.y + 6.0],
        filter.inset(4.0),
        if state.branch_filter.is_empty() {
            theme.text_dim
        } else {
            theme.text
        },
        10.0,
        15.0,
        FontFace::Monospace,
    );
    if state.focus == crate::app::state::FocusField::BranchFilter {
        crate::ui::widgets::caret_overlay(
            scene,
            2,
            [filter.x + 8.0, filter.y + 6.0],
            filter.inset(2.0),
            &state.branch_filter,
            6.0,
            15.0,
            theme,
        );
    }
    scene.text(
        icons::SEARCH,
        [filter.right() - 20.0, filter.y + 5.0],
        filter,
        theme.text_dim,
        13.0,
        17.0,
        FontFace::Sans,
    );
    scene.hit_clipped(
        filter,
        clip,
        UiAction::FocusBranchFilter,
        CursorHint::Text,
        Some("Filter branches"),
    );

    let body = sidebar_body_rect(rect);
    let Some(snapshot) = &state.snapshot else {
        scene.text(
            "Opening repo...",
            [body.x + 24.0, body.y + 15.0],
            body,
            theme.text_muted,
            12.0,
            16.0,
            FontFace::Sans,
        );
        return;
    };
    let query = state.branch_filter.to_lowercase();
    let filtered = !query.is_empty();
    let local = snapshot
        .branches
        .iter()
        .filter(|branch| !branch.remote && branch.name.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let remote = snapshot
        .branches
        .iter()
        .filter(|branch| branch.remote && branch.name.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let local_tree = build_branch_tree(
        &local
            .iter()
            .map(|branch| branch.name.clone())
            .collect::<Vec<_>>(),
    );
    let remote_tree = build_remote_tree(&remote);
    let sections =
        sidebar_section_data(state, &local, &remote, &local_tree, &remote_tree, filtered);
    let layouts = sidebar_section_layouts(state, body, &sections);

    for layout in &layouts {
        draw_sidebar_section_header(scene, state, theme, body, layout);
        if layout.collapsed || layout.content.height <= 0.0 {
            continue;
        }
        let scroll = sidebar_scroll(state, layout.target);
        let mut y = layout.content.y
            - scroll.clamp(
                0.0,
                (layout.content_height - layout.content.height).max(0.0),
            );
        match layout.index {
            0 if filtered => y = branch_rows(scene, state, theme, layout.content, y, &local, 0),
            0 => {
                y = branch_tree_rows(
                    scene,
                    state,
                    theme,
                    layout.content,
                    y,
                    &local_tree,
                    "local",
                    0,
                    &local,
                )
            }
            1 if filtered => y = branch_rows(scene, state, theme, layout.content, y, &remote, 0),
            1 => {
                for group in &remote_tree {
                    let key = format!("remote/{}", group.name);
                    y = sidebar_folder_row(
                        scene,
                        state,
                        theme,
                        layout.content,
                        y,
                        0,
                        icons::REMOTE,
                        &group.name,
                        &key,
                    );
                    if !state.collapsed_sections.contains(&key) {
                        y = branch_tree_rows(
                            scene,
                            state,
                            theme,
                            layout.content,
                            y,
                            &group.children,
                            &key,
                            1,
                            &remote,
                        );
                    }
                }
            }
            2 => {
                for worktree in &snapshot.worktrees {
                    y = sidebar_row(
                        scene,
                        state,
                        theme,
                        layout.content,
                        y,
                        0,
                        icons::WORKSPACE,
                        &worktree.name,
                        None,
                        false,
                    );
                }
            }
            3 => {
                for stash in &snapshot.stashes {
                    y = sidebar_row(
                        scene,
                        state,
                        theme,
                        layout.content,
                        y,
                        0,
                        icons::ARCHIVE,
                        &stash.name,
                        Some(UiAction::OpenStashContext(stash.index)),
                        false,
                    );
                }
            }
            8 => {
                for tag in &snapshot.tags {
                    y = sidebar_row(
                        scene,
                        state,
                        theme,
                        layout.content,
                        y,
                        0,
                        icons::TAG,
                        &tag.name,
                        Some(UiAction::TagClick(tag.name.clone())),
                        false,
                    );
                }
            }
            _ => {}
        }
        let _ = y;
        if let Some(target) = layout.target {
            scrollbar(
                scene,
                layout.content,
                layout.content_height,
                scroll,
                target,
                theme,
            );
        }
        if layout.index < 4 && layout.content_height > 0.0 {
            scene.hit_clipped(
                Rect::new(
                    layout.content.x,
                    layout.content.bottom() - 2.0,
                    layout.content.width,
                    4.0,
                ),
                body,
                UiAction::BeginResize(ResizeTarget::SidebarSection(layout.index as u8)),
                CursorHint::ResizeVertical,
                None,
            );
        }
    }
}

const SIDEBAR_HEADER_HEIGHT: f32 = 28.0;
const SIDEBAR_ROW_HEIGHT: f32 = 24.0;
pub(crate) const SIDEBAR_RAIL_WIDTH: f32 = 44.0;

/// One entry of the collapsed icon rail: NF glyph, section title, count.
fn sidebar_rail_entries(state: &AppState) -> [(&'static str, &'static str, usize); 10] {
    let snapshot = state.snapshot.as_ref();
    let local = snapshot.map_or(0, |snapshot| {
        snapshot
            .branches
            .iter()
            .filter(|branch| !branch.remote)
            .count()
    });
    let remote = snapshot.map_or(0, |snapshot| {
        snapshot
            .branches
            .iter()
            .filter(|branch| branch.remote)
            .count()
    });
    [
        (icons::BRANCH, "LOCAL", local),
        (icons::REMOTE, "REMOTE", remote),
        (
            icons::WORKSPACE,
            "WORKTREES",
            snapshot.map_or(0, |snapshot| snapshot.worktrees.len()),
        ),
        (
            icons::ARCHIVE,
            "STASHES",
            snapshot.map_or(0, |snapshot| snapshot.stashes.len()),
        ),
        (icons::CLOUD, "CLOUD PATCHES", 0),
        (icons::GIT_PULL_REQUEST, "PULL REQUESTS", 0),
        (icons::ISSUES, "GITHUB ISSUES", 0),
        (icons::ORGANIZATION, "TEAMS", 0),
        (
            icons::TAG,
            "TAGS",
            snapshot.map_or(0, |snapshot| snapshot.tags.len()),
        ),
        (icons::SUBMODULE, "SUBMODULES", 0),
    ]
}

/// GitKraken-style collapsed sidebar: a narrow rail of section icons with
/// count badges. Clicking an icon expands the sidebar onto that section.
fn build_sidebar_rail(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect, clip: Rect) {
    let toggle = Rect::new(rect.x + 9.0, rect.y + 12.0, 26.0, 26.0);
    if toggle.contains(state.hover()) {
        scene.rounded_rect(1, toggle, clip, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
    }
    scene.text(
        icons::CHEVRON_RIGHT,
        [toggle.x + 8.0, toggle.y + 4.0],
        toggle,
        theme.text_muted,
        13.0,
        17.0,
        FontFace::Sans,
    );
    scene.hit_clipped(
        toggle,
        clip,
        UiAction::ToggleSidebarCollapse,
        CursorHint::Pointer,
        Some("Expand sidebar"),
    );
    let mut y = toggle.bottom() + 12.0;
    for (icon, title, count) in sidebar_rail_entries(state) {
        let cell = Rect::new(rect.x + 6.0, y, rect.width - 12.0, 32.0);
        if cell.bottom() > clip.bottom() {
            break;
        }
        if cell.contains(state.hover()) {
            scene.rounded_rect(1, cell, clip, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
        }
        scene.text(
            icon,
            [cell.x + 8.0, cell.y + 7.0],
            cell,
            theme.text_dim,
            14.0,
            18.0,
            FontFace::Terminal,
        );
        if count > 0 {
            scene.text(
                count.to_string(),
                [cell.right() - 11.0, cell.y + 16.0],
                cell,
                theme.text_muted,
                8.0,
                11.0,
                FontFace::Monospace,
            );
        }
        scene.hit_clipped(
            cell,
            clip,
            UiAction::ExpandSidebarSection(title.to_owned()),
            CursorHint::Pointer,
            Some(title),
        );
        y = cell.bottom() + 4.0;
    }
}
const SIDEBAR_MIN_ROWS_HEIGHT: f32 = SIDEBAR_ROW_HEIGHT * 3.0;

#[derive(Clone, Copy)]
struct SidebarSectionData {
    title: &'static str,
    count: usize,
    content_height: f32,
    target: Option<ScrollTarget>,
}

#[derive(Clone, Copy)]
struct SidebarSectionLayout {
    index: usize,
    title: &'static str,
    count: usize,
    header: Rect,
    content: Rect,
    content_height: f32,
    target: Option<ScrollTarget>,
    collapsed: bool,
}

struct RemoteTree {
    name: String,
    children: Vec<BranchTreeNode>,
}

#[derive(Debug, PartialEq, Eq)]
enum BranchTreeNode {
    Folder {
        name: String,
        children: Vec<BranchTreeNode>,
    },
    Leaf {
        name: String,
        branch_name: String,
    },
}

fn sidebar_filter_rect(rect: Rect) -> Rect {
    Rect::new(rect.x + 12.0, rect.y + 68.0, rect.width - 24.0, 28.0)
}

fn sidebar_body_rect(rect: Rect) -> Rect {
    let clip = rect.inset(1.0);
    let filter = sidebar_filter_rect(rect);
    Rect::new(
        clip.x,
        filter.bottom() + 10.0,
        clip.width,
        (clip.bottom() - filter.bottom() - 11.0).max(0.0),
    )
}

fn build_branch_tree(names: &[String]) -> Vec<BranchTreeNode> {
    let pairs = names
        .iter()
        .map(|name| (name.as_str(), name.as_str()))
        .collect::<Vec<_>>();
    build_branch_tree_pairs(&pairs)
}

fn build_branch_tree_pairs(names: &[(&str, &str)]) -> Vec<BranchTreeNode> {
    let mut roots = Vec::new();
    for (display_name, branch_name) in names {
        let parts = display_name
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            continue;
        }
        insert_branch_tree_node(&mut roots, &parts, branch_name);
    }
    roots
}

fn insert_branch_tree_node(nodes: &mut Vec<BranchTreeNode>, parts: &[&str], branch_name: &str) {
    if parts.len() == 1 {
        nodes.push(BranchTreeNode::Leaf {
            name: parts[0].to_owned(),
            branch_name: branch_name.to_owned(),
        });
        return;
    }
    let index = nodes
        .iter()
        .position(|node| matches!(node, BranchTreeNode::Folder { name, .. } if name == parts[0]));
    let folder = if let Some(index) = index {
        &mut nodes[index]
    } else {
        nodes.push(BranchTreeNode::Folder {
            name: parts[0].to_owned(),
            children: Vec::new(),
        });
        nodes.last_mut().expect("just inserted folder")
    };
    let BranchTreeNode::Folder { children, .. } = folder else {
        unreachable!("folder lookup only returns folders");
    };
    insert_branch_tree_node(children, &parts[1..], branch_name);
}

fn build_remote_tree(branches: &[&BranchInfo]) -> Vec<RemoteTree> {
    let mut groups = BTreeMap::<String, Vec<(&str, &str)>>::new();
    for branch in branches {
        let (remote, suffix) = branch
            .name
            .split_once('/')
            .unwrap_or(("remote", &branch.name));
        groups
            .entry(remote.to_owned())
            .or_default()
            .push((suffix, &branch.name));
    }
    groups
        .into_iter()
        .map(|(name, pairs)| RemoteTree {
            name,
            children: build_branch_tree_pairs(&pairs),
        })
        .collect()
}

fn tree_visible_rows(nodes: &[BranchTreeNode], key_prefix: &str, state: &AppState) -> usize {
    nodes
        .iter()
        .map(|node| match node {
            BranchTreeNode::Leaf { .. } => 1,
            BranchTreeNode::Folder { name, children } => {
                let key = format!("{key_prefix}/{name}");
                1 + (!state.collapsed_sections.contains(&key))
                    .then(|| tree_visible_rows(children, &key, state))
                    .unwrap_or(0)
            }
        })
        .sum()
}

fn remote_tree_visible_rows(groups: &[RemoteTree], state: &AppState) -> usize {
    groups
        .iter()
        .map(|group| {
            let key = format!("remote/{}", group.name);
            1 + (!state.collapsed_sections.contains(&key))
                .then(|| tree_visible_rows(&group.children, &key, state))
                .unwrap_or(0)
        })
        .sum()
}

fn sidebar_section_data(
    state: &AppState,
    local: &[&BranchInfo],
    remote: &[&BranchInfo],
    local_tree: &[BranchTreeNode],
    remote_tree: &[RemoteTree],
    filtered: bool,
) -> Vec<SidebarSectionData> {
    let rows = |count: usize| count.to_f32().unwrap_or(0.0) * SIDEBAR_ROW_HEIGHT;
    vec![
        SidebarSectionData {
            title: "LOCAL",
            count: local.len(),
            content_height: rows(if filtered {
                local.len()
            } else {
                tree_visible_rows(local_tree, "local", state)
            }),
            target: Some(ScrollTarget::SidebarLocal),
        },
        SidebarSectionData {
            title: "REMOTE",
            count: remote.len(),
            content_height: rows(if filtered {
                remote.len()
            } else {
                remote_tree_visible_rows(remote_tree, state)
            }),
            target: Some(ScrollTarget::SidebarRemote),
        },
        SidebarSectionData {
            title: "WORKTREES",
            count: state
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.worktrees.len()),
            content_height: rows(
                state
                    .snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.worktrees.len()),
            ),
            target: Some(ScrollTarget::SidebarWorktrees),
        },
        SidebarSectionData {
            title: "STASHES",
            count: state
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.stashes.len()),
            content_height: rows(
                state
                    .snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.stashes.len()),
            ),
            target: Some(ScrollTarget::SidebarStashes),
        },
        SidebarSectionData {
            title: "CLOUD PATCHES",
            count: 0,
            content_height: 0.0,
            target: None,
        },
        SidebarSectionData {
            title: "PULL REQUESTS",
            count: 0,
            content_height: 0.0,
            target: None,
        },
        SidebarSectionData {
            title: "GITHUB ISSUES",
            count: 0,
            content_height: 0.0,
            target: None,
        },
        SidebarSectionData {
            title: "TEAMS",
            count: 0,
            content_height: 0.0,
            target: None,
        },
        SidebarSectionData {
            title: "TAGS",
            count: state
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.tags.len()),
            content_height: rows(
                state
                    .snapshot
                    .as_ref()
                    .map_or(0, |snapshot| snapshot.tags.len()),
            ),
            target: Some(ScrollTarget::SidebarTags),
        },
        SidebarSectionData {
            title: "SUBMODULES",
            count: 0,
            content_height: 0.0,
            target: None,
        },
    ]
}

fn sidebar_section_layouts(
    state: &AppState,
    body: Rect,
    sections: &[SidebarSectionData],
) -> Vec<SidebarSectionLayout> {
    let available =
        (body.height - sections.len().to_f32().unwrap_or(0.0) * SIDEBAR_HEADER_HEIGHT).max(0.0);
    let mut heights = vec![0.0; sections.len()];
    for (index, section) in sections.iter().enumerate() {
        if state.collapsed_sections.contains(section.title) || section.content_height <= 0.0 {
            continue;
        }
        heights[index] = section.content_height.min(SIDEBAR_MIN_ROWS_HEIGHT);
    }
    let base_total = heights.iter().sum::<f32>();
    let mut remaining = if base_total > available {
        let scale = available / base_total.max(1.0);
        for height in &mut heights {
            *height *= scale;
        }
        0.0
    } else {
        available - base_total
    };
    while remaining > 0.5 {
        let eligible = sections
            .iter()
            .enumerate()
            .filter(|(index, section)| {
                !state.collapsed_sections.contains(section.title)
                    && section.content_height > heights[*index] + 0.5
            })
            .collect::<Vec<_>>();
        if eligible.is_empty() {
            break;
        }
        let weight = eligible
            .iter()
            .map(|(index, _)| {
                state
                    .sidebar_section_fractions
                    .get(*index)
                    .copied()
                    .unwrap_or(1.0)
            })
            .sum::<f32>()
            .max(1.0);
        let mut used = 0.0;
        for (index, section) in eligible {
            let share = remaining
                * state
                    .sidebar_section_fractions
                    .get(index)
                    .copied()
                    .unwrap_or(1.0)
                / weight;
            let addition = share.min(section.content_height - heights[index]);
            heights[index] += addition;
            used += addition;
        }
        if used <= 0.5 {
            break;
        }
        remaining -= used;
    }
    let mut y = body.y;
    sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let header = Rect::new(body.x, y, body.width, SIDEBAR_HEADER_HEIGHT);
            y = header.bottom();
            let collapsed = state.collapsed_sections.contains(section.title);
            let content = Rect::new(
                body.x,
                y,
                body.width,
                if collapsed { 0.0 } else { heights[index] },
            );
            y = content.bottom();
            SidebarSectionLayout {
                index,
                title: section.title,
                count: section.count,
                header,
                content,
                content_height: section.content_height,
                target: section.target,
                collapsed,
            }
        })
        .collect()
}

fn draw_sidebar_section_header(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    section: &SidebarSectionLayout,
) {
    if section.header.y >= clip.bottom() {
        return;
    }
    if section.header.contains(state.hover()) {
        let wash = Rect::new(
            section.header.x + 4.0,
            section.header.y + 2.0,
            section.header.width - 8.0,
            section.header.height - 4.0,
        );
        scene.rounded_rect(1, wash, clip, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
    }
    scene.text(
        if section.collapsed {
            icons::CHEVRON_RIGHT
        } else {
            icons::CHEVRON_DOWN
        },
        [section.header.x + 10.0, section.header.y + 6.0],
        section.header,
        theme.text_dim,
        10.0,
        15.0,
        FontFace::Terminal,
    );
    scene.text(
        section.title,
        [section.header.x + 24.0, section.header.y + 7.0],
        Rect::new(
            section.header.x + 20.0,
            section.header.y,
            section.header.width - 58.0,
            section.header.height,
        ),
        theme.text_dim,
        11.0,
        15.0,
        FontFace::SansMedium,
    );
    let count_x = section.header.right() - 42.0;
    scene.text(
        section.count.to_string(),
        [count_x + 8.0, section.header.y + 7.0],
        Rect::new(count_x, section.header.y, 34.0, 18.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    scene.hit_clipped(
        section.header,
        clip,
        UiAction::ToggleSection(section.title.to_owned()),
        CursorHint::Pointer,
        None,
    );
    // GitKraken-style hover affordance: a "+" on the REMOTE header opens the
    // Add Remote form. Registered after the header hit so it wins the click.
    if section.title == "REMOTE" && section.header.contains(state.hover()) {
        let button = Rect::new(
            section.header.right() - 58.0,
            section.header.y + 6.0,
            16.0,
            16.0,
        );
        let hovered = button.contains(state.hover());
        scene.rounded_rect(
            1,
            button,
            clip,
            if hovered { theme.row_hover } else { theme.panel_alt },
            theme.border_strong,
            RADIUS_SM,
            1.0,
        );
        scene.text(
            icons::ADD,
            [button.x + 3.0, button.y + 1.0],
            button,
            if hovered { theme.text } else { theme.text_dim },
            10.0,
            14.0,
            FontFace::Terminal,
        );
        scene.hit_clipped(
            button,
            clip,
            UiAction::OpenAddRemote,
            CursorHint::Pointer,
            Some("Add remote"),
        );
    }
}

fn branch_tree_rows(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    mut y: f32,
    nodes: &[BranchTreeNode],
    key_prefix: &str,
    depth: usize,
    branches: &[&BranchInfo],
) -> f32 {
    for node in nodes {
        match node {
            BranchTreeNode::Folder { name, children } => {
                let key = format!("{key_prefix}/{name}");
                y = sidebar_folder_row(
                    scene,
                    state,
                    theme,
                    clip,
                    y,
                    depth,
                    icons::FOLDER,
                    name,
                    &key,
                );
                if !state.collapsed_sections.contains(&key) {
                    y = branch_tree_rows(
                        scene,
                        state,
                        theme,
                        clip,
                        y,
                        children,
                        &key,
                        depth + 1,
                        branches,
                    );
                }
            }
            BranchTreeNode::Leaf { name, branch_name } => {
                if let Some(branch) = branches.iter().find(|branch| branch.name == *branch_name) {
                    let icon = if branch.current {
                        icons::CHECK
                    } else {
                        icons::BRANCH
                    };
                    y = sidebar_row(
                        scene,
                        state,
                        theme,
                        clip,
                        y,
                        depth,
                        icon,
                        name,
                        Some(UiAction::BranchClick(branch.name.clone())),
                        branch.current,
                    );
                }
            }
        }
    }
    y
}

fn branch_rows(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    mut y: f32,
    branches: &[&BranchInfo],
    depth: usize,
) -> f32 {
    for branch in branches {
        let icon = if branch.current {
            icons::CHECK
        } else if branch.remote {
            icons::REMOTE
        } else {
            icons::BRANCH
        };
        y = sidebar_row(
            scene,
            state,
            theme,
            clip,
            y,
            depth,
            icon,
            &branch.name,
            Some(UiAction::BranchClick(branch.name.clone())),
            branch.current,
        );
    }
    y
}

#[allow(clippy::too_many_arguments)]
fn sidebar_folder_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    y: f32,
    depth: usize,
    icon: &str,
    label: &str,
    key: &str,
) -> f32 {
    let row = Rect::new(clip.x + 6.0, y, clip.width - 12.0, SIDEBAR_ROW_HEIGHT);
    if let Some(visible_row) = row.intersection(clip) {
        if visible_row.contains(state.hover()) {
            scene.rounded_rect(
                1,
                visible_row,
                clip,
                theme.row_hover,
                theme.row_hover,
                RADIUS_SM,
                0.0,
            );
        }
        let x = row.x + 8.0 + depth.to_f32().unwrap_or(0.0) * 12.0;
        scene.text(
            if state.collapsed_sections.contains(key) {
                icons::CHEVRON_RIGHT
            } else {
                icons::CHEVRON_DOWN
            },
            [x, row.y + 4.0],
            visible_row,
            theme.text_dim,
            11.0,
            16.0,
            FontFace::Terminal,
        );
        scene.text(
            icon,
            [x + 13.0, row.y + 4.0],
            visible_row,
            theme.text_dim,
            12.0,
            16.0,
            FontFace::Terminal,
        );
        truncated_text(
            scene,
            label,
            [x + 32.0, row.y + 4.0],
            Rect::new(x + 32.0, row.y, row.right() - x - 32.0, row.height),
            clip,
            theme.text,
            13.0,
            16.0,
            FontFace::Sans,
        );
    }
    scene.hit_clipped(
        row,
        clip,
        UiAction::ToggleSection(key.to_owned()),
        CursorHint::Pointer,
        None,
    );
    y + SIDEBAR_ROW_HEIGHT
}

#[allow(clippy::too_many_arguments)]
fn sidebar_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    y: f32,
    depth: usize,
    icon: &str,
    label: &str,
    action: Option<UiAction>,
    selected: bool,
) -> f32 {
    let row = Rect::new(clip.x + 6.0, y, clip.width - 12.0, SIDEBAR_ROW_HEIGHT);
    let droppable = state.dragging_ref().is_some_and(|drag| {
        matches!(
            &action,
            Some(UiAction::BranchClick(name) | UiAction::TagClick(name)) if *name != drag.source
        )
    });
    if let Some(visible_row) = row.intersection(clip) {
        if droppable {
            let hovered = visible_row.contains(state.hover());
            scene.rounded_rect(
                1,
                visible_row,
                clip,
                theme.yellow_muted,
                if hovered {
                    theme.yellow
                } else {
                    theme.yellow_muted
                },
                RADIUS_SM,
                if hovered { 1.4 } else { 0.0 },
            );
        } else if selected {
            scene.rounded_rect(
                1,
                visible_row,
                clip,
                theme.accent_soft,
                theme.accent_soft,
                RADIUS_SM,
                0.0,
            );
        } else if visible_row.contains(state.hover()) {
            scene.rounded_rect(
                1,
                visible_row,
                clip,
                theme.row_hover,
                theme.row_hover,
                RADIUS_SM,
                0.0,
            );
        }
        let x = row.x + 10.0 + depth.to_f32().unwrap_or(0.0) * 12.0;
        let text_color = if selected { theme.accent } else { theme.text };
        let icon_color = if selected {
            theme.accent
        } else {
            theme.text_dim
        };
        scene.text(
            icon,
            [x, row.y + 4.0],
            visible_row,
            icon_color,
            13.0,
            16.0,
            FontFace::Sans,
        );
        truncated_text(
            scene,
            label,
            [x + 20.0, row.y + 4.0],
            Rect::new(x + 20.0, row.y, row.right() - x - 28.0, row.height),
            clip,
            text_color,
            13.0,
            16.0,
            FontFace::Sans,
        );
    }
    if let Some(action) = action {
        scene.hit_clipped(row, clip, action, CursorHint::Pointer, None);
    }
    y + SIDEBAR_ROW_HEIGHT
}

pub(crate) fn sidebar_scrollbar_metrics(
    state: &AppState,
    target: ScrollTarget,
) -> Option<(Rect, f32)> {
    if state.settings.sidebar_collapsed {
        return None;
    }
    let snapshot = state.snapshot.as_ref()?;
    let query = state.branch_filter.to_lowercase();
    let filtered = !query.is_empty();
    let local = snapshot
        .branches
        .iter()
        .filter(|branch| !branch.remote && branch.name.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let remote = snapshot
        .branches
        .iter()
        .filter(|branch| branch.remote && branch.name.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let local_tree = build_branch_tree(
        &local
            .iter()
            .map(|branch| branch.name.clone())
            .collect::<Vec<_>>(),
    );
    let remote_tree = build_remote_tree(&remote);
    let layout = crate::views::Layout::for_state(state);
    let sections =
        sidebar_section_data(state, &local, &remote, &local_tree, &remote_tree, filtered);
    sidebar_section_layouts(state, sidebar_body_rect(layout.sidebar), &sections)
        .into_iter()
        .find(|section| section.target == Some(target))
        .map(|section| (section.content, section.content_height))
}

pub(crate) fn sidebar_scroll_target_at(state: &AppState, point: [f32; 2]) -> Option<ScrollTarget> {
    if state.settings.sidebar_collapsed {
        return None;
    }
    let snapshot = state.snapshot.as_ref()?;
    let query = state.branch_filter.to_lowercase();
    let filtered = !query.is_empty();
    let local = snapshot
        .branches
        .iter()
        .filter(|branch| !branch.remote && branch.name.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let remote = snapshot
        .branches
        .iter()
        .filter(|branch| branch.remote && branch.name.to_lowercase().contains(&query))
        .collect::<Vec<_>>();
    let local_tree = build_branch_tree(
        &local
            .iter()
            .map(|branch| branch.name.clone())
            .collect::<Vec<_>>(),
    );
    let remote_tree = build_remote_tree(&remote);
    let layout = crate::views::Layout::for_state(state);
    let sections =
        sidebar_section_data(state, &local, &remote, &local_tree, &remote_tree, filtered);
    sidebar_section_layouts(state, sidebar_body_rect(layout.sidebar), &sections)
        .into_iter()
        .find(|section| section.content.contains(point))
        .and_then(|section| section.target)
}

fn sidebar_scroll(state: &AppState, target: Option<ScrollTarget>) -> f32 {
    match target {
        Some(ScrollTarget::SidebarLocal) => state.sidebar_local_scroll,
        Some(ScrollTarget::SidebarRemote) => state.sidebar_remote_scroll,
        Some(ScrollTarget::SidebarWorktrees) => state.sidebar_worktrees_scroll,
        Some(ScrollTarget::SidebarStashes) => state.sidebar_stashes_scroll,
        Some(ScrollTarget::SidebarTags) => state.sidebar_tags_scroll,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::{BranchTreeNode, build_branch_tree};

    #[test]
    fn branch_tree_groups_shared_prefixes_and_nested_folders() {
        let tree = build_branch_tree(&[
            "feat/macros".to_owned(),
            "feat/remote".to_owned(),
            "farm/2fc0230c/ci-node24-actions".to_owned(),
        ]);
        assert_eq!(
            tree,
            vec![
                BranchTreeNode::Folder {
                    name: "feat".to_owned(),
                    children: vec![
                        BranchTreeNode::Leaf {
                            name: "macros".to_owned(),
                            branch_name: "feat/macros".to_owned(),
                        },
                        BranchTreeNode::Leaf {
                            name: "remote".to_owned(),
                            branch_name: "feat/remote".to_owned(),
                        },
                    ],
                },
                BranchTreeNode::Folder {
                    name: "farm".to_owned(),
                    children: vec![BranchTreeNode::Folder {
                        name: "2fc0230c".to_owned(),
                        children: vec![BranchTreeNode::Leaf {
                            name: "ci-node24-actions".to_owned(),
                            branch_name: "farm/2fc0230c/ci-node24-actions".to_owned(),
                        }],
                    }],
                },
            ]
        );
        assert_eq!(tree_row_count(&tree), 6);
    }

    fn tree_row_count(nodes: &[BranchTreeNode]) -> usize {
        nodes
            .iter()
            .map(|node| match node {
                BranchTreeNode::Leaf { .. } => 1,
                BranchTreeNode::Folder { children, .. } => 1 + tree_row_count(children),
            })
            .sum()
    }
}

fn build_status(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.window);
    divider(scene, Rect::new(rect.x, rect.y, rect.width, 1.0), theme);
    let left = if state.busy_jobs > 0 {
        format!(
            "{}  {} Git operation(s) running",
            icons::LOADING,
            state.busy_jobs
        )
    } else if let Some(snapshot) = &state.snapshot {
        format!(
            "{} {}  •  {} commits loaded",
            icons::BRANCH,
            snapshot.head,
            snapshot.commits.len()
        )
    } else {
        format!("{} Opening repository", icons::LOADING)
    };
    scene.text(
        left,
        [rect.x + 12.0, rect.y + 4.0],
        Rect::new(rect.x + 8.0, rect.y, rect.width * 0.6, rect.height),
        if state.busy_jobs > 0 {
            theme.accent
        } else {
            theme.text_muted
        },
        11.0,
        15.0,
        FontFace::Sans,
    );
    scene.text(
        format!(
            "{}   {}   {}      Support      0.1.0",
            icons::LAYOUT,
            icons::SPLIT_VERTICAL,
            icons::ACCOUNT
        ),
        [rect.right() - 240.0, rect.y + 4.0],
        Rect::new(rect.right() - 250.0, rect.y, 240.0, rect.height),
        theme.text_muted,
        11.0,
        15.0,
        FontFace::Sans,
    );
}
