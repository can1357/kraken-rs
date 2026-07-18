use crate::{
    app::state::{AppState, Overlay},
    ui::{
        FontFace, RADIUS_MD, RADIUS_SM, Rect, Scene, Theme,
        action::{CursorHint, UiAction},
        icons,
    },
};

/// Mutable query and selection shared by the two command-palette skins.
#[derive(Clone, Debug, Default)]
pub(crate) struct PaletteState {
    pub(crate) query: crate::ui::TextField,
    pub(crate) cursor: usize,
    pub(crate) scroll: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteSkin {
    General,
    Editor,
}

#[derive(Clone)]
struct PaletteCommand {
    label: &'static str,
    action: UiAction,
    keybinding: Option<&'static [&'static str]>,
}

const GENERAL: &[PaletteCommand] = &[
    PaletteCommand {
        label: "Close Diff",
        action: UiAction::CloseDiff,
        keybinding: None,
    },
    PaletteCommand {
        label: "Commit",
        action: UiAction::Commit,
        keybinding: None,
    },
    PaletteCommand {
        label: "Create Branch",
        action: UiAction::ToggleCreateBranch,
        keybinding: None,
    },
    PaletteCommand {
        label: "Fetch All",
        action: UiAction::Fetch,
        keybinding: None,
    },
    PaletteCommand {
        label: "Open External Editor",
        action: UiAction::OpenExternalEditor,
        keybinding: None,
    },
    PaletteCommand {
        label: "Open External Terminal",
        action: UiAction::OpenExternalTerminal,
        keybinding: None,
    },
    PaletteCommand {
        label: "Open Preferences",
        action: UiAction::OpenPreferences,
        keybinding: Some(&[icons::KEY_COMMAND, ","]),
    },
    PaletteCommand {
        label: "Open Terminal",
        action: UiAction::OpenTerminal,
        keybinding: None,
    },
    PaletteCommand {
        label: "Pop Stash",
        action: UiAction::PopStash,
        keybinding: None,
    },
    PaletteCommand {
        label: "Pull",
        action: UiAction::Pull,
        keybinding: None,
    },
    PaletteCommand {
        label: "Push",
        action: UiAction::Push,
        keybinding: None,
    },
    PaletteCommand {
        label: "Stage All Changes",
        action: UiAction::StageAll,
        keybinding: None,
    },
    PaletteCommand {
        label: "Stash Changes",
        action: UiAction::Stash,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle Amend",
        action: UiAction::ToggleAmend,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle File History",
        action: UiAction::ToggleFileHistory,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle Path Tree",
        action: UiAction::TogglePathTree,
        keybinding: None,
    },
    PaletteCommand {
        label: "Unstage All Changes",
        action: UiAction::UnstageAll,
        keybinding: None,
    },
    PaletteCommand {
        label: "View Diff",
        action: UiAction::ShowDiffView,
        keybinding: None,
    },
    PaletteCommand {
        label: "View File",
        action: UiAction::ShowFileView,
        keybinding: None,
    },
    PaletteCommand {
        label: "View Working Directory Changes",
        action: UiAction::SelectWip,
        keybinding: None,
    },
];

const EDITOR: &[PaletteCommand] = &[
    PaletteCommand {
        label: "Close Diff",
        action: UiAction::CloseDiff,
        keybinding: None,
    },
    PaletteCommand {
        label: "Fetch All",
        action: UiAction::Fetch,
        keybinding: None,
    },
    PaletteCommand {
        label: "Next Change",
        action: UiAction::NextHunk,
        keybinding: None,
    },
    PaletteCommand {
        label: "Open Preferences",
        action: UiAction::OpenPreferences,
        keybinding: Some(&[icons::KEY_COMMAND, ","]),
    },
    PaletteCommand {
        label: "Previous Change",
        action: UiAction::PreviousHunk,
        keybinding: None,
    },
    PaletteCommand {
        label: "Pull",
        action: UiAction::Pull,
        keybinding: None,
    },
    PaletteCommand {
        label: "Push",
        action: UiAction::Push,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle File History",
        action: UiAction::ToggleFileHistory,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle File View",
        action: UiAction::ShowFileView,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle Split View",
        action: UiAction::ToggleDiffLayout,
        keybinding: None,
    },
    PaletteCommand {
        label: "Toggle Diff View",
        action: UiAction::ShowDiffView,
        keybinding: None,
    },
];

