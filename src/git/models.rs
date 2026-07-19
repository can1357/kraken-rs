use std::path::PathBuf;

/// The default operation dispatched by the Pull toolbar control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub(crate) enum PullOperation {
    FetchAll,
    #[default]
    FastForward,
    FastForwardOnly,
    Rebase,
}
/// A local or remote branch shown in the sidebar and checkout menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BranchInfo {
    pub(crate) name: String,
    pub(crate) target: String,
    pub(crate) current: bool,
    pub(crate) remote: bool,
    pub(crate) upstream: Option<String>,
}

/// A branch, tag, HEAD, or worktree label attached to a commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RefLabel {
    pub(crate) name: String,
    pub(crate) kind: RefKind,
}

/// Visual category for a repository reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefKind {
    Head,
    LocalBranch,
    RemoteBranch,
    Tag,
    Worktree,
}

/// Consolidated branch presence attached to a commit row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitBranchRef {
    /// Branch name without a remote prefix.
    pub(crate) branch_short_name: String,
    pub(crate) is_local: bool,
    /// Remote names that have this branch at this commit.
    pub(crate) remote_names: Vec<String>,
    pub(crate) is_head: bool,
    pub(crate) is_tag: bool,
}

/// Immutable commit data needed by the virtualized graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitSummary {
    pub(crate) id: String,
    pub(crate) short_id: String,
    pub(crate) subject: String,
    /// First body line, shown dimmed inline after the subject in the graph.
    pub(crate) description: String,
    pub(crate) author: String,
    pub(crate) email: String,
    pub(crate) authored_seconds: i64,
    pub(crate) parents: Vec<String>,
    /// True when no remote branch can reach this commit (an unpushed commit).
    pub(crate) is_local: bool,
    pub(crate) refs: Vec<RefLabel>,
    pub(crate) branch_refs: Vec<CommitBranchRef>,
}

/// A stash entry rendered in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StashInfo {
    pub(crate) index: usize,
    pub(crate) name: String,
    pub(crate) target: String,
}

/// A linked worktree rendered below the branch lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeInfo {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) branch: Option<String>,
    pub(crate) target: Option<String>,
    pub(crate) changes: usize,
}

/// Git's semantic classification for one side of a file status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    TypeChanged,
    Conflicted,
}

impl ChangeKind {
    /// Returns the terse status marker used by file rows.
    pub(crate) const fn marker(self) -> &'static str {
        match self {
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::TypeChanged => "T",
            Self::Conflicted => "!",
        }
    }
}

/// Combined index and worktree status for one repository-relative path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorkingFile {
    pub(crate) path: PathBuf,
    pub(crate) old_path: Option<PathBuf>,
    pub(crate) staged: Option<ChangeKind>,
    pub(crate) unstaged: Option<ChangeKind>,
}

/// Current index/worktree state used by the WIP node and commit form.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkingTree {
    pub(crate) files: Vec<WorkingFile>,
}

impl WorkingTree {
    /// Counts paths with index changes.
    pub(crate) fn staged_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.staged.is_some())
            .count()
    }

    /// Counts paths with worktree changes.
    pub(crate) fn unstaged_count(&self) -> usize {
        self.files
            .iter()
            .filter(|file| file.unstaged.is_some())
            .count()
    }

    /// Reports whether the worktree needs a WIP row.
    pub(crate) fn is_dirty(&self) -> bool {
        !self.files.is_empty()
    }
}

/// One bounded repository snapshot delivered from the Git worker.
#[derive(Clone, Debug)]
pub(crate) struct RepoSnapshot {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) head: String,
    pub(crate) head_id: Option<String>,
    pub(crate) branches: Vec<BranchInfo>,
    pub(crate) tags: Vec<RefLabel>,
    pub(crate) stashes: Vec<StashInfo>,
    pub(crate) worktrees: Vec<WorktreeInfo>,
    pub(crate) commits: Vec<CommitSummary>,
    pub(crate) working: WorkingTree,
    pub(crate) loaded_limit: usize,
    pub(crate) has_more: bool,
    /// Digest of all references when the snapshot was taken; lets the
    /// filesystem watcher skip history re-walks while no reference moves.
    pub(crate) refs_sig: u64,
}

