use std::path::PathBuf;

use crate::git::models::{DiffLineSelection, DiffScope};

/// Draggable divider whose position is persisted in application state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResizeTarget {
    Sidebar,
    /// Boundary after a sidebar data section (local through stashes).
    SidebarSection(u8),
    DetailPanel,
    RefColumn,
    GraphColumn,
    MessageColumn,
    TerminalPane,
    /// Bottom edge of the commit-detail message block.
    DetailMessage,
}
/// Identifies a scrollable surface for scrollbar interactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScrollTarget {
    Graph,
    SidebarLocal,
    SidebarRemote,
    SidebarWorktrees,
    SidebarStashes,
    SidebarTags,
    Detail,
    WipUnstaged,
    WipStaged,
    Diff,
    Preferences,
}

/// Which list a right-clicked file row belongs to; drives context-menu items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FileContextScope {
    Unstaged,
    Staged,
    Committed(String),
}

/// Hosted forge selected in the Add Remote form's provider tab strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AddRemoteProvider {
    /// Raw pull/push URLs entered by hand.
    #[default]
    Url,
    /// `owner/repo` slug resolved against github.com.
    GitHub,
    /// `owner/repo` slug resolved against a self-hosted Gitea instance.
    Gitea,
}

/// Semantic actions emitted by hit-tested UI regions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UiAction {
    SelectCommit(String),
    SelectWip,
    SelectFile {
        path: PathBuf,
        /// Tree/index/worktree pair the diff should compare.
        scope: DiffScope,
    },
    StageFile(PathBuf),
    ToggleSection(String),
    /// Collapses or expands the sidebar to/from the narrow icon rail.
    ToggleSidebarCollapse,
    /// Expands a collapsed sidebar and focuses the named section.
    ExpandSidebarSection(String),
    UnstageFile(PathBuf),
    /// Opens the GitKraken-style file context menu for a right-clicked row.
    OpenFileContext {
        path: PathBuf,
        scope: FileContextScope,
    },
    FileContextStage,
    FileContextUnstage,
    FileContextDiscard,
    FileContextStashFile,
    /// Appends the carried pattern to the repository .gitignore.
    FileContextIgnore(String),
    FileContextHistory,
    FileContextOpenEditor,
    FileContextOpenDefault,
    FileContextReveal,
    FileContextCopyPath,
    FileContextDelete,
    StageDiffLines {
        path: PathBuf,
        lines: Vec<DiffLineSelection>,
    },
    UnstageDiffLines {
        path: PathBuf,
        lines: Vec<DiffLineSelection>,
    },
    DiscardDiffLines {
        path: PathBuf,
        lines: Vec<DiffLineSelection>,
    },
    CopyDiffLines(Vec<String>),
    CopyDiffText,
    BeginDiffSelection(usize),
    BeginDiffTextSelection {
        row: usize,
        side: u8,
        column: usize,
        clicks: u8,
    },
    CommitAndPush,
    ToggleCommitOptions,
    ToggleFileSelection(PathBuf),
    StageSelection,
    StageAll,
    UnstageAll,
    Commit,
    ToggleAmend,
    FocusCommitSummary,
    FocusCommitBody,
    ToggleBranchMenu,
    FocusBranchFilter,
    FocusTabFilter,
    CheckoutBranch(String),
    /// Single click on a branch chip/row: jump to the branch tip; a second
    /// click on the same ref within the double-click window checks it out.
    BranchClick(String),
    /// Single click on a tag chip/row: jump to the tagged commit; a second
    /// click within the double-click window checks the tag out (detached).
    TagClick(String),
    OpenBranchContext(String),
    BranchContextCheckout,
    BranchContextFastForward,
    BranchContextMerge,
    BranchContextRebase,
    ToggleCreateBranch,
    FocusCreateBranch,
    CreateBranch,
    /// Opens the Add Remote form from the sidebar REMOTE header.
    OpenAddRemote,
    /// Switches the Add Remote form to another provider tab.
    SelectAddRemoteProvider(AddRemoteProvider),
    FocusAddRemoteName,
    FocusAddRemoteUrl,
    FocusAddRemotePushUrl,
    FocusAddRemoteRepo,
    FocusAddRemoteHost,
    /// Submits the Add Remote form: registers and fetches the new remote.
    AddRemote,
    OpenCommitContext(String),
    CommitContextCheckout,
    CommitContextCreateBranch,
    CommitContextCreateTag,
    CommitContextCherryPick,
    CommitContextRevert,
    CommitContextReset(String),
    CopyCommitSha,
    CopyCommitMessage,
    /// Opens the amend modal prefilled with the HEAD commit's message.
    CommitContextEditMessage,
    /// Amends HEAD with the edited summary/body from the modal.
    ConfirmEditMessage,
    FocusEditMessageSummary,
    FocusEditMessageBody,
    /// Prompts for a destination and writes `format-patch` output there.
    CommitContextCreatePatch,
    /// Copies the right-clicked branch name to the clipboard.
    BranchContextCopyName,
    BranchContextDelete,
    BranchContextRename,
    FocusRenameBranch,
    RenameBranch,
    FocusCreateTagName,
    FocusCreateTagMessage,
    CreateTag,
    OpenStashContext(usize),
    StashContextApply,
    StashContextPop,
    StashContextDrop,
    OpenTagContext(String),
    TagContextCheckout,
    TagContextDelete,
    /// Copies the right-clicked tag name to the clipboard.
    TagContextCopyName,
    /// Drop-menu: merges the dragged ref into the drop target (target is HEAD).
    DropMerge,
    /// Drop-menu: rebases the dragged branch onto the drop target.
    DropRebaseOnto,
    /// Drop-menu: fast-forwards the drop-target branch to the dragged ref.
    DropFastForward,
    Undo,
    Redo,
    Fetch,
    Pull,
    TogglePullOptions,
    SetPullOperation(crate::git::models::PullOperation),
    Push,
    Stash,
    PopStash,
    OpenTerminal,
    FocusTerminal,
    ToggleLfsMenu,
    LfsCheckout,
    LfsPull,
    LfsPush,
    LfsPrune,
    ToggleActionsMenu,
    ToggleSearch,
    FocusSearch,
    PreviousSearchResult,
    NextSearchResult,
    CloseSearch,
    ToggleCommandPalette,
    ToggleEditorPalette,
    FocusPalette,
    PalettePrevious,
    PaletteNext,
    ExecutePaletteCommand(usize),
    ToggleDiffSearch,
    CloseDiffSearch,
    PreviousDiffSearch,
    NextDiffSearch,
    ToggleTabSwitcher,
    NewTab,
    SelectTab(usize),
    CloseTab(usize),
    OpenRepository(PathBuf),
    OpenRepositoryPicker,
    CreateRepositoryPicker,
    ToggleCloneForm,
    FocusWelcomeSearch,
    FocusCloneUrl,
    PickCloneDestination,
    CloneRepository,
    OpenExternalUrl(String),
    ToggleNotifications,
    OpenPreferences,
    ExitPreferences,
    SelectPreferencePage(String),
    TogglePreference(String),
    AdjustPreference {
        key: String,
        delta: i32,
    },
    FocusPreferenceText(String),
    AddCommitProfile,
    SelectCommitProfile(String),
    BrowsePreferencePath(String),
    InitializeGitflow,
    ApplySparseCheckout,
    DisableSparseCheckout,
    AddLfsPattern,
    OpenExternalEditor,
    OpenExternalTerminal,
    TogglePathTree,
    ToggleViewAllFiles,
    CloseDetail,
    CloseDiff,
    ShowFileView,
    ShowDiffView,
    ToggleDiffLayout,
    ToggleDiffScope,
    PreviousHunk,
    NextHunk,
    /// Scrolls the diff canvas so the given row lands near the top (minimap jump).
    SeekDiffRow(usize),
    ToggleFileHistory,
    BeginResize(ResizeTarget),
    JumpToCommit(String),
    /// Jumps a scroll surface to the content fraction under the pointer.
    ScrollbarJump(ScrollTarget),
    /// Starts direct manipulation of a scrollbar thumb.
    BeginScrollbarDrag(ScrollTarget),
    /// Passive hover region revealing the full text of a truncated element.
    RevealText,
    ShowAiStatus,
    DismissOverlay,
}

/// A cursor hint attached to a hit-tested region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CursorHint {
    #[default]
    Default,
    Pointer,
    ResizeHorizontal,
    ResizeVertical,
    Text,
}

/// A semantic UI hit target generated during immediate-mode layout.
#[derive(Clone, Debug)]
pub(crate) struct HitRegion {
    pub(crate) rect: super::geometry::Rect,
    pub(crate) action: UiAction,
    pub(crate) cursor: CursorHint,
    pub(crate) tooltip: Option<String>,
}
