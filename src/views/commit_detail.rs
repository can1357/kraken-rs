use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

use chrono::{DateTime, Local, Utc};
use num_traits::ToPrimitive;

use crate::{
    app::state::AppState,
    git::models::{ChangeKind, CommitDetail, DiffScope, FileChange},
    graph::avatars,
    ui::{
        FontFace, RADIUS_MD, RADIUS_SM, Rect, Scene, Theme,
        action::{CursorHint, ResizeTarget, ScrollTarget, UiAction},
        icons,
        widgets::{button, checkbox, divider, scrollbar, truncated_text},
    },
};

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.panel);
    divider(scene, Rect::new(rect.x, rect.y, 1.0, rect.height), theme);
    if state.selected_commits.len() > 1 {
        build_multi(scene, state, theme, rect);
        return;
    }

    let Some(detail) = &state.detail else {
        let header = Rect::new(rect.x, rect.y, rect.width, 39.0);
        scene.rect(1, header, rect, theme.panel);
        scene.text(
            format!("{}  Loading commit details…", icons::LOADING),
            [rect.x + 32.0, rect.y + 82.0],
            Rect::new(rect.x + 20.0, rect.y + 70.0, rect.width - 40.0, 32.0),
            theme.accent,
            13.0,
            18.0,
            FontFace::Sans,
        );
        return;
    };

    // Pre-calculate heights for the dark header background block
    let mut body_height = 0.0;
    if !detail.body.is_empty() {
        let lines = detail.body.lines().count().clamp(1, 10);
        body_height = lines.to_f32().unwrap_or(1.0) * 18.0 + 8.0 + 10.0;
    }
    let mut conflicts_height = 0.0;
    if !detail.conflicts.is_empty() {
        conflicts_height = 22.0
            + detail
                .conflicts
                .iter()
                .take(5)
                .count()
                .to_f32()
                .unwrap_or(0.0)
                * 18.0
            + 10.0;
    }

    let header_height = 39.0;
    let auto_message_height = header_height + 18.0 + 55.0 + body_height + conflicts_height + 15.0;
    let message_bg_height = if state.detail_message_height > 0.0 {
        state
            .detail_message_height
            .clamp(110.0, (rect.height * 0.7).max(110.0))
    } else {
        auto_message_height
    };

    // Sunken dark background separating header + message from the file list.
    scene.rect(
        1,
        Rect::new(rect.x, rect.y, rect.width, message_bg_height),
        rect,
        theme.panel_alt,
    );

    // Header top section
    let header = Rect::new(rect.x, rect.y, rect.width, 39.0);
    let commit_id = detail.short_id.as_str();
    let commit_label = "COMMIT:";
    let label_width = (commit_label.len() + commit_id.len())
        .to_f32()
        .unwrap_or(14.0)
        * 6.5;

    let is_local = state.commit_is_local(&detail.id);
    let ai_label = if is_local {
        format!("{} Recompose with AI", icons::SPARKLE)
    } else {
        format!("{} Explain commit", icons::SPARKLE)
    };
    let ai_width = if is_local { 145.0 } else { 125.0 };
    // The close button is small and on the right.
    let close = Rect::new(header.right() - 25.0, header.y + 6.0, 19.0, 26.0);
    let ai = Rect::new(close.x - ai_width - 8.0, header.y + 6.0, ai_width, 26.0);
    // Keep the commit ID strictly left of the action button. Text is clipped by this rect
    // when the detail panel is too narrow to show the complete label.
    let label_rect = Rect::new(
        header.x + 12.0,
        header.y,
        (ai.x - 8.0 - (header.x + 12.0)).max(0.0),
        header.height,
    );

    let label_x = label_rect.x + (label_rect.width - label_width) / 2.0;
    scene.text(
        commit_label,
        [label_x, header.y + 12.0],
        label_rect,
        theme.text_dim,
        11.0,
        15.0,
        FontFace::Sans,
    );
    scene.text(
        commit_id,
        [label_x + 52.0, header.y + 12.0],
        label_rect,
        theme.accent,
        11.0,
        15.0,
        FontFace::Monospace,
    );
    scene.hit(
        Rect::new(
            label_x + 52.0,
            header.y,
            commit_id.len().to_f32().unwrap_or(0.0) * 6.5,
            header.height,
        ),
        UiAction::JumpToCommit(detail.id.clone()),
        CursorHint::Pointer,
        None,
    );

    button(
        scene,
        ai,
        ai_label,
        UiAction::ShowAiStatus,
        state.hover(),
        theme,
        false,
        true,
        Some("Use the configured AI provider"),
    );

    scene.text(
        icons::CLOSE,
        [close.x + 5.0, close.y + 3.0],
        close,
        theme.text_muted,
        15.0,
        19.0,
        FontFace::Sans,
    );
    scene.hit(
        close,
        UiAction::CloseDetail,
        CursorHint::Pointer,
        Some("Close details"),
    );

    // Divider below header
    divider(
        scene,
        Rect::new(rect.x, header.bottom(), rect.width, 1.0),
        theme,
    );

    let clip = Rect::new(
        rect.x + 1.0,
        header.bottom(),
        rect.width - 2.0,
        rect.height - header.height,
    );
    let files = collect_rows(state, detail);
    let tree_rows = state.path_tree.then(|| build_tree_rows(&files));
    let total_files = tree_rows.as_ref().map_or(files.len(), Vec::len);
    let content_height = 430.0 + total_files.to_f32().unwrap_or(0.0) * 24.0;
    let scroll = state
        .detail_scroll
        .min((content_height - clip.height).max(0.0));
    let message_clip = clip
        .intersection(Rect::new(
            rect.x,
            rect.y,
            rect.width,
            (message_bg_height - 16.0).max(0.0),
        ))
        .unwrap_or(clip);
    let mut y = header.bottom() + 15.0;

    truncated_text(
        scene,
        &detail.subject,
        [rect.x + 16.0, y],
        Rect::new(rect.x + 16.0, y, rect.width - 32.0, 54.0),
        message_clip,
        theme.text,
        15.0,
        20.0,
        FontFace::SansMedium,
    );
    y += 55.0;

    if !detail.body.is_empty() {
        let lines = detail.body.lines().count().clamp(1, 10);
        let draw_height = lines.to_f32().unwrap_or(1.0) * 18.0 + 8.0;
        scene.text(
            &detail.body,
            [rect.x + 16.0, y],
            Rect::new(rect.x + 16.0, y, rect.width - 32.0, draw_height)
                .intersection(message_clip)
                .unwrap_or(message_clip),
            theme.text_muted,
            13.0,
            18.0,
            FontFace::Sans,
        );
        y += draw_height + 10.0;
    }

    if !detail.conflicts.is_empty() {
        scene.text(
            "Conflicts:",
            [rect.x + 16.0, y],
            Rect::new(rect.x + 16.0, y, rect.width - 32.0, 22.0)
                .intersection(message_clip)
                .unwrap_or(message_clip),
            theme.text_muted,
            12.0,
            16.0,
            FontFace::Sans,
        );
        y += 22.0;
        for conflict in detail.conflicts.iter().take(5) {
            scene.text(
                conflict.display().to_string(),
                [rect.x + 24.0, y],
                Rect::new(rect.x + 24.0, y, rect.width - 40.0, 19.0)
                    .intersection(message_clip)
                    .unwrap_or(message_clip),
                theme.text_dim,
                11.0,
                15.0,
                FontFace::Monospace,
            );
            y += 18.0;
        }
    }

    // Resizable bottom edge of the dark message block.
    let handle_y = rect.y + message_bg_height - 16.0;
    let handle = Rect::new(rect.x, handle_y, rect.width, 14.0);
    let handle_hovered =
        handle.contains(state.hover()) || state.drag == Some(ResizeTarget::DetailMessage);
    scene.rect(
        2,
        Rect::new(
            rect.x + (rect.width - 60.0) / 2.0,
            handle_y + 6.0,
            60.0,
            2.0,
        ),
        clip,
        if handle_hovered {
            theme.accent
        } else {
            theme.border
        },
    );
    divider(
        scene,
        Rect::new(rect.x, rect.y + message_bg_height - 1.0, rect.width, 1.0),
        theme,
    );
    scene.hit(
        handle,
        UiAction::BeginResize(ResizeTarget::DetailMessage),
        CursorHint::ResizeVertical,
        None,
    );

    let mut y = rect.y + message_bg_height + 20.0 - scroll;

    let avatar = Rect::new(rect.x + 16.0, y, 28.0, 28.0);
    scene.image(2, avatar, clip, avatars::request(&detail.email));
    scene.rounded_rect(
        2,
        avatar,
        clip,
        theme.window.with_alpha(0.0),
        theme.border_hard.with_alpha(0.5),
        14.0,
        1.0,
    );
    scene.text(
        &detail.author,
        [avatar.right() + 12.0, y - 2.0],
        Rect::new(avatar.right() + 12.0, y - 2.0, rect.width - 200.0, 20.0)
            .intersection(clip)
            .unwrap_or(clip),
        theme.text,
        13.0,
        17.0,
        FontFace::Sans,
    );
    scene.text(
        &detail.email,
        [avatar.right() + 12.0, y + 14.0],
        Rect::new(avatar.right() + 12.0, y + 14.0, rect.width - 200.0, 18.0)
            .intersection(clip)
            .unwrap_or(clip),
        theme.text_dim,
        11.0,
        14.0,
        FontFace::Monospace,
    );
    scene.text(
        format!("AUTHORED {}", format_time(detail.authored_seconds)).to_uppercase(),
        [avatar.right() + 12.0, y + 28.0],
        Rect::new(avatar.right() + 12.0, y + 28.0, rect.width - 200.0, 18.0)
            .intersection(clip)
            .unwrap_or(clip),
        theme.text_dim,
        11.0,
        14.0,
        FontFace::Sans,
    );

    scene.text(
        "PARENT:",
        [rect.right() - 150.0, y + 2.0],
        Rect::new(rect.right() - 150.0, y + 2.0, 52.0, 18.0)
            .intersection(clip)
            .unwrap_or(clip),
        theme.text_dim,
        11.0,
        15.0,
        FontFace::Sans,
    );
    let mut parent_x = rect.right() - 94.0;
    for parent in &detail.parents {
        let short = parent.chars().take(7).collect::<String>();
        let parent_rect = Rect::new(parent_x, y + 2.0, 46.0, 18.0);
        scene.text(
            short,
            [parent_x, y + 2.0],
            parent_rect.intersection(clip).unwrap_or(clip),
            theme.accent,
            11.0,
            15.0,
            FontFace::Monospace,
        );
        scene.hit_clipped(
            parent_rect,
            clip,
            UiAction::JumpToCommit(parent.clone()),
            CursorHint::Pointer,
            None,
        );
        parent_x += 48.0;
    }

    y += 52.0;

    let modified = detail
        .files
        .iter()
        .filter(|file| file.kind == ChangeKind::Modified)
        .count();
    let added = detail
        .files
        .iter()
        .filter(|file| file.kind == ChangeKind::Added)
        .count();
    let stats_x = rect.x + 16.0;
    scene.text(
        format!("{modified} MODIFIED").to_uppercase(),
        [stats_x, y + 6.0],
        Rect::new(stats_x, y, 92.0, 26.0)
            .intersection(clip)
            .unwrap_or(clip),
        theme.orange,
        11.0,
        15.0,
        FontFace::Sans,
    );
    scene.text(
        format!("+ {added} ADDED").to_uppercase(),
        [stats_x + 96.0, y + 6.0],
        Rect::new(stats_x + 96.0, y, 84.0, 26.0)
            .intersection(clip)
            .unwrap_or(clip),
        theme.green,
        11.0,
        15.0,
        FontFace::Sans,
    );

    // Path | Tree Segmented Control
    let toggle = Rect::new(rect.x + (rect.width - 120.0) / 2.0, y, 120.0, 26.0);
    scene.rounded_rect(1, toggle, clip, theme.panel_alt, theme.panel_alt, RADIUS_MD, 0.0);
    let path_rect = Rect::new(toggle.x, toggle.y, 60.0, 26.0);
    let tree_rect = Rect::new(toggle.x + 60.0, toggle.y, 60.0, 26.0);
    let active_rect = if state.path_tree {
        tree_rect
    } else {
        path_rect
    };
    scene.rounded_rect(
        2,
        active_rect.inset(2.0),
        clip,
        theme.surface_3,
        theme.border_strong,
        RADIUS_MD - 2.0,
        1.0,
    );
    scene.text(
        "PATH",
        [path_rect.x + 12.0, path_rect.y + 6.0],
        path_rect.intersection(clip).unwrap_or(clip),
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
        [tree_rect.x + 12.0, tree_rect.y + 6.0],
        tree_rect.intersection(clip).unwrap_or(clip),
        if state.path_tree {
            theme.text
        } else {
            theme.text_muted
        },
        11.0,
        14.0,
        FontFace::Sans,
    );

    scene.hit_clipped(
        toggle,
        clip,
        UiAction::TogglePathTree,
        CursorHint::Pointer,
        None,
    );

    // View all files checkbox
    checkbox(
        scene,
        Rect::new(rect.right() - 120.0, y, 110.0, 26.0),
        "View all files",
        state.view_all_files,
        UiAction::ToggleViewAllFiles,
        state.hover(),
        theme,
    );

    y += 38.0;

    let file_top = y;
    let file_clip_y = file_top.max(clip.y).min(clip.bottom());
    let file_clip = Rect::new(
        rect.x + 1.0,
        file_clip_y,
        rect.width - 2.0,
        (clip.bottom() - file_clip_y).max(0.0),
    );
    if file_clip.height > 0.0 {
        let first = ((file_clip.y - file_top).max(0.0) / 24.0)
            .floor()
            .to_usize()
            .unwrap_or(0)
            .min(total_files);
        let visible = (file_clip.height / 24.0)
            .ceil()
            .to_usize()
            .unwrap_or(0)
            .saturating_add(2);
        let end = first.saturating_add(visible).min(total_files);
        for index in first..end {
            let row = Rect::new(
                rect.x,
                file_top + index.to_f32().unwrap_or(0.0) * 24.0,
                rect.width,
                24.0,
            );
            if let Some(tree_rows) = &tree_rows {
                match &tree_rows[index] {
                    TreeRow::Folder { name, depth } => {
                        draw_folder_row(scene, state, theme, file_clip, row, name, *depth);
                    }
                    TreeRow::File {
                        path,
                        change,
                        depth,
                    } => {
                        let scope = DiffScope::Commit(detail.id.clone());
                        draw_file_row(
                            scene, state, theme, file_clip, row, path, *change, &scope, *depth,
                            false,
                        );
                    }
                }
            } else {
                let (path, change) = files[index];
                let scope = DiffScope::Commit(detail.id.clone());
                draw_file_row(
                    scene, state, theme, file_clip, row, path, change, &scope, 0, true,
                );
            }
        }
    }
    if total_files > 12 {
        let strip = Rect::new(rect.x + 1.0, rect.bottom() - 30.0, rect.width - 2.0, 30.0);
        scene.rect(2, strip, clip, theme.panel);
        scene.text(
            format!("{total_files} files • scroll to browse"),
            [rect.x + 16.0, rect.bottom() - 25.0],
            Rect::new(rect.x + 12.0, rect.bottom() - 27.0, rect.width - 24.0, 22.0)
                .intersection(clip)
                .unwrap_or(clip),
            theme.text_dim,
            11.0,
            14.0,
            FontFace::Sans,
        );
    }
    let scroll_content_height =
        content_height.max(file_top + scroll - clip.y + total_files.to_f32().unwrap_or(0.0) * 24.0);
    scrollbar(
        scene,
        clip,
        scroll_content_height,
        scroll,
        ScrollTarget::Detail,
        theme,
    );
}

