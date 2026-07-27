//! Command palette state machine: query state, fuzzy filtering, cursor
//! movement, and command dispatch. Drawing lives in `views::palette`.

use crate::{
    app::state::Overlay,
    ui::{action::UiAction, icons},
};

/// Mutable query and selection shared by the two command-palette skins.
#[derive(Clone, Debug, Default)]
pub(crate) struct PaletteState {
    pub(crate) query: crate::ui::TextField,
    pub(crate) cursor: usize,
    pub(crate) scroll: usize,
}

/// Which command table and chrome the palette presents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaletteSkin {
    General,
    Editor,
}

/// One palette entry: display label, dispatched action, optional key chips.
#[derive(Clone)]
pub(crate) struct PaletteCommand {
    pub(crate) label: &'static str,
    pub(crate) action: UiAction,
    pub(crate) keybinding: Option<&'static [&'static str]>,
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

/// Returns the full command table backing one palette skin.
pub(crate) fn commands(skin: PaletteSkin) -> &'static [PaletteCommand] {
    match skin {
        PaletteSkin::General => GENERAL,
        PaletteSkin::Editor => EDITOR,
    }
}

/// Returns one filtered command's display label and compact keybinding.
pub(crate) fn command_presentation(
    skin: PaletteSkin,
    source_index: usize,
) -> Option<(&'static str, String)> {
    let command = commands(skin).get(source_index)?;
    let keybinding = command
        .keybinding
        .map_or_else(String::new, |keys| keys.join(" "));
    Some((command.label, keybinding))
}

/// Maps a palette overlay to its skin; `None` for non-palette overlays.
pub(crate) fn skin(overlay: &Overlay) -> Option<PaletteSkin> {
    match overlay {
        Overlay::CommandPalette => Some(PaletteSkin::General),
        Overlay::EditorPalette => Some(PaletteSkin::Editor),
        _ => None,
    }
}

/// Rewinds cursor and scroll to the top of the filtered list.
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

/// Steps the selection cursor with wrap-around and keeps it scrolled into view.
pub(crate) fn move_cursor(palette: &mut PaletteState, skin: PaletteSkin, delta: i32) {
    /// Rows visible in the palette list without scrolling.
    const VISIBLE_ROWS: usize = 8;
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
    if palette.cursor < palette.scroll {
        palette.scroll = palette.cursor;
    } else if palette.cursor >= palette.scroll + VISIBLE_ROWS {
        palette.scroll = palette.cursor + 1 - VISIBLE_ROWS;
    }
}

/// Resolves the action behind one visible row of the filtered list.
pub(crate) fn action_for(
    skin: PaletteSkin,
    filtered_index: usize,
    query: &str,
) -> Option<UiAction> {
    filtered_indices(skin, query)
        .get(filtered_index)
        .map(|index| commands(skin)[*index].action.clone())
}