impl RepoSnapshot {
    /// Counts current and linked worktrees that need visible WIP rows.
    pub(crate) fn wip_rows(&self) -> usize {
        usize::from(self.working.is_dirty()).saturating_add(
            self.worktrees
                .iter()
                .filter(|worktree| worktree.changes > 0)
                .count(),
        )
    }
}

/// A path and change category from a committed tree diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileChange {
    pub(crate) path: PathBuf,
    pub(crate) old_path: Option<PathBuf>,
    pub(crate) kind: ChangeKind,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
}

/// Full metadata and changed files for the selected commit.
#[derive(Clone, Debug)]
pub(crate) struct CommitDetail {
    pub(crate) id: String,
    pub(crate) short_id: String,
    pub(crate) subject: String,
    pub(crate) body: String,
    pub(crate) author: String,
    pub(crate) email: String,
    pub(crate) authored_seconds: i64,
    pub(crate) parents: Vec<String>,
    pub(crate) files: Vec<FileChange>,
    /// Every path in the commit tree, present only when the detail was
    /// requested with the tree included ("View all files").
    pub(crate) all_files: Option<Vec<PathBuf>>,
    pub(crate) conflicts: Vec<PathBuf>,
}

/// Combined changes across a multi-selected, inclusive commit range.
///
/// `files` is the tree diff between the oldest selected commit's first parent
/// and the newest selected commit, matching GitKraken's multi-select panel.
#[derive(Clone, Debug)]
pub(crate) struct RangeDetail {
    pub(crate) oldest: String,
    pub(crate) newest: String,
    pub(crate) oldest_short: String,
    pub(crate) newest_short: String,
    pub(crate) files: Vec<FileChange>,
}

/// Selects the tree/index/worktree pair used to create a file diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiffScope {
    Commit(String),
    /// Inclusive multi-commit selection: `parent(oldest)` vs `newest` trees.
    CommitRange { oldest: String, newest: String },
    Staged,
    Unstaged,
}

/// Request for a single file's bespoke split-diff document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffRequest {
    pub(crate) path: PathBuf,
    pub(crate) scope: DiffScope,
}

/// Semantic background category for one aligned split-diff row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffRowKind {
    Context,
    Changed,
    Added,
    Deleted,
    Hunk,
}

/// One aligned old/new row rendered by the custom diff canvas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffRow {
    pub(crate) old_number: Option<u32>,
    pub(crate) new_number: Option<u32>,
    pub(crate) old_text: String,
    pub(crate) new_text: String,
    pub(crate) kind: DiffRowKind,
    pub(crate) old_mark: Option<(usize, usize)>,
    pub(crate) new_mark: Option<(usize, usize)>,
}
/// An aligned changed row selected for a line-level index or worktree mutation.
///
/// Both coordinates are retained for replacements, whose old and new lines must
/// move together to avoid staging only half of the edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DiffLineSelection {
    pub(crate) old_line: Option<u32>,
    pub(crate) new_line: Option<u32>,
}

/// A parsed, aligned diff for one real repository file.
#[derive(Clone, Debug)]
pub(crate) struct DiffDocument {
    pub(crate) path: PathBuf,
    pub(crate) scope: DiffScope,
    pub(crate) old_label: String,
    pub(crate) new_label: String,
    pub(crate) rows: Vec<DiffRow>,
    pub(crate) content: Option<String>,
    pub(crate) hunks: Vec<usize>,
    pub(crate) binary: bool,
}

/// User-authored commit fields passed to the backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CommitInput {
    pub(crate) summary: String,
    pub(crate) body: String,
    pub(crate) amend: bool,
}