enum TreeRow<'a> {
    Folder {
        name: String,
        depth: usize,
    },
    File {
        path: &'a Path,
        change: Option<&'a FileChange>,
        depth: usize,
    },
}

fn build_tree_rows<'a>(files: &[(&'a Path, Option<&'a FileChange>)]) -> Vec<TreeRow<'a>> {
    let mut directories = BTreeSet::new();
    let mut rows = Vec::with_capacity(files.len());
    for &(path, change) in files {
        let mut directory = PathBuf::new();
        let mut components = path.components().peekable();
        let mut depth = 0;
        while let Some(component) = components.next() {
            let Component::Normal(name) = component else {
                continue;
            };
            if components.peek().is_none() {
                rows.push(TreeRow::File {
                    path,
                    change,
                    depth,
                });
                continue;
            }
            directory.push(name);
            if directories.insert(directory.clone()) {
                rows.push(TreeRow::Folder {
                    name: name.to_string_lossy().into_owned(),
                    depth,
                });
            }
            depth += 1;
        }
    }
    rows
}

fn draw_folder_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    row: Rect,
    name: &str,
    depth: usize,
) {
    let Some(hit_row) = row.intersection(clip) else {
        return;
    };
    if row.contains(state.hover()) {
        scene.rounded_rect(
            1,
            Rect::new(row.x + 4.0, row.y + 1.0, row.width - 8.0, row.height - 2.0),
            clip,
            theme.row_hover,
            theme.row_hover,
            RADIUS_SM,
            0.0,
        );
    }
    let x = row.x + 16.0 + depth.to_f32().unwrap_or(0.0) * 16.0;
    scene.text(
        icons::CHEVRON_DOWN,
        [x, row.y + 5.0],
        hit_row,
        theme.text_muted,
        11.5,
        15.0,
        FontFace::Sans,
    );
    scene.text(
        icons::FOLDER_OPEN,
        [x + 14.0, row.y + 5.0],
        hit_row,
        theme.text_dim,
        11.5,
        15.0,
        FontFace::Sans,
    );
    scene.text(
        name,
        [x + 31.0, row.y + 5.0],
        hit_row,
        theme.text,
        11.5,
        15.0,
        FontFace::Sans,
    );
}