fn commands(skin: PaletteSkin) -> &'static [PaletteCommand] {
    match skin {
        PaletteSkin::General => GENERAL,
        PaletteSkin::Editor => EDITOR,
    }
}

pub(crate) fn skin(overlay: &Overlay) -> Option<PaletteSkin> {
    match overlay {
        Overlay::CommandPalette => Some(PaletteSkin::General),
        Overlay::EditorPalette => Some(PaletteSkin::Editor),
        _ => None,
    }
}

pub(crate) fn reset_selection(palette: &mut PaletteState) {
    palette.cursor = 0;
    palette.scroll = 0;
}

/// Returns command indices in their source order after a fuzzy subsequence match.
pub(crate) fn filtered_indices(skin: PaletteSkin, query: &str) -> Vec<usize> {
    let mut matched = commands(skin)
        .iter()
        .enumerate()
        .filter_map(|(index, command)| fuzzy_score(command.label, query).map(|_| index))
        .collect::<Vec<_>>();
    matched.sort_by_key(|index| commands(skin)[*index].label);
    matched
}

fn fuzzy_score(label: &str, query: &str) -> Option<i32> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let label = label.to_lowercase();
    let mut search_from = 0;
    let mut previous = None;
    let mut score = 0;
    for needle in query.chars() {
        let (relative, candidate) = label[search_from..]
            .char_indices()
            .find(|(_, candidate)| *candidate == needle)?;
        let position = search_from + relative;
        score += if previous.is_some_and(|last| position == last + 1) {
            4
        } else {
            1
        };
        previous = Some(position);
        search_from = position + candidate.len_utf8();
    }
    if label.starts_with(&query) {
        score += 8;
    }
    Some(score)
}

pub(crate) fn move_cursor(palette: &mut PaletteState, skin: PaletteSkin, delta: i32) {
    let count = filtered_indices(skin, &palette.query).len();
    if count == 0 {
        reset_selection(palette);
        return;
    }
    palette.cursor = if delta < 0 {
        palette.cursor.checked_sub(1).unwrap_or(count - 1)
    } else {
        (palette.cursor + 1) % count
    };
    const VISIBLE_ROWS: usize = 8;
    if palette.cursor < palette.scroll {
        palette.scroll = palette.cursor;
    } else if palette.cursor >= palette.scroll + VISIBLE_ROWS {
        palette.scroll = palette.cursor + 1 - VISIBLE_ROWS;
    }
}

pub(crate) fn action_for(
    skin: PaletteSkin,
    filtered_index: usize,
    query: &str,
) -> Option<UiAction> {
    filtered_indices(skin, query)
        .get(filtered_index)
        .map(|index| commands(skin)[*index].action.clone())
}

pub(crate) fn popup_rect(state: &AppState, skin: PaletteSkin) -> Rect {
    let width = state.width as f32;
    match skin {
        PaletteSkin::General => Rect::new(width * 0.5 - 300.0, 76.0, 600.0, 396.0),
        PaletteSkin::Editor => Rect::new(width * 0.5 - 300.0, 44.0, 600.0, 360.0),
    }
}

