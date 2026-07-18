//! Semantic Nerd Font glyphs used by the application chrome.
//!
//! Chrome uses Codicons; shortcut labels use Material Design keyboard glyphs.

/// Account or profile affordance.
pub(crate) const ACCOUNT: &str = "\u{EB99}";
/// Add or create affordance.
pub(crate) const ADD: &str = "\u{EA60}";
/// Stash or archive affordance.
pub(crate) const ARCHIVE: &str = "\u{EA98}";
/// Downward navigation affordance.
pub(crate) const ARROW_DOWN: &str = "\u{EA9A}";
/// Forward transition affordance.
pub(crate) const ARROW_RIGHT: &str = "\u{EA9C}";
/// Upward navigation affordance.
pub(crate) const ARROW_UP: &str = "\u{EAA1}";
/// Notification affordance.
pub(crate) const BELL: &str = "\u{EAA2}";
/// Branch or source-control affordance.
pub(crate) const BRANCH: &str = "\u{EA68}";
/// Confirmed or selected state.
pub(crate) const CHECK: &str = "\u{EAB2}";
/// Expanded or dropdown affordance.
pub(crate) const CHEVRON_DOWN: &str = "\u{EAB4}";
/// Backward or collapse affordance.
pub(crate) const CHEVRON_LEFT: &str = "\u{EAB5}";
/// Forward or expand affordance.
pub(crate) const CHEVRON_RIGHT: &str = "\u{EAB6}";
/// Upward or collapse affordance.
pub(crate) const CHEVRON_UP: &str = "\u{EAB7}";
/// Spaced forward chevron used between path components.
pub(crate) const BREADCRUMB_SEPARATOR: &str = "  \u{EAB6}  ";
/// Unselected circular state.
pub(crate) const CIRCLE: &str = "\u{EABC}";
/// Selected circular state.
pub(crate) const CIRCLE_FILLED: &str = "\u{EA71}";
/// Neutral compact marker.
pub(crate) const CIRCLE_SMALL: &str = "\u{EC07}";
/// Close or dismiss affordance.
pub(crate) const CLOSE: &str = "\u{EA76}";
/// Cloud-backed resource affordance.
pub(crate) const CLOUD: &str = "\u{EBAA}";
/// Added-diff marker.
pub(crate) const DIFF_ADDED: &str = "\u{EADC}";
/// Modified-diff marker.
pub(crate) const DIFF_MODIFIED: &str = "\u{EADE}";
/// Removed-diff marker.
pub(crate) const DIFF_REMOVED: &str = "\u{EADF}";
/// External-link affordance.
pub(crate) const EXTERNAL_LINK: &str = "\u{EB14}";
/// Remote fetch affordance.
pub(crate) const FETCH: &str = "\u{EC1D}";
/// Closed folder affordance.
pub(crate) const FOLDER: &str = "\u{EA83}";
/// Open folder affordance.
pub(crate) const FOLDER_OPEN: &str = "\u{EAF7}";
/// Settings affordance.
pub(crate) const GEAR: &str = "\u{EAF8}";
/// Gitea forge affordance.
pub(crate) const GITEA: &str = "\u{F339}";
/// GitHub forge affordance.
pub(crate) const GITHUB: &str = "\u{EA84}";
/// Pull-request affordance.
pub(crate) const GIT_PULL_REQUEST: &str = "\u{EA64}";
/// Generic URL or web-host affordance.
pub(crate) const GLOBE: &str = "\u{EB01}";
/// Empty or home tab affordance.
pub(crate) const HOME: &str = "\u{EB06}";
/// Restore or history affordance.
pub(crate) const HISTORY: &str = "\u{EA82}";
/// Issue tracker affordance.
pub(crate) const ISSUES: &str = "\u{EB0C}";
/// macOS Command key in shortcut labels.
pub(crate) const KEY_COMMAND: &str = "\u{F0633}";
/// macOS Command-Return chord in shortcut labels.
pub(crate) const KEY_COMMAND_RETURN: &str = "\u{F0633}\u{F0311}";
/// macOS Command-Shift-A chord in shortcut labels.
pub(crate) const KEY_COMMAND_SHIFT_A: &str = "\u{F0633}\u{F0636}A";
/// Application layout affordance.
pub(crate) const LAYOUT: &str = "\u{EBEB}";
/// Flat-list layout affordance.
pub(crate) const LIST: &str = "\u{EB84}";
/// Tree-list layout affordance.
pub(crate) const LIST_TREE: &str = "\u{EB86}";
/// In-progress state.
pub(crate) const LOADING: &str = "\u{EB19}";
/// Action-menu affordance.
pub(crate) const MENU: &str = "\u{EB94}";
/// New repository or folder affordance.
pub(crate) const NEW_FOLDER: &str = "\u{EA80}";
/// Team or organization affordance.
pub(crate) const ORGANIZATION: &str = "\u{EA7E}";
/// Redo affordance.
pub(crate) const REDO: &str = "\u{EBB0}";
/// Remove or decrement affordance.
pub(crate) const REMOVE: &str = "\u{EB3B}";
/// Remote resource affordance.
pub(crate) const REMOTE: &str = "\u{EB3A}";
/// Repository affordance.
pub(crate) const REPOSITORY: &str = "\u{EA62}";
/// Repository clone affordance.
pub(crate) const REPOSITORY_CLONE: &str = "\u{EB3E}";
/// Repository pull affordance.
pub(crate) const REPOSITORY_PULL: &str = "\u{EB40}";
/// Repository push affordance.
pub(crate) const REPOSITORY_PUSH: &str = "\u{EB41}";
/// Search or filter affordance.
pub(crate) const SEARCH: &str = "\u{EA6D}";
/// AI-assisted action affordance.
pub(crate) const SPARKLE: &str = "\u{EC10}";
/// Vertically split layout affordance.
pub(crate) const SPLIT_VERTICAL: &str = "\u{EB57}";
/// Git submodule affordance.
pub(crate) const SUBMODULE: &str = "\u{EAEC}";
/// Git tag affordance.
pub(crate) const TAG: &str = "\u{EA66}";
/// Terminal affordance.
pub(crate) const TERMINAL: &str = "\u{EA85}";
/// Undo or discard affordance.
pub(crate) const UNDO: &str = "\u{EAE2}";
/// Linked worktree affordance.
pub(crate) const WORKSPACE: &str = "\u{EBC1}";

/// True when a character belongs to a Unicode private-use area used by icon fonts.
pub(crate) fn is_private_use(character: char) -> bool {
    matches!(
        u32::from(character),
        0xE000..=0xF8FF
            | 0x000F_0000..=0x000F_FFFD
            | 0x0010_0000..=0x0010_FFFD
    )
}

/// True when text contains only icon glyphs and spacing.
pub(crate) fn is_icon_only(text: &str) -> bool {
    let mut has_icon = false;
    for character in text.chars() {
        if is_private_use(character) {
            has_icon = true;
        } else if !character.is_whitespace() {
            return false;
        }
    }
    has_icon
}