/// Rows shown in the detail file section for the current view flags.
fn collect_rows<'a>(
    state: &AppState,
    detail: &'a CommitDetail,
) -> Vec<(&'a Path, Option<&'a FileChange>)> {
    if state.view_all_files {
        detail
            .all_files
            .iter()
            .map(|path| {
                let change = detail
                    .files
                    .binary_search_by(|file| file.path.as_path().cmp(path.as_path()))
                    .ok()
                    .map(|changed| &detail.files[changed]);
                (path.as_path(), change)
            })
            .collect::<Vec<_>>()
    } else {
        detail
            .files
            .iter()
            .map(|file| (file.path.as_path(), Some(file)))
            .collect::<Vec<_>>()
    }
}

/// Row count of the detail file section; shared with scrollbar metrics.
pub(crate) fn detail_row_count(state: &AppState) -> usize {
    state.detail.as_ref().map_or(0, |detail| {
        let files = collect_rows(state, detail);
        if state.path_tree {
            build_tree_rows(&files).len()
        } else {
            files.len()
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn draw_file_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    row: Rect,
    path: &Path,
    change: Option<&FileChange>,
    commit: &str,
    depth: usize,
    show_directory_prefix: bool,
) {
    let Some(hit_row) = row.intersection(clip) else {
        return;
    };
    let selected = state
        .selected_file
        .as_ref()
        .is_some_and(|request| request.path == path);
    let wash = Rect::new(row.x + 4.0, row.y + 1.0, row.width - 8.0, row.height - 2.0);
    if selected {
        scene.rounded_rect(
            1,
            wash,
            clip,
            theme.row_selected,
            theme.row_selected,
            RADIUS_SM,
            0.0,
        );
    } else if row.contains(state.hover()) {
        scene.rounded_rect(1, wash, clip, theme.row_hover, theme.row_hover, RADIUS_SM, 0.0);
    }

    let marker_style = change.map(|file| match file.kind {
        ChangeKind::Added => (theme.green_muted, theme.green),
        ChangeKind::Deleted | ChangeKind::Conflicted => (theme.red_muted, theme.red),
        ChangeKind::Renamed => (theme.panel_alt, theme.text_muted),
        ChangeKind::Modified | ChangeKind::TypeChanged => (theme.orange_muted, theme.orange),
    });
    let marker = change.map_or(icons::CIRCLE_SMALL, |file| file.kind.marker());
    let mut x = row.x + 16.0 + depth.to_f32().unwrap_or(0.0) * 16.0;
    if let Some((badge_fill, letter)) = marker_style {
        let badge = Rect::new(x - 3.0, row.y + 4.0, 16.0, 16.0);
        scene.rounded_rect(1, badge, hit_row, badge_fill, badge_fill, RADIUS_SM, 0.0);
        scene.text(
            marker,
            [x + 1.0, row.y + 7.0],
            hit_row,
            letter,
            10.0,
            12.0,
            FontFace::Monospace,
        );
    } else {
        scene.text(
            marker,
            [x, row.y + 6.0],
            row.inset(4.0).intersection(clip).unwrap_or(hit_row),
            theme.text_dim,
            13.0,
            16.0,
            FontFace::Monospace,
        );
    }
    x += 18.0;
    let advance = 7.2;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let budget = ((row.right() - 16.0 - x).max(0.0) / advance)
        .floor()
        .to_usize()
        .unwrap_or(0);
    let name_count = name.chars().count();
    let mut elided = false;
    if show_directory_prefix
        && let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        && budget > name_count.saturating_add(2)
    {
        let mut prefix = format!("{}/", parent.display());
        let room = budget - name_count;
        if prefix.chars().count() > room {
            // Elide the front of the directory like GitKraken: `…agent/src/`.
            let tail = prefix
                .chars()
                .rev()
                .take(room.saturating_sub(1))
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            prefix = format!("…{tail}");
            elided = true;
        }
        scene.text(
            &prefix,
            [x, row.y + 5.0],
            row.inset(4.0).intersection(clip).unwrap_or(hit_row),
            theme.text_dim,
            12.0,
            16.0,
            FontFace::Monospace,
        );
        x += prefix.chars().count().to_f32().unwrap_or(0.0) * advance;
    }
    let shown_name = if name_count > budget {
        elided = true;
        let tail = name
            .chars()
            .rev()
            .take(budget.saturating_sub(1))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<String>();
        format!("…{tail}")
    } else {
        name.to_string()
    };
    scene.text(
        shown_name,
        [x, row.y + 5.0],
        row.inset(4.0).intersection(clip).unwrap_or(hit_row),
        theme.text,
        12.0,
        16.0,
        FontFace::Monospace,
    );
    if elided {
        // Hovering an elided row reveals the full original path.
        scene.hit_clipped(
            hit_row,
            clip,
            UiAction::RevealText,
            CursorHint::Default,
            Some(&path.display().to_string()),
        );
    }
    scene.hit_clipped(
        hit_row,
        clip,
        UiAction::SelectFile {
            path: path.to_path_buf(),
            staged: false,
            commit: Some(commit.to_owned()),
        },
        CursorHint::Pointer,
        None,
    );
}

fn format_time(seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(DateTime::<Local>::from)
        .map_or_else(
            || "unknown time".to_owned(),
            |time| time.format("%m/%d/%Y @ %-I:%M %p").to_string(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_rows_group_directories_and_indent_files() {
        let files = [
            (Path::new("src/engine/detail.rs"), None),
            (Path::new("src/lib.rs"), None),
            (Path::new("README.md"), None),
        ];
        let rows = build_tree_rows(&files);
        assert!(matches!(
            &rows[0],
            TreeRow::Folder { name, depth: 0 } if name == "src"
        ));
        assert!(matches!(
            &rows[1],
            TreeRow::Folder { name, depth: 1 } if name == "engine"
        ));
        assert!(matches!(
            &rows[2],
            TreeRow::File { path, depth: 2, .. } if *path == Path::new("src/engine/detail.rs")
        ));
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, TreeRow::Folder { name, .. } if name == "src"))
                .count(),
            1
        );
    }
}