pub(crate) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, skin: PaletteSkin) {
    let Some(palette) = &state.palette else {
        return;
    };
    let rect = popup_rect(state, skin);
    let input = Rect::new(rect.x, rect.y, rect.width, 40.0);
    let list = Rect::new(rect.x, input.bottom() + 2.0, rect.width, rect.height - 42.0);
    let viewport = scene.viewport();
    crate::views::overlays::popup_panel(scene, rect, theme);
    scene.rounded_rect(4, input, viewport, theme.input, theme.accent, RADIUS_MD, 1.0);

    let prompt = match skin {
        PaletteSkin::General => "Search for commands and actions (e.g., Open Repo)",
        PaletteSkin::Editor => ">",
    };
    let value = if skin == PaletteSkin::Editor && !palette.query.is_empty() {
        format!(">{}", palette.query)
    } else {
        palette.query.text().to_owned()
    };
    scene.text(
        if value.is_empty() { prompt } else { &value },
        [input.x + 12.0, input.y + 11.0],
        input.inset(9.0),
        if value.is_empty() {
            theme.text_dim
        } else {
            theme.text
        },
        14.0,
        18.0,
        FontFace::Sans,
    );
    if state.focus == crate::app::state::FocusField::Palette {
        let prefix = if skin == PaletteSkin::Editor && !palette.query.is_empty() {
            7.65
        } else {
            0.0
        };
        crate::ui::widgets::caret_overlay(
            scene,
            4,
            [input.x + 12.0 + prefix, input.y + 11.0],
            input.inset(2.0),
            &palette.query,
            7.65,
            18.0,
            theme,
        );
    }
    if skin == PaletteSkin::General {
        scene.text(
            icons::CHEVRON_UP,
            [input.right() - 24.0, input.y + 11.0],
            input,
            theme.text_dim,
            14.0,
            18.0,
            FontFace::Sans,
        );
    }
    scene.hit(input, UiAction::FocusPalette, CursorHint::Text, None);

    let filtered = filtered_indices(skin, &palette.query);
    let row_height = if skin == PaletteSkin::General {
        40.0
    } else {
        26.0
    };
    let visible = (list.height / row_height).floor() as usize;
    for (visible_index, command_index) in filtered
        .iter()
        .skip(palette.scroll)
        .take(visible)
        .enumerate()
    {
        let row_index = palette.scroll + visible_index;
        let row = Rect::new(
            list.x + 1.0,
            list.y + visible_index as f32 * row_height,
            list.width - 2.0,
            row_height,
        );
        let hovered = row.contains(state.mouse);
        if hovered || row_index == palette.cursor {
            let wash = Rect::new(row.x + 4.0, row.y + 1.0, row.width - 8.0, row.height - 2.0);
            let fill = if row_index == palette.cursor {
                theme.row_selected
            } else {
                theme.row_hover
            };
            scene.rounded_rect(4, wash, list, fill, fill, RADIUS_SM, 0.0);
        }
        let command = &commands(skin)[*command_index];
        scene.text(
            command.label,
            [row.x + 12.0, row.y + (row_height - 17.0) * 0.5],
            row.inset(8.0),
            theme.text,
            14.0,
            17.0,
            FontFace::Sans,
        );
        if let Some(keys) = command.keybinding {
            draw_keys(scene, row, keys, theme);
        }
        scene.hit(
            row,
            UiAction::ExecutePaletteCommand(row_index),
            CursorHint::Pointer,
            None,
        );
    }
    if filtered.len() > visible {
        let bar_height = (list.height * visible as f32 / filtered.len() as f32).max(20.0);
        let offset =
            (list.height - bar_height) * palette.scroll as f32 / (filtered.len() - visible) as f32;
        scene.rounded_rect(
            4,
            Rect::new(list.right() - 5.0, list.y + offset, 3.0, bar_height),
            list,
            theme.border_strong,
            theme.border_strong,
            2.0,
            0.0,
        );
    }
}

fn draw_keys(scene: &mut Scene, row: Rect, keys: &[&str], theme: &Theme) {
    let mut x = row.right() - 10.0;
    for key in keys.iter().rev() {
        let width = key.chars().count() as f32 * 7.0 + 12.0;
        x -= width;
        let chip = Rect::new(x, row.y + 4.0, width, row.height - 8.0);
        scene.rounded_rect(4, chip, row, theme.panel_alt, theme.border, RADIUS_SM, 1.0);
        scene.text(
            *key,
            [chip.x + 6.0, chip.y + 3.0],
            chip,
            theme.text_muted,
            11.0,
            13.0,
            FontFace::Monospace,
        );
        x -= 6.0;
    }
}
