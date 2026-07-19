use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use num_traits::ToPrimitive;
use winit::event_loop::EventLoopProxy;

use crate::{
    app::ai::{AiRunner, provider_from_environment},
    git::{
        backend::{Backend, GitBackend, LfsOperation},
        models::{
            CommitDetail, CommitInput, DiffDocument, DiffRequest, DiffRowKind, DiffScope,
            RangeDetail, RepoSnapshot, WorkingTree,
        },
        runner::{GitEvent, GitJob, GitJobKind, GitPayload, GitRunner},
    },
    graph::layout::GraphLayout,
    settings::{RecentRepo, Settings, SettingsStore},
    term::{Terminal, TerminalSnapshot},
    ui::{
        action::{
            AddRemoteProvider, CursorHint, FileContextScope, HitRegion, ResizeTarget, ScrollTarget,
            UiAction,
        },
        geometry::{
            COMMIT_HEADER_HEIGHT, COMMIT_ROW_HEIGHT, CONTENT_TOP, Rect, STATUS_BAR_HEIGHT, px,
        },
        menu::{MenuEntry, MenuSpec},
        scene::Scene,
        text_field::{Jump, TextField},
    },
};

/// One concrete top-bar tab. A missing path is a welcome/new-tab surface.
#[derive(Clone, Debug)]
pub(crate) struct RepoTab {
    pub(crate) title: String,
    pub(crate) path: Option<PathBuf>,
}

/// Center-canvas mode selected by graph, WIP, and file actions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MainView {
    #[default]
    Graph,
    Wip,
    Diff,
}

/// Flyout or context surface shown above the main shell.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum Overlay {
    #[default]
    None,
    Branches,
    Lfs,
    Actions,
    CommitOptions,
    PullOptions,
    DiffSelection,
    Tabs,
    Notifications,
    CreateBranch,
    AddRemote,
    BranchContext(String),
    RenameBranch(String),
    CreateTag(String),
    StashContext(usize),
    TagContext(String),
    CommitContext(String),
    EditCommitMessage(String),
    /// Menu shown after dropping a dragged ref onto another ref.
    DropMenu {
        source: String,
        source_tag: bool,
        target: String,
        target_tag: bool,
    },
    FileContext {
        path: PathBuf,
        scope: FileContextScope,
    },
    Ai,
    CommandPalette,
    EditorPalette,
}

/// Text field currently receiving keyboard input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FocusField {
    #[default]
    None,
    CommitSummary,
    CommitBody,
    Search,
    DiffSearch,
    BranchFilter,
    CreateBranch,
    RenameBranch,
    CreateTagName,
    CreateTagMessage,
    EditMessageSummary,
    EditMessageBody,
    TabFilter,
    WelcomeSearch,
    CloneUrl,
    AddRemoteName,
    AddRemoteUrl,
    AddRemotePushUrl,
    AddRemoteRepo,
    AddRemoteHost,
    Palette,
    PreferenceText,
}

/// Editing keystroke normalized across the winit and automation front-ends.
#[derive(Clone, Copy, Debug)]
pub(crate) struct EditKey {
    pub(crate) kind: EditKeyKind,
    pub(crate) shift: bool,
    pub(crate) alt: bool,
    /// Primary shortcut modifier: Command or Control.
    pub(crate) command: bool,
}

/// Key identity of an [`EditKey`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EditKeyKind {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Backspace,
    Delete,
    /// A character key pressed with the shortcut modifier (copy/paste/…).
    Char(char),
}

/// Field focused when an Add Remote provider tab activates.
pub(crate) fn add_remote_first_field(provider: AddRemoteProvider) -> FocusField {
    match provider {
        AddRemoteProvider::Url => FocusField::AddRemoteName,
        AddRemoteProvider::GitHub => FocusField::AddRemoteRepo,
        AddRemoteProvider::Gitea => FocusField::AddRemoteHost,
    }
}

/// Centered modal rect for the Add Remote form; shared by the overlay
/// renderer and outside-click dismissal.
pub(crate) fn add_remote_popup_rect(width: f32, height: f32) -> Rect {
    Rect::new(width * 0.5 - 240.0, height * 0.5 - 196.0, 480.0, 392.0)
}

/// A validated Add Remote form result, ready to hand to the Git worker.
pub(crate) struct RemoteSubmission {
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) push_url: Option<String>,
}

/// Builds `<host>/<owner>/<repo>.git` from a forge repository slug; full
/// URLs or scp-style paths pasted into the field pass through normalized.
fn hosted_remote_url(host: &str, repo: &str) -> Option<String> {
    let repo = repo.trim().trim_matches('/');
    if repo.is_empty() {
        return None;
    }
    if repo.contains("://") || repo.starts_with("git@") {
        return Some(format!("{}.git", repo.trim_end_matches(".git")));
    }
    repo.contains('/')
        .then(|| format!("{host}/{}.git", repo.trim_end_matches(".git")))
}

#[derive(Clone, Debug)]
enum UndoRecord {
    Head {
        before: String,
        after: String,
        mode: &'static str,
        label: &'static str,
    },
    Checkout {
        previous: String,
        target: String,
    },
    BranchCreate {
        branch: String,
        target: String,
        previous: String,
    },
    BranchDelete {
        branch: String,
        target: String,
    },
}

/// Toolbar Git operations that gray out and spin while their job runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolbarOp {
    Undo,
    Redo,
    Pull,
    Push,
    Stash,
    Pop,
}

/// Number of [`ToolbarOp`] variants, sizing the in-flight counter array.
const TOOLBAR_OPS: usize = 6;

/// An in-flight branch/tag drag from a sidebar row or a graph ref chip.
#[derive(Clone, Debug)]
pub(crate) struct RefDrag {
    /// Ref label as carried by its click action (e.g. `main`, `origin/x`, tag name).
    pub(crate) source: String,
    /// True when the dragged ref is a tag.
    pub(crate) tag: bool,
    /// Pointer position at press; the drag activates beyond a small threshold.
    press: [f32; 2],
    /// True once the pointer moved far enough to be a drag rather than a click.
    pub(crate) active: bool,
    /// Deferred click action dispatched when the press never became a drag.
    click: UiAction,
}

/// Maps a Git job to the toolbar control that dispatched it, if any.
fn toolbar_op(kind: &GitJobKind) -> Option<ToolbarOp> {
    match kind {
        GitJobKind::Pull { .. } => Some(ToolbarOp::Pull),
        GitJobKind::Push { .. } => Some(ToolbarOp::Push),
        GitJobKind::Stash { .. } => Some(ToolbarOp::Stash),
        GitJobKind::PopStash { .. } => Some(ToolbarOp::Pop),
        _ => None,
    }
}

#[derive(Clone, Debug)]
enum PendingMutation {
    Ignore,
    Record(UndoRecord),
    History { record: UndoRecord, undo: bool },
}

/// Deterministic view selected by the screenshot command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreenshotView {
    Graph,
    Wip,
    Diff,
    File,
    Preferences,
    Tabs,
}

/// Mutable UI and repository state shared by windowed and offscreen render paths.
pub(crate) struct AppState {
    pub(crate) width: u32,
    pub(crate) renamed_branch: TextField,
    pub(crate) tag_name: TextField,
    pub(crate) tag_message: TextField,
    pub(crate) edit_summary: TextField,
    pub(crate) edit_body: TextField,
    pub(crate) height: u32,
    pub(crate) mouse: [f32; 2],
    pub(crate) overlay_anchor: [f32; 2],
    pub(crate) snapshot: Option<RepoSnapshot>,
    pub(crate) graph: GraphLayout,
    pub(crate) main_view: MainView,
    pub(crate) overlay: Overlay,
    pub(crate) preferences_open: bool,
    pub(crate) preference_page: String,
    pub(crate) focus: FocusField,
    pub(crate) preference_text_key: Option<String>,
    /// Edit buffer for the focused preference text setting; every edit is
    /// written through to the settings value it mirrors.
    pub(crate) preference_text: TextField,
    pub(crate) terminal_open: bool,
    pub(crate) terminal_focused: bool,
    overlay_focus: FocusField,
    pub(crate) selected_commit: Option<String>,
    /// Every selected commit id, lead included; two or more entries switch the
    /// detail panel to the combined range view.
    pub(crate) selected_commits: HashSet<String>,
    /// Range pivot: the last plainly clicked or toggled-in commit.
    selection_anchor: Option<String>,
    pub(crate) detail: Option<Arc<CommitDetail>>,
    /// Combined-range detail shown while two or more commits are selected.
    pub(crate) range_detail: Option<Arc<RangeDetail>>,
    /// Immutable commit details already fetched for this repository, FIFO-bounded.
    detail_cache: HashMap<String, Arc<CommitDetail>>,
    detail_cache_order: VecDeque<String>,
    range_cache: HashMap<(String, String), Arc<RangeDetail>>,
    /// Commit id and `include_tree` flag of the in-flight `LoadDetail` request.
    pending_detail: Option<(String, bool)>,
    /// Live keyboard modifiers; shift extends and primary toggles graph selection.
    pub(crate) modifier_shift: bool,
    pub(crate) modifier_primary: bool,
    pub(crate) selected_file: Option<DiffRequest>,
    pub(crate) diff: Option<DiffDocument>,
    pub(crate) palette: Option<crate::views::palette::PaletteState>,
    pub(crate) selected_working_files: HashSet<PathBuf>,
    pub(crate) collapsed_sections: HashSet<String>,
    pub(crate) commit_summary: TextField,
    pub(crate) commit_body: TextField,
    pub(crate) amend: bool,
    pub(crate) search: TextField,
    pub(crate) search_cursor: usize,
    pub(crate) diff_search: TextField,
    pub(crate) diff_search_cursor: usize,
    pub(crate) clone_form: bool,
    pub(crate) branch_filter: TextField,
    pub(crate) tab_filter: TextField,
    branch_target: Option<String>,
    pub(crate) new_branch: TextField,
    pub(crate) add_remote_provider: AddRemoteProvider,
    pub(crate) add_remote_name: TextField,
    pub(crate) add_remote_url: TextField,
    pub(crate) add_remote_push_url: TextField,
    pub(crate) add_remote_repo: TextField,
    pub(crate) add_remote_host: TextField,
    pub(crate) welcome_search: TextField,
    pub(crate) clone_url: TextField,
    pub(crate) clone_destination: Option<PathBuf>,
    pub(crate) path_tree: bool,
    pub(crate) view_all_files: bool,
    pub(crate) diff_split: bool,
    pub(crate) diff_file_view: bool,
    pub(crate) file_history: bool,
    pub(crate) diff_selected_rows: HashSet<usize>,
    pub(crate) diff_drag_start: Option<usize>,
    pub(crate) diff_text_selection: Option<((usize, u8, usize), (usize, u8, usize))>,
    diff_text_drag: Option<(usize, u8, usize)>,
    diff_last_click: Option<((usize, u8, usize), Instant, u8)>,
    last_ref_click: Option<(String, Instant)>,
    push_after_commit: bool,
    pub(crate) current_hunk: usize,
    pub(crate) sidebar_width: f32,
    pub(crate) detail_width: f32,
    pub(crate) ref_column_width: f32,
    pub(crate) graph_column_width: f32,
    pub(crate) graph_column_explicit: bool,
    pub(crate) message_column_width: f32,
    pub(crate) graph_scroll: f32,
    pub(crate) sidebar_scroll: f32,
    pub(crate) sidebar_local_scroll: f32,
    pub(crate) sidebar_remote_scroll: f32,
    pub(crate) sidebar_worktrees_scroll: f32,
    pub(crate) sidebar_stashes_scroll: f32,
    pub(crate) sidebar_tags_scroll: f32,
    pub(crate) sidebar_section_fractions: [f32; 5],
    sidebar_section_drag: Option<(u8, f32, [f32; 5])>,
    pub(crate) detail_scroll: f32,
    pub(crate) wip_unstaged_scroll: f32,
    pub(crate) wip_staged_scroll: f32,
    pub(crate) diff_scroll: f32,
    diff_scroll_target: f32,
    diff_scroll_updated: Option<Instant>,
    pub(crate) preferences_scroll: f32,
    pub(crate) busy_jobs: usize,
    inflight_ops: [usize; TOOLBAR_OPS],
    pub(crate) loading_history: bool,
    pub(crate) error: Option<String>,
    pub(crate) toast: Option<String>,
    pub(crate) ai_message: Option<String>,
    pub(crate) ai_loading: bool,
    pub(crate) settings: Settings,
    pub(crate) drag: Option<ResizeTarget>,
    pub(crate) ref_drag: Option<RefDrag>,
    /// Explicit commit-detail message-block height; `0` uses the content size.
    pub(crate) detail_message_height: f32,
    scrollbar_drag: Option<ScrollTarget>,
    pub(crate) terminal_height_fraction: f32,
    pub(crate) hits: Vec<HitRegion>,
    pub(crate) terminal: Option<Terminal>,
    pub(crate) tabs: Vec<RepoTab>,
    pub(crate) active_tab: usize,
    repo_path: Option<PathBuf>,
    generation: u64,
    requested_limit: usize,
    last_fetch: Instant,
    started: Instant,
    event_loop_proxy: Option<EventLoopProxy<super::UserEvent>>,
    settings_store: SettingsStore,
    git: GitRunner,
    ai: AiRunner,
    undo_stack: Vec<UndoRecord>,
    redo_stack: Vec<UndoRecord>,
    pending_mutations: Vec<PendingMutation>,
}

impl AppState {
    /// Starts an interactive model and asynchronously opens the requested repository.
    pub(crate) fn new(
        repo: Option<PathBuf>,
        width: u32,
        height: u32,
        event_loop_proxy: Option<EventLoopProxy<super::UserEvent>>,
    ) -> Self {
        let settings_store = SettingsStore::platform();
        let settings = settings_store.load().unwrap_or_default();
        let initial_limit = settings.initial_commits.max(200);
        let mut state =
            Self::base_with_proxy(width, height, settings_store, settings, event_loop_proxy);
        state.requested_limit = initial_limit;
        if let Some(path) = repo {
            state.open_repository(path);
        }
        state
    }

    /// Loads all data needed by a screenshot synchronously before GPU rendering.
    pub(crate) fn for_screenshot(
        repo: Option<PathBuf>,
        view: ScreenshotView,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let settings_store = SettingsStore::platform();
        let settings = settings_store.load().unwrap_or_default();
        let mut state = Self::base(width, height, settings_store, settings);
        state.requested_limit = state.settings.initial_commits.max(10_000);
        let path = repo.context("--repo must identify a Git repository for screenshots")?;
        let backend = GitBackend::discover(path)?;
        state.repo_path = Some(backend.path().to_path_buf());
        let snapshot = backend.snapshot(state.requested_limit)?;
        state.apply_snapshot(snapshot);

        match view {
            ScreenshotView::Graph => {
                state.main_view = MainView::Graph;
                if let Some(id) = state.selected_commit.clone() {
                    state.detail = Some(Arc::new(backend.commit_detail(&id, false)?));
                }
            }
            ScreenshotView::Wip => {
                state.main_view = MainView::Wip;
                state.select_only(None);
                state.detail = None;
            }
            ScreenshotView::Diff | ScreenshotView::File => {
                state.diff_file_view = view == ScreenshotView::File;
                let working_request = state.snapshot.as_ref().and_then(|snapshot| {
                    snapshot.working.files.iter().find_map(|file| {
                        let scope = if file.unstaged.is_some() {
                            Some(DiffScope::Unstaged)
                        } else if file.staged.is_some() {
                            Some(DiffScope::Staged)
                        } else {
                            None
                        }?;
                        Some(DiffRequest {
                            path: file.path.clone(),
                            scope,
                        })
                    })
                });
                let request = if let Some(request) = working_request {
                    request
                } else {
                    let id = state
                        .selected_commit
                        .clone()
                        .context("repository has no commit to diff")?;
                    let detail = backend.commit_detail(&id, false)?;
                    let path = detail
                        .files
                        .first()
                        .context("selected commit has no changed file")?
                        .path
                        .clone();
                    state.detail = Some(Arc::new(detail));
                    DiffRequest {
                        path,
                        scope: DiffScope::Commit(id),
                    }
                };
                state.diff = Some(backend.diff(&request)?);
                state.selected_file = Some(request);
                state.main_view = MainView::Diff;
            }
            ScreenshotView::Preferences => {
                state.preferences_open = true;
                "General".clone_into(&mut state.preference_page);
            }
            ScreenshotView::Tabs => {
                state.overlay = Overlay::Tabs;
            }
        }
        Ok(state)
    }

    fn base(width: u32, height: u32, settings_store: SettingsStore, settings: Settings) -> Self {
        Self::base_with_proxy(width, height, settings_store, settings, None)
    }

    fn base_with_proxy(
        width: u32,
        height: u32,
        settings_store: SettingsStore,
        settings: Settings,
        event_loop_proxy: Option<EventLoopProxy<super::UserEvent>>,
    ) -> Self {
        if let Some(proxy) = &event_loop_proxy {
            crate::graph::avatars::set_event_loop_proxy(proxy.clone());
        }
        Self {
            width,
            height,
            mouse: [-10_000.0, -10_000.0],
            overlay_anchor: [0.0, 0.0],
            snapshot: None,
            graph: GraphLayout::default(),
            main_view: MainView::Graph,
            overlay: Overlay::None,
            preferences_open: false,
            preference_page: "General".to_owned(),
            focus: FocusField::None,
            preference_text_key: None,
            preference_text: TextField::default(),
            overlay_focus: FocusField::None,
            selected_commit: None,
            selected_commits: HashSet::new(),
            selection_anchor: None,
            detail: None,
            range_detail: None,
            detail_cache: HashMap::new(),
            detail_cache_order: VecDeque::new(),
            range_cache: HashMap::new(),
            pending_detail: None,
            modifier_shift: false,
            modifier_primary: false,
            selected_file: None,
            diff: None,
            palette: None,
            diff_search: TextField::default(),
            diff_search_cursor: 0,
            selected_working_files: HashSet::new(),
            collapsed_sections: HashSet::new(),
            commit_summary: TextField::default(),
            commit_body: TextField::default(),
            amend: false,
            search: TextField::default(),
            search_cursor: 0,
            branch_filter: TextField::default(),
            tab_filter: TextField::default(),
            welcome_search: TextField::default(),
            clone_url: TextField::default(),
            clone_destination: None,
            clone_form: false,
            branch_target: None,
            new_branch: TextField::default(),
            add_remote_provider: AddRemoteProvider::default(),
            add_remote_name: TextField::default(),
            add_remote_url: TextField::default(),
            add_remote_push_url: TextField::default(),
            add_remote_repo: TextField::default(),
            add_remote_host: TextField::default(),
            renamed_branch: TextField::default(),
            tag_name: TextField::default(),
            tag_message: TextField::default(),
            edit_summary: TextField::default(),
            edit_body: TextField::default(),
            path_tree: false,
            diff_text_selection: None,
            diff_text_drag: None,
            diff_last_click: None,
            last_ref_click: None,
            view_all_files: false,
            diff_split: true,
            diff_file_view: false,
            file_history: false,
            diff_selected_rows: HashSet::new(),
            diff_drag_start: None,
            push_after_commit: false,
            current_hunk: 0,
            sidebar_width: 260.0,
            detail_width: 690.0,
            ref_column_width: 440.0,
            graph_column_width: 410.0,
            message_column_width: 0.0,
            graph_column_explicit: false,
            diff_scroll_target: 0.0,
            diff_scroll_updated: None,
            graph_scroll: 0.0,
            sidebar_scroll: 0.0,
            sidebar_local_scroll: 0.0,
            sidebar_remote_scroll: 0.0,
            sidebar_worktrees_scroll: 0.0,
            sidebar_stashes_scroll: 0.0,
            sidebar_tags_scroll: 0.0,
            sidebar_section_fractions: [1.0; 5],
            sidebar_section_drag: None,
            detail_scroll: 0.0,
            wip_unstaged_scroll: 0.0,
            wip_staged_scroll: 0.0,
            diff_scroll: 0.0,
            preferences_scroll: 0.0,
            scrollbar_drag: None,
            busy_jobs: 0,
            inflight_ops: [0; TOOLBAR_OPS],
            loading_history: false,
            error: None,
            toast: None,
            ai_message: None,
            ai_loading: false,
            settings,
            drag: None,
            ref_drag: None,
            detail_message_height: 0.0,
            terminal_open: false,
            terminal_focused: false,
            terminal_height_fraction: 0.20,
            hits: Vec::new(),
            terminal: None,
            tabs: vec![RepoTab {
                title: "New Tab".to_owned(),
                path: None,
            }],
            active_tab: 0,

            repo_path: None,
            generation: 1,
            last_fetch: Instant::now(),
            started: Instant::now(),
            requested_limit: 500,
            settings_store,
            git: GitRunner::new(event_loop_proxy.clone()),
            ai: AiRunner::new(provider_from_environment(), event_loop_proxy.clone()),
            event_loop_proxy,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            pending_mutations: Vec::new(),
        }
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// True while a Git job dispatched by the given toolbar control is still running.
    pub(crate) fn op_in_flight(&self, op: ToolbarOp) -> bool {
        self.inflight_ops[op as usize] != 0
    }

    fn begin_op(&mut self, op: ToolbarOp) {
        self.inflight_ops[op as usize] = self.inflight_ops[op as usize].saturating_add(1);
    }

    fn end_op(&mut self, op: ToolbarOp) {
        self.inflight_ops[op as usize] = self.inflight_ops[op as usize].saturating_sub(1);
    }

    /// Seconds since the app started; drives time-based UI animations like spinners.
    pub(crate) fn animation_time(&self) -> f32 {
        self.started.elapsed().as_secs_f32()
    }

    /// Pointer position for base-layer hover effects; parked offscreen while
    /// an overlay is open so popups never leak hover highlights underneath.
    pub(crate) fn hover(&self) -> [f32; 2] {
        if self.overlay == Overlay::None {
            self.mouse
        } else {
            [-10_000.0, -10_000.0]
        }
    }

    /// The active ref drag once the pointer crossed the click threshold.
    pub(crate) fn dragging_ref(&self) -> Option<&RefDrag> {
        self.ref_drag.as_ref().filter(|drag| drag.active)
    }

    /// Updates the physical extent used by layout and splitter clamping.
    pub(crate) fn resize(&mut self, width: u32, height: u32) {
        self.width = width.max(1);
        self.height = height.max(1);
        self.sidebar_width = self.sidebar_width.min(px(width) * 0.45);
        self.detail_width = self.detail_width.min(px(width) * 0.55);
        self.resize_terminal();
    }

    pub(crate) fn terminal_snapshot(&self) -> Option<TerminalSnapshot> {
        self.terminal.as_ref().map(Terminal::snapshot)
    }

    pub(crate) fn terminal_accepts_input(&self) -> bool {
        self.terminal_open && self.terminal_focused
    }

    pub(crate) fn terminal_input(&self, bytes: &[u8]) {
        if self.terminal_accepts_input() {
            if let Some(terminal) = &self.terminal {
                terminal.write(bytes);
            }
        }
    }

    pub(crate) fn resize_terminal(&mut self) {
        let (cols, rows) = self.terminal_dimensions();
        if let Some(terminal) = &self.terminal {
            terminal.resize(cols, rows);
        }
    }

    fn terminal_dimensions(&self) -> (usize, usize) {
        let font_size = f32::from(self.settings.terminal_font_size.max(8));
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.2;
        let rect = crate::views::Layout::for_state(self)
            .terminal
            .unwrap_or_else(|| Rect::new(0.0, 0.0, cell_width, cell_height * 3.0));
        let width = (rect.width - 24.0).max(cell_width);
        let height = (rect.height - 24.0).max(cell_height * 3.0);
        (
            (width / cell_width).floor().max(1.0) as usize,
            (height / cell_height).floor().max(3.0) as usize,
        )
    }

    fn toggle_terminal(&mut self) {
        if self.terminal_open {
            self.terminal_open = false;
            self.terminal_focused = false;
            return;
        }
        if self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.snapshot().exited)
        {
            self.terminal = None;
        }
        self.terminal_open = true;
        if self.terminal.is_none() {
            let Some(path) = self.repo_path.as_deref() else {
                self.terminal_open = false;
                self.error = Some("No repository is open.".to_owned());
                return;
            };
            let (cols, rows) = self.terminal_dimensions();
            match Terminal::spawn(path, cols, rows, self.event_loop_proxy.clone()) {
                Ok(terminal) => self.terminal = Some(terminal),
                Err(error) => {
                    self.terminal_open = false;
                    self.error = Some(format!("Start terminal: {error:#}"));
                    return;
                }
            }
        } else {
            self.resize_terminal();
        }
        self.focus = FocusField::None;
        self.terminal_focused = true;
    }

    /// Replaces semantic hit regions after immediate layout.
    pub(crate) fn adopt_scene(&mut self, scene: &Scene) {
        self.hits.clone_from(&scene.hits);
    }

    /// Queues loading for the active repository tab and invalidates stale results.
    pub(crate) fn open_repository(&mut self, path: PathBuf) {
        let title = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repository")
            .to_owned();
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.title = title;
            tab.path = Some(path.clone());
        }
        self.generation = self.generation.wrapping_add(1);
        self.repo_path = Some(path);
        self.terminal = None;
        self.snapshot = None;
        self.detail = None;
        self.diff = None;
        self.select_only(None);
        self.detail_cache.clear();
        self.detail_cache_order.clear();
        self.range_cache.clear();
        self.pending_detail = None;
        self.graph = GraphLayout::default();
        self.busy_jobs = 0;
        self.inflight_ops = [0; TOOLBAR_OPS];
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.pending_mutations.clear();
        self.error = None;
        self.submit(GitJobKind::LoadSnapshot {
            limit: self.requested_limit,
        });
    }

    fn new_tab(&mut self) {
        self.tabs.push(RepoTab {
            title: "New Tab".to_owned(),
            path: None,
        });
        self.active_tab = self.tabs.len() - 1;
        self.show_welcome();
    }

    fn select_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index).cloned() else {
            return;
        };
        self.active_tab = index;
        self.close_overlay();
        if let Some(path) = tab.path {
            self.open_repository(path);
        } else {
            self.show_welcome();
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(RepoTab {
                title: "New Tab".to_owned(),
                path: None,
            });
            self.active_tab = 0;
            self.show_welcome();
            return;
        }
        self.active_tab = if index < self.tabs.len() {
            index
        } else {
            self.tabs.len() - 1
        };
        self.select_tab(self.active_tab);
    }

    fn show_welcome(&mut self) {
        self.repo_path = None;
        self.snapshot = None;
        self.detail = None;
        self.diff = None;
        self.select_only(None);
        self.selected_file = None;
        self.focus = FocusField::None;
    }

    /// Promotes `path` to the top of the recent-repository list; the settings
    /// file is only rewritten when the front entry actually changes.
    fn remember_repository(&mut self, path: &Path, name: &str) {
        if self
            .settings
            .recent_repos
            .first()
            .is_some_and(|recent| recent.path == path && recent.name == name)
        {
            return;
        }
        self.settings
            .recent_repos
            .retain(|recent| recent.path != path);
        self.settings.recent_repos.insert(
            0,
            RecentRepo {
                name: name.to_owned(),
                path: path.to_path_buf(),
                last_opened: chrono::Utc::now().timestamp(),
            },
        );
        self.settings.recent_repos.truncate(30);
        let _ = self.settings_store.save(&self.settings);
    }

    /// Applies all available worker completions and returns whether state changed.
    pub(crate) fn process_events(&mut self) -> bool {
        let events = self.git.drain().collect::<Vec<_>>();
        let mut changed = !events.is_empty();
        for event in events {
            self.apply_git_event(event);
        }
        if let Some(event) = self.ai.try_event() {
            changed = true;
            self.ai_loading = false;
            self.ai_message = Some(match event.result {
                Ok(message) => message,
                Err(error) => error,
            });
            self.show_popup(Overlay::Ai, FocusField::None);
        }
        let auto_fetch = self.schedule_auto_fetch();
        changed || auto_fetch
    }

    /// Returns the next eligible automatic-fetch deadline while the repository is idle.
    pub(crate) fn next_auto_fetch_deadline(&self) -> Option<Instant> {
        let minutes = self.settings.auto_fetch_minutes;
        if minutes == 0 || self.repo_path.is_none() || self.busy_jobs != 0 {
            return None;
        }
        Some(self.last_fetch + Duration::from_secs(u64::from(minutes) * 60))
    }

    fn schedule_auto_fetch(&mut self) -> bool {
        let Some(deadline) = self.next_auto_fetch_deadline() else {
            return false;
        };
        if Instant::now() < deadline {
            return false;
        }
        self.last_fetch = Instant::now();
        self.submit_mutation(GitJobKind::Fetch {
            prune: self.settings.auto_prune,
            limit: self.requested_limit,
        });
        true
    }

    fn apply_git_event(&mut self, event: GitEvent) {
        if event.generation != self.generation {
            return;
        }
        // Watcher-originated refreshes (no kind) were never counted as jobs.
        if event.kind.is_some() {
            self.busy_jobs = self.busy_jobs.saturating_sub(1);
        }
        if let Some(op) = event.kind.as_ref().and_then(toolbar_op) {
            self.end_op(op);
        }
        if let Some(GitJobKind::LoadDetail { id, .. }) = event.kind.as_ref()
            && self
                .pending_detail
                .as_ref()
                .is_some_and(|(pending, _)| pending == id)
        {
            self.pending_detail = None;
        }
        let mutation = event.kind.as_ref().is_some_and(is_mutation_job);
        let fetch = matches!(event.kind.as_ref(), Some(GitJobKind::Fetch { .. }));
        match event.result {
            Ok(GitPayload::Snapshot(snapshot)) => {
                self.loading_history = false;
                self.apply_snapshot(snapshot);
                self.refresh_selection_details();
            }
            Ok(GitPayload::History(mut snapshot)) => {
                self.loading_history = false;
                // History jobs skip the status scan; the current working tree
                // stays authoritative.
                if let Some(current) = &self.snapshot {
                    snapshot.working.clone_from(&current.working);
                }
                self.apply_snapshot(snapshot);
                self.refresh_selection_details();
            }
            Ok(GitPayload::WorkingStatus(working)) => self.apply_working_status(working),
            Ok(GitPayload::Cloned(path)) => {
                self.toast = Some("Repository cloned".to_owned());
                self.open_repository(path);
            }
            Ok(GitPayload::Detail(detail)) => {
                let detail = Arc::new(detail);
                self.cache_detail(&detail);
                if let Overlay::EditCommitMessage(id) = &self.overlay
                    && *id == detail.id
                    && self.edit_summary.is_empty()
                    && self.edit_body.is_empty()
                {
                    self.edit_summary.insert(&detail.subject);
                    self.edit_body.insert(&detail.body);
                }
                if self.selected_commits.len() <= 1
                    && self.selected_commit.as_deref() == Some(detail.id.as_str())
                {
                    self.detail = Some(detail);
                }
            }
            Ok(GitPayload::RangeDetail(range)) => {
                let range = Arc::new(range);
                self.range_cache.insert(
                    (range.oldest.clone(), range.newest.clone()),
                    Arc::clone(&range),
                );
                if self
                    .selection_endpoints()
                    .is_some_and(|(oldest, newest)| oldest == range.oldest && newest == range.newest)
                {
                    self.range_detail = Some(range);
                }
            }
            Ok(GitPayload::Diff(diff)) => {
                if self
                    .selected_file
                    .as_ref()
                    .is_some_and(|request| request.path == diff.path && request.scope == diff.scope)
                {
                    if self.diff.is_none() && !self.diff_file_view {
                        self.current_hunk = 0;
                        if let Some(&first) = diff.hunks.first() {
                            self.diff_scroll = seek_scroll(first);
                            self.diff_scroll_target = self.diff_scroll;
                            self.diff_scroll_updated = None;
                        }
                    }
                    self.diff = Some(diff);
                }
            }
            Ok(GitPayload::Mutated {
                snapshot,
                commit_id,
                message,
            }) => {
                let pending = self
                    .pending_mutations
                    .drain(..1)
                    .next()
                    .unwrap_or(PendingMutation::Ignore);
                self.loading_history = false;
                self.apply_snapshot(snapshot);
                self.selected_working_files.clear();
                self.diff = None;
                if let Some(id) = commit_id {
                    if self.settings.notify_operation_success {
                        self.toast =
                            if matches!(event.kind.as_ref(), Some(GitJobKind::Reword { .. })) {
                                message.clone()
                            } else {
                                Some(format!("Created commit {}", &id[..7.min(id.len())]))
                            };
                    }
                    self.commit_summary.clear();
                    self.commit_body.clear();
                    self.amend = false;
                    if self.push_after_commit {
                        self.push_after_commit = false;
                        self.submit_mutation(GitJobKind::Push {
                            limit: self.requested_limit,
                        });
                    }
                    self.select_only(Some(id.clone()));
                    self.main_view = MainView::Graph;
                    self.request_detail(id);
                } else {
                    if let Some(message) = message
                        && self.settings.notify_operation_success
                        && (!fetch || self.settings.notify_fetch_results)
                    {
                        self.toast = Some(message);
                    }
                    if let Some(request) = self.selected_file.clone() {
                        self.submit(GitJobKind::LoadDiff { request });
                    }
                }
                self.complete_pending_mutation(pending);
                self.push_after_commit = false;
            }
            Err(error) => {
                if mutation {
                    let pending = self
                        .pending_mutations
                        .drain(..1)
                        .next()
                        .unwrap_or(PendingMutation::Ignore);
                    self.restore_failed_history(pending);
                }
                self.loading_history = false;
                self.push_after_commit = false;
                if self.settings.notify_operation_failure {
                    self.error = Some(error);
                }
            }
        }
    }

    fn apply_snapshot(&mut self, snapshot: RepoSnapshot) {
        self.repo_path = Some(snapshot.path.clone());
        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
            tab.title.clone_from(&snapshot.name);
            tab.path = Some(snapshot.path.clone());
        }
        self.remember_repository(&snapshot.path, &snapshot.name);
        self.requested_limit = snapshot.loaded_limit.max(self.requested_limit);
        // Drop selected ids that vanished from history in one pass, keeping
        // the lead on the topmost surviving row.
        if !self.selected_commits.is_empty() {
            let mut retained = HashSet::with_capacity(self.selected_commits.len());
            let mut topmost = None;
            for commit in &snapshot.commits {
                if self.selected_commits.contains(&commit.id) {
                    if topmost.is_none() {
                        topmost = Some(commit.id.clone());
                    }
                    retained.insert(commit.id.clone());
                    if retained.len() == self.selected_commits.len() {
                        break;
                    }
                }
            }
            self.selected_commits = retained;
            if self
                .selected_commit
                .as_ref()
                .is_some_and(|lead| !self.selected_commits.contains(lead))
            {
                self.selected_commit = topmost;
            }
            if self
                .selection_anchor
                .as_ref()
                .is_some_and(|anchor| !self.selected_commits.contains(anchor))
            {
                self.selection_anchor.clone_from(&self.selected_commit);
            }
        }
        if self.selected_commits.is_empty() {
            let fallback = snapshot
                .head_id
                .clone()
                .or_else(|| snapshot.commits.first().map(|commit| commit.id.clone()));
            self.select_only(fallback);
        }
        self.retarget_working_diff(&snapshot.working);
        // An identical history (ids, refs, lanes inputs) keeps the existing
        // layout; watcher refreshes frequently change only the working tree.
        let commits_unchanged = self
            .snapshot
            .as_ref()
            .is_some_and(|old| old.commits == snapshot.commits);
        if !commits_unchanged {
            self.graph = GraphLayout::build(&snapshot.commits);
        }
        self.snapshot = Some(snapshot);
    }

    /// Applies a watcher-detected working-tree change without touching the
    /// graph, detail panel, or selection.
    fn apply_working_status(&mut self, working: WorkingTree) {
        self.retarget_working_diff(&working);
        let Some(snapshot) = self.snapshot.as_mut() else {
            return;
        };
        if snapshot.working == working {
            return;
        }
        snapshot.working = working;
        // The file content behind an open working diff likely changed on disk.
        if let Some(request) = self.selected_file.clone()
            && matches!(request.scope, DiffScope::Staged | DiffScope::Unstaged)
        {
            self.submit(GitJobKind::LoadDiff { request });
        }
    }

    /// Keeps an open working-file diff aligned with a fresh working tree.
    fn retarget_working_diff(&mut self, working: &WorkingTree) {
        let Some(request) = self.selected_file.as_mut() else {
            return;
        };
        let (was_staged, was_unstaged) = match request.scope {
            DiffScope::Staged => (true, false),
            DiffScope::Unstaged => (false, true),
            DiffScope::Commit(_) | DiffScope::CommitRange { .. } => return,
        };
        let file = working
            .files
            .iter()
            .find(|file| file.path == request.path);
        let remains_in_scope = file.is_some_and(|file| {
            (was_staged && file.staged.is_some()) || (was_unstaged && file.unstaged.is_some())
        });
        if remains_in_scope {
            return;
        }
        let new_scope = file.and_then(|file| {
            if was_staged && file.unstaged.is_some() {
                Some(DiffScope::Unstaged)
            } else if was_unstaged && file.staged.is_some() {
                Some(DiffScope::Staged)
            } else {
                None
            }
        });
        if let Some(scope) = new_scope {
            request.scope = scope;
        } else {
            self.selected_file = None;
            self.diff = None;
            self.main_view = MainView::Wip;
        }
    }

    /// Selects a graph row honoring the live keyboard modifiers: shift extends
    /// from the anchor across every row in between, the primary modifier
    /// (Cmd/Ctrl) toggles individual commits, and a plain click replaces the
    /// selection — matching `GitKraken`'s multi-select.
    fn select_commit(&mut self, id: String) {
        if self.modifier_shift {
            self.extend_selection_to(&id);
            self.selected_commit = Some(id);
        } else if self.modifier_primary {
            self.toggle_selection(id);
        } else {
            self.select_only(Some(id));
        }
        self.main_view = MainView::Graph;
        self.selected_file = None;
        self.diff = None;
        self.refresh_selection_details();
    }

    /// Replaces the whole selection with zero or one commit.
    fn select_only(&mut self, id: Option<String>) {
        self.selected_commits.clear();
        if let Some(id) = &id {
            self.selected_commits.insert(id.clone());
        }
        self.selection_anchor.clone_from(&id);
        self.selected_commit = id;
        self.range_detail = None;
    }

    /// Selects every row between the anchor and `id`, inclusive.
    fn extend_selection_to(&mut self, id: &str) {
        let anchor = self
            .selection_anchor
            .clone()
            .or_else(|| self.selected_commit.clone())
            .unwrap_or_else(|| id.to_owned());
        let rows = self.snapshot.as_ref().and_then(|snapshot| {
            let anchor_row = snapshot
                .commits
                .iter()
                .position(|commit| commit.id == anchor)?;
            let target_row = snapshot.commits.iter().position(|commit| commit.id == id)?;
            Some((anchor_row.min(target_row), anchor_row.max(target_row)))
        });
        let Some((first, last)) = rows else {
            self.select_only(Some(id.to_owned()));
            return;
        };
        self.selected_commits.clear();
        if let Some(snapshot) = &self.snapshot {
            for commit in &snapshot.commits[first..=last] {
                self.selected_commits.insert(commit.id.clone());
            }
        }
        self.selection_anchor = Some(anchor);
        self.range_detail = None;
    }

    /// Adds or removes one commit from the selection; the selection never
    /// empties, and a removed lead passes to the topmost remaining row.
    fn toggle_selection(&mut self, id: String) {
        if self.selected_commits.contains(&id) {
            if self.selected_commits.len() <= 1 {
                return;
            }
            self.selected_commits.remove(&id);
            if self.selection_anchor.as_deref() == Some(id.as_str()) {
                self.selection_anchor = None;
            }
            if self.selected_commit.as_deref() == Some(id.as_str()) {
                let topmost = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .commits
                        .iter()
                        .find(|commit| self.selected_commits.contains(&commit.id))
                        .map(|commit| commit.id.clone())
                });
                self.selected_commit = topmost.or_else(|| {
                    self.selected_commits.iter().next().cloned()
                });
            }
        } else {
            self.selected_commits.insert(id.clone());
            self.selected_commit = Some(id.clone());
            self.selection_anchor = Some(id);
        }
        self.range_detail = None;
    }

    /// Whether a graph row participates in the current selection.
    pub(crate) fn is_commit_selected(&self, id: &str) -> bool {
        self.selected_commits.contains(id) || self.selected_commit.as_deref() == Some(id)
    }

    /// Whether the commit is unpushed, from the loaded graph summary.
    pub(crate) fn commit_is_local(&self, id: &str) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.commits.iter().find(|commit| commit.id == id))
            .is_some_and(|commit| commit.is_local)
    }

    /// Oldest/newest ids of the multi-selection in graph row order (row 0 is
    /// newest); `None` while fewer than two commits are selected.
    pub(crate) fn selection_endpoints(&self) -> Option<(String, String)> {
        if self.selected_commits.len() < 2 {
            return None;
        }
        let snapshot = self.snapshot.as_ref()?;
        let mut newest = None;
        let mut oldest = None;
        for commit in &snapshot.commits {
            if self.selected_commits.contains(&commit.id) {
                if newest.is_none() {
                    newest = Some(commit.id.clone());
                }
                oldest = Some(commit.id.clone());
            }
        }
        Some((oldest?, newest?))
    }

    /// Loads the panel data matching the current selection, from cache when
    /// the commit or range was already fetched.
    fn refresh_selection_details(&mut self) {
        if self.selected_commits.len() > 1 {
            self.detail = None;
            self.request_range_detail();
        } else if let Some(id) = self.selected_commit.clone() {
            self.range_detail = None;
            self.request_detail(id);
        }
    }

    /// Ensures `detail` holds `id`, hitting the immutable per-commit cache
    /// before submitting a `LoadDetail` job; in-flight requests are not repeated.
    fn request_detail(&mut self, id: String) {
        let want_tree = self.view_all_files;
        if self
            .detail
            .as_ref()
            .is_some_and(|detail| detail.id == id && (!want_tree || detail.all_files.is_some()))
        {
            return;
        }
        if let Some(cached) = self.detail_cache.get(&id)
            && (!want_tree || cached.all_files.is_some())
        {
            self.detail = Some(Arc::clone(cached));
            return;
        }
        self.detail = None;
        if self
            .pending_detail
            .as_ref()
            .is_some_and(|(pending, tree)| *pending == id && (*tree || !want_tree))
        {
            return;
        }
        self.pending_detail = Some((id.clone(), want_tree));
        self.submit(GitJobKind::LoadDetail {
            id,
            include_tree: want_tree,
        });
    }

    /// Ensures `range_detail` matches the multi-selection endpoints; ranges
    /// are immutable, so cached ones never re-fetch.
    fn request_range_detail(&mut self) {
        let Some((oldest, newest)) = self.selection_endpoints() else {
            self.range_detail = None;
            return;
        };
        if self
            .range_detail
            .as_ref()
            .is_some_and(|range| range.oldest == oldest && range.newest == newest)
        {
            return;
        }
        if let Some(cached) = self.range_cache.get(&(oldest.clone(), newest.clone())) {
            self.range_detail = Some(Arc::clone(cached));
            return;
        }
        self.range_detail = None;
        self.submit(GitJobKind::LoadRangeDetail { oldest, newest });
    }

    /// Bounded FIFO insert into the per-repository commit-detail cache.
    fn cache_detail(&mut self, detail: &Arc<CommitDetail>) {
        const DETAIL_CACHE_LIMIT: usize = 512;
        if self
            .detail_cache
            .insert(detail.id.clone(), Arc::clone(detail))
            .is_none()
        {
            self.detail_cache_order.push_back(detail.id.clone());
            if self.detail_cache_order.len() > DETAIL_CACHE_LIMIT
                && let Some(evicted) = self.detail_cache_order.pop_front()
            {
                self.detail_cache.remove(&evicted);
            }
        }
    }

    /// Dispatches a semantic action shared by pointer input and core-loop automation.
    pub(crate) fn dispatch(&mut self, action: UiAction) {
        self.toast = None;
        match action {
            UiAction::SelectCommit(id) => self.select_commit(id),
            UiAction::JumpToCommit(id) => {
                if let Some(index) = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.commits.iter().position(|commit| commit.id == id))
                {
                    let viewport =
                        (px(self.height) - CONTENT_TOP - STATUS_BAR_HEIGHT - COMMIT_HEADER_HEIGHT)
                            .max(0.0);
                    self.graph_scroll =
                        (index.to_f32().unwrap_or(0.0) * COMMIT_ROW_HEIGHT - viewport * 0.5).clamp(
                            0.0,
                            self.graph.max_scroll(
                                viewport,
                                COMMIT_ROW_HEIGHT,
                                self.snapshot.as_ref().map_or(0, RepoSnapshot::wip_rows),
                            ),
                        );
                } else if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.has_more)
                    && !self.loading_history
                {
                    self.loading_history = true;
                    self.requested_limit = self.requested_limit.saturating_mul(2).min(100_000);
                    self.submit(GitJobKind::LoadHistory {
                        limit: self.requested_limit,
                    });
                    self.toast = Some("Loading older commits…".to_owned());
                } else {
                    self.toast = Some("Commit is not in the loaded graph".to_owned());
                }
                self.select_only(Some(id.clone()));
                self.main_view = MainView::Graph;
                self.selected_file = None;
                self.diff = None;
                self.request_detail(id);
            }
            UiAction::SelectWip => {
                self.select_only(None);
                self.detail = None;
                self.main_view = MainView::Wip;
                self.selected_file = None;
                self.diff = None;
            }
            UiAction::SelectFile { path, scope } => {
                let request = DiffRequest { path, scope };
                self.selected_file = Some(request.clone());
                self.diff = None;
                self.main_view = MainView::Diff;
                self.current_hunk = 0;
                self.diff_scroll = 0.0;
                self.diff_scroll_target = 0.0;
                self.diff_scroll_updated = None;
                self.submit(GitJobKind::LoadDiff { request });
            }
            UiAction::ToggleSection(section) => {
                if !self.collapsed_sections.remove(&section) {
                    self.collapsed_sections.insert(section);
                }
            }
            UiAction::ToggleSidebarCollapse => {
                self.settings.sidebar_collapsed = !self.settings.sidebar_collapsed;
                self.persist_settings();
            }
            UiAction::ExpandSidebarSection(section) => {
                self.settings.sidebar_collapsed = false;
                self.persist_settings();
                self.collapsed_sections.remove(&section);
                match section.as_str() {
                    "LOCAL" => self.sidebar_local_scroll = 0.0,
                    "REMOTE" => self.sidebar_remote_scroll = 0.0,
                    "WORKTREES" => self.sidebar_worktrees_scroll = 0.0,
                    "STASHES" => self.sidebar_stashes_scroll = 0.0,
                    "TAGS" => self.sidebar_tags_scroll = 0.0,
                    _ => {}
                }
            }
            UiAction::StageFile(path) => self.submit_mutation(GitJobKind::Stage {
                paths: vec![path],
                limit: self.requested_limit,
            }),
            UiAction::UnstageFile(path) => self.submit_mutation(GitJobKind::Unstage {
                paths: vec![path],
                limit: self.requested_limit,
            }),
            UiAction::StageDiffLines { path, lines } => {
                self.close_overlay();
                self.diff_selected_rows.clear();
                self.submit_mutation(GitJobKind::StageLines {
                    path,
                    lines,
                    limit: self.requested_limit,
                });
            }
            UiAction::UnstageDiffLines { path, lines } => {
                self.diff_selected_rows.clear();
                self.close_overlay();
                self.submit_mutation(GitJobKind::UnstageLines {
                    path,
                    lines,
                    limit: self.requested_limit,
                });
            }
            UiAction::DiscardDiffLines { path, lines } => {
                self.close_overlay();
                self.diff_selected_rows.clear();
                self.submit_mutation(GitJobKind::DiscardLines {
                    path,
                    lines,
                    limit: self.requested_limit,
                });
            }
            UiAction::CopyDiffLines(lines) => {
                self.close_overlay();
                self.diff_selected_rows.clear();
                match arboard::Clipboard::new() {
                    Ok(mut clipboard) => {
                        if let Err(error) = clipboard.set_text(lines.join("\n")) {
                            self.error = Some(format!("copy selected lines: {error}"));
                        }
                    }
                    Err(error) => self.error = Some(format!("open clipboard: {error}")),
                }
            }
            UiAction::CopyDiffText => self.copy_diff_text(),
            UiAction::BeginDiffSelection(row) => {
                self.diff_selected_rows.clear();
                self.diff_selected_rows.insert(row);
                self.diff_drag_start = Some(row);
            }
            UiAction::BeginDiffTextSelection {
                row,
                side,
                column,
                clicks,
            } => self.begin_diff_text_selection(row, side, column, clicks),
            UiAction::ToggleCommitOptions => {
                self.overlay_anchor = self.mouse;
                self.toggle_popup(Overlay::CommitOptions, FocusField::None);
            }
            UiAction::CommitAndPush => {
                self.close_overlay();
                self.push_after_commit = true;
                self.dispatch(UiAction::Commit);
            }
            UiAction::ToggleFileSelection(path) => {
                if !self.selected_working_files.remove(&path) {
                    self.selected_working_files.insert(path);
                }
            }
            UiAction::StageSelection => {
                let paths = self.selected_working_files.iter().cloned().collect();
                self.submit_mutation(GitJobKind::Stage {
                    paths,
                    limit: self.requested_limit,
                });
            }
            UiAction::StageAll => self.submit_mutation(GitJobKind::StageAll {
                limit: self.requested_limit,
            }),
            UiAction::UnstageAll => self.submit_mutation(GitJobKind::UnstageAll {
                limit: self.requested_limit,
            }),
            UiAction::Commit => {
                let input = CommitInput {
                    summary: self.commit_summary.text().to_owned(),
                    body: self.commit_body.text().to_owned(),
                    amend: self.amend,
                };
                self.submit_mutation(GitJobKind::Commit {
                    input,
                    limit: self.requested_limit,
                });
            }
            UiAction::ToggleAmend => self.amend = !self.amend,
            UiAction::FocusCommitSummary => self.focus = FocusField::CommitSummary,
            UiAction::FocusCommitBody => self.focus = FocusField::CommitBody,
            UiAction::ToggleBranchMenu => {
                self.toggle_popup(Overlay::Branches, FocusField::BranchFilter)
            }
            UiAction::FocusBranchFilter => self.focus = FocusField::BranchFilter,
            UiAction::FocusTabFilter => self.focus = FocusField::TabFilter,
            UiAction::CheckoutBranch(branch) => {
                self.close_overlay();
                self.last_ref_click = None;
                self.submit_mutation(GitJobKind::Checkout {
                    branch,
                    limit: self.requested_limit,
                });
            }
            UiAction::BranchClick(name) => {
                if self.take_ref_double_click("branch", &name) {
                    self.dispatch(UiAction::CheckoutBranch(name));
                } else if let Some(tip) = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .branches
                        .iter()
                        .find(|branch| branch.name == name)
                        .map(|branch| branch.target.clone())
                }) {
                    self.dispatch(UiAction::JumpToCommit(tip));
                }
            }
            UiAction::TagClick(name) => {
                if self.take_ref_double_click("tag", &name) {
                    self.dispatch(UiAction::CheckoutBranch(name));
                } else if let Some(tagged) = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .commits
                        .iter()
                        .find(|commit| {
                            commit.refs.iter().any(|label| {
                                label.kind == crate::git::models::RefKind::Tag && label.name == name
                            })
                        })
                        .map(|commit| commit.id.clone())
                }) {
                    self.dispatch(UiAction::JumpToCommit(tagged));
                }
            }
            UiAction::OpenBranchContext(branch) => {
                self.overlay_anchor = self.mouse;
                self.show_popup(Overlay::BranchContext(branch), FocusField::None);
            }
            UiAction::BranchContextCheckout => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.dispatch(UiAction::CheckoutBranch(branch));
                }
            }
            UiAction::BranchContextFastForward => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::FastForward {
                        branch,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::BranchContextMerge => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::Merge {
                        branch,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::BranchContextRebase => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::Rebase {
                        branch,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::OpenCommitContext(id) => {
                self.overlay_anchor = self.mouse;
                self.show_popup(Overlay::CommitContext(id), FocusField::None);
            }
            UiAction::CommitContextCheckout => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.dispatch(UiAction::CheckoutBranch(id));
                }
            }
            UiAction::CommitContextCreateBranch => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.branch_target = Some(id);
                    self.close_overlay();
                    self.show_popup(Overlay::CreateBranch, FocusField::CreateBranch);
                }
            }
            UiAction::CommitContextCreateTag => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    self.show_popup(Overlay::CreateTag(id), FocusField::CreateTagName);
                }
            }
            UiAction::CommitContextCherryPick => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::CherryPick {
                        id,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::CommitContextRevert => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::Revert {
                        id,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::CommitContextReset(mode) => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::Reset {
                        target: id,
                        mode,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::CopyCommitSha => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    self.copy_text(id);
                }
            }
            UiAction::CopyCommitMessage => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    let message = self
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.commits.iter().find(|commit| commit.id == id))
                        .map(|commit| commit.subject.clone());
                    if let Some(message) = message {
                        self.close_overlay();
                        self.copy_text(message);
                    }
                }
            }
            UiAction::BranchContextCopyName => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.close_overlay();
                    self.copy_text(branch);
                }
            }
            UiAction::TagContextCopyName => {
                if let Overlay::TagContext(tag) = self.overlay.clone() {
                    self.close_overlay();
                    self.copy_text(tag);
                }
            }
            UiAction::DropMerge => {
                if let Overlay::DropMenu { source, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::Merge {
                        branch: source,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::DropRebaseOnto => {
                if let Overlay::DropMenu { source, target, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::RebaseOnto {
                        source,
                        target,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::DropFastForward => {
                if let Overlay::DropMenu { source, target, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::FastForwardTo {
                        source,
                        target,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::CommitContextEditMessage => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    self.edit_summary = TextField::default();
                    self.edit_body = TextField::default();
                    if let Some(detail) = self.detail.as_ref().filter(|detail| detail.id == id) {
                        self.edit_summary.insert(&detail.subject);
                        self.edit_body.insert(&detail.body);
                    } else {
                        self.submit(GitJobKind::LoadDetail {
                            id: id.clone(),
                            include_tree: false,
                        });
                    }
                    self.show_popup(
                        Overlay::EditCommitMessage(id),
                        FocusField::EditMessageSummary,
                    );
                }
            }
            UiAction::ConfirmEditMessage => {
                if matches!(self.overlay, Overlay::EditCommitMessage(_))
                    && !self.edit_summary.trim().is_empty()
                {
                    let summary = self.edit_summary.trim().to_owned();
                    let body = self.edit_body.trim().to_owned();
                    self.edit_summary = TextField::default();
                    self.edit_body = TextField::default();
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::Reword {
                        summary,
                        body,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::FocusEditMessageSummary => self.focus = FocusField::EditMessageSummary,
            UiAction::FocusEditMessageBody => self.focus = FocusField::EditMessageBody,
            UiAction::CommitContextCreatePatch => {
                if let Overlay::CommitContext(id) = self.overlay.clone() {
                    self.close_overlay();
                    let short: String = id.chars().take(7).collect();
                    if let Some(destination) = rfd::FileDialog::new()
                        .set_file_name(format!("{short}.patch"))
                        .save_file()
                    {
                        self.submit_mutation(GitJobKind::SavePatch {
                            id,
                            destination,
                            limit: self.requested_limit,
                        });
                    }
                }
            }
            UiAction::BranchContextDelete => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::DeleteBranch {
                        branch,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::BranchContextRename => {
                if let Overlay::BranchContext(branch) = self.overlay.clone() {
                    self.renamed_branch.clear();
                    self.close_overlay();
                    self.show_popup(Overlay::RenameBranch(branch), FocusField::RenameBranch);
                }
            }
            UiAction::FocusRenameBranch => self.focus = FocusField::RenameBranch,
            UiAction::RenameBranch => {
                if let Overlay::RenameBranch(branch) = self.overlay.clone()
                    && !self.renamed_branch.trim().is_empty()
                {
                    let new_name = self.renamed_branch.trim().to_owned();
                    self.renamed_branch.clear();
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::RenameBranch {
                        branch,
                        new_name,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::FocusCreateTagName => self.focus = FocusField::CreateTagName,
            UiAction::FocusCreateTagMessage => self.focus = FocusField::CreateTagMessage,
            UiAction::CreateTag => {
                if let Overlay::CreateTag(target) = self.overlay.clone()
                    && !self.tag_name.trim().is_empty()
                {
                    let name = self.tag_name.trim().to_owned();
                    let message = (!self.tag_message.trim().is_empty())
                        .then(|| self.tag_message.trim().to_owned());
                    self.tag_name.clear();
                    self.tag_message.clear();
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::CreateTag {
                        name,
                        target,
                        message,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::OpenFileContext { path, scope } => {
                self.overlay_anchor = self.mouse;
                self.show_popup(Overlay::FileContext { path, scope }, FocusField::None);
            }
            UiAction::FileContextStage => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.dispatch(UiAction::StageFile(path));
                }
            }
            UiAction::FileContextUnstage => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.dispatch(UiAction::UnstageFile(path));
                }
            }
            UiAction::FileContextDiscard => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::DiscardFile {
                        path,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::FileContextStashFile => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::StashFile {
                        path,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::FileContextIgnore(pattern) => {
                self.close_overlay();
                self.submit_mutation(GitJobKind::IgnorePattern {
                    pattern,
                    limit: self.requested_limit,
                });
            }
            UiAction::FileContextHistory => {
                if let Overlay::FileContext { path, scope } = self.overlay.clone() {
                    self.close_overlay();
                    let scope = match scope {
                        FileContextScope::Staged => DiffScope::Staged,
                        FileContextScope::Unstaged => DiffScope::Unstaged,
                        FileContextScope::Committed(id) => DiffScope::Commit(id),
                    };
                    self.dispatch(UiAction::SelectFile { path, scope });
                    self.file_history = true;
                }
            }
            UiAction::FileContextOpenEditor => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.open_file_in_editor(&path);
                }
            }
            UiAction::FileContextOpenDefault => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.open_path_externally(&path, false);
                }
            }
            UiAction::FileContextReveal => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.open_path_externally(&path, true);
                }
            }
            UiAction::FileContextCopyPath => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    self.copy_text(self.workdir_path(&path).display().to_string());
                }
            }
            UiAction::FileContextDelete => {
                if let Overlay::FileContext { path, .. } = self.overlay.clone() {
                    self.close_overlay();
                    if let Err(error) = std::fs::remove_file(self.workdir_path(&path)) {
                        self.error = Some(format!("Delete {}: {error}", path.display()));
                    } else {
                        self.toast = Some(format!("Deleted {}", path.display()));
                        self.submit(GitJobKind::LoadSnapshot {
                            limit: self.requested_limit,
                        });
                    }
                }
            }
            UiAction::OpenStashContext(index) => {
                self.overlay_anchor = self.mouse;
                self.show_popup(Overlay::StashContext(index), FocusField::None);
            }
            UiAction::StashContextApply => {
                if let Overlay::StashContext(index) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::ApplyStash {
                        index,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::StashContextPop => {
                if let Overlay::StashContext(index) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::ApplyStash {
                        index,
                        limit: self.requested_limit,
                    });
                    self.submit_mutation(GitJobKind::DropStash {
                        index,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::StashContextDrop => {
                if let Overlay::StashContext(index) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::DropStash {
                        index,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::OpenTagContext(tag) => {
                self.overlay_anchor = self.mouse;
                self.show_popup(Overlay::TagContext(tag), FocusField::None);
            }
            UiAction::TagContextCheckout => {
                if let Overlay::TagContext(tag) = self.overlay.clone() {
                    self.dispatch(UiAction::CheckoutBranch(tag));
                }
            }
            UiAction::TagContextDelete => {
                if let Overlay::TagContext(tag) = self.overlay.clone() {
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::DeleteTag {
                        tag,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::Undo => {
                if let Some(record) = self.undo_stack.pop() {
                    let job = self.history_job(&record, true);
                    self.submit_history_mutation(job, record, true);
                } else {
                    self.toast = Some("Nothing to undo".to_owned());
                }
            }
            UiAction::Redo => {
                if let Some(record) = self.redo_stack.pop() {
                    let job = self.history_job(&record, false);
                    self.submit_history_mutation(job, record, false);
                } else {
                    self.toast = Some("Nothing to redo".to_owned());
                }
            }
            UiAction::ToggleCreateBranch => {
                self.toggle_popup(Overlay::CreateBranch, FocusField::CreateBranch);
            }
            UiAction::FocusCreateBranch => self.focus = FocusField::CreateBranch,
            UiAction::CreateBranch => {
                if !self.new_branch.trim().is_empty() {
                    let branch = self.new_branch.trim().to_owned();
                    self.new_branch.clear();
                    let target = self.branch_target.take();
                    self.submit_mutation(GitJobKind::CreateBranch {
                        branch,
                        target,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::OpenAddRemote => {
                self.show_popup(
                    Overlay::AddRemote,
                    add_remote_first_field(self.add_remote_provider),
                );
            }
            UiAction::SelectAddRemoteProvider(provider) => {
                self.add_remote_provider = provider;
                self.focus = add_remote_first_field(provider);
            }
            UiAction::FocusAddRemoteName => self.focus = FocusField::AddRemoteName,
            UiAction::FocusAddRemoteUrl => self.focus = FocusField::AddRemoteUrl,
            UiAction::FocusAddRemotePushUrl => self.focus = FocusField::AddRemotePushUrl,
            UiAction::FocusAddRemoteRepo => self.focus = FocusField::AddRemoteRepo,
            UiAction::FocusAddRemoteHost => self.focus = FocusField::AddRemoteHost,
            UiAction::AddRemote => {
                if let Some(remote) = self.add_remote_submission() {
                    self.add_remote_name.clear();
                    self.add_remote_url.clear();
                    self.add_remote_push_url.clear();
                    self.add_remote_repo.clear();
                    self.add_remote_host.clear();
                    self.close_overlay();
                    self.submit_mutation(GitJobKind::AddRemote {
                        name: remote.name,
                        url: remote.url,
                        push_url: remote.push_url,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::Fetch => {
                self.close_overlay();
                self.submit_mutation(GitJobKind::Fetch {
                    prune: self.settings.auto_prune,
                    limit: self.requested_limit,
                });
            }
            UiAction::Pull => {
                if !self.op_in_flight(ToolbarOp::Pull) {
                    self.submit_mutation(GitJobKind::Pull {
                        operation: self.settings.default_pull_operation,
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::TogglePullOptions => {
                self.overlay_anchor = self.mouse;
                self.toggle_popup(Overlay::PullOptions, FocusField::None);
            }
            UiAction::SetPullOperation(operation) => {
                self.settings.default_pull_operation = operation;
                self.persist_settings();
                self.close_overlay();
                self.submit_mutation(GitJobKind::Pull {
                    operation,
                    limit: self.requested_limit,
                });
            }
            UiAction::Push => {
                if !self.op_in_flight(ToolbarOp::Push) {
                    self.submit_mutation(GitJobKind::Push {
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::Stash => {
                self.close_overlay();
                if !self.op_in_flight(ToolbarOp::Stash) {
                    self.submit_mutation(GitJobKind::Stash {
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::PopStash => {
                if !self.op_in_flight(ToolbarOp::Pop) {
                    self.submit_mutation(GitJobKind::PopStash {
                        limit: self.requested_limit,
                    });
                }
            }
            UiAction::OpenTerminal => self.toggle_terminal(),
            UiAction::FocusTerminal => {
                self.focus = FocusField::None;
                self.terminal_focused = true;
            }
            UiAction::ToggleLfsMenu => self.toggle_popup(Overlay::Lfs, FocusField::None),
            UiAction::LfsCheckout => self.submit_lfs(LfsOperation::Checkout),
            UiAction::LfsPull => self.submit_lfs(LfsOperation::Pull),
            UiAction::LfsPush => self.submit_lfs(LfsOperation::Push),
            UiAction::LfsPrune => self.submit_lfs(LfsOperation::Prune),
            UiAction::ToggleActionsMenu => self.toggle_popup(Overlay::Actions, FocusField::None),
            UiAction::ToggleCommandPalette => self.toggle_palette(Overlay::CommandPalette),
            UiAction::ToggleEditorPalette => {
                if self.main_view == MainView::Diff {
                    self.toggle_palette(Overlay::EditorPalette);
                }
            }
            UiAction::FocusPalette => self.focus = FocusField::Palette,
            UiAction::PalettePrevious => self.move_palette(-1),
            UiAction::PaletteNext => self.move_palette(1),
            UiAction::ExecutePaletteCommand(index) => self.execute_palette_command(index),
            UiAction::ToggleSearch | UiAction::FocusSearch => {
                self.close_overlay();
                self.focus = FocusField::Search;
            }
            UiAction::ToggleDiffSearch => {
                if self.main_view == MainView::Diff {
                    self.close_overlay();
                    self.focus = FocusField::DiffSearch;
                }
            }
            UiAction::CloseDiffSearch => {
                self.diff_search.clear();
                self.diff_search_cursor = 0;
                self.focus = FocusField::None;
            }
            UiAction::PreviousDiffSearch => self.move_diff_search(-1),
            UiAction::NextDiffSearch => self.move_diff_search(1),
            UiAction::PreviousSearchResult => self.move_search(-1),
            UiAction::NextSearchResult => self.move_search(1),
            UiAction::CloseSearch => {
                self.search.clear();
                self.search_cursor = 0;
                self.focus = FocusField::None;
            }
            UiAction::ToggleTabSwitcher => self.toggle_popup(Overlay::Tabs, FocusField::TabFilter),
            UiAction::NewTab => self.new_tab(),
            UiAction::SelectTab(index) => self.select_tab(index),
            UiAction::CloseTab(index) => self.close_tab(index),
            UiAction::OpenRepository(path) => self.open_repository(path),
            UiAction::OpenRepositoryPicker => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    if GitBackend::discover(&path).is_ok() {
                        self.open_repository(path);
                    } else {
                        self.toast = Some("Selected folder is not a Git repository".to_owned());
                    }
                }
            }
            UiAction::CreateRepositoryPicker => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    match git2::Repository::init(&path).and_then(|repository| {
                        let branch = self.settings.default_branch_name.trim();
                        if branch.is_empty() {
                            return Err(git2::Error::from_str(
                                "default branch name cannot be empty",
                            ));
                        }
                        repository.set_head(&format!("refs/heads/{branch}"))?;
                        Ok(repository)
                    }) {
                        Ok(_) => self.open_repository(path),
                        Err(error) => self.toast = Some(format!("Create repository: {error}")),
                    }
                }
            }
            UiAction::ToggleCloneForm => {
                self.clone_form = !self.clone_form;
                if self.clone_form {
                    self.clone_destination = None;
                    self.clone_url.clear();
                    self.focus = FocusField::CloneUrl;
                }
            }
            UiAction::FocusWelcomeSearch => self.focus = FocusField::WelcomeSearch,
            UiAction::FocusCloneUrl => self.focus = FocusField::CloneUrl,
            UiAction::PickCloneDestination => {
                self.clone_destination = rfd::FileDialog::new().pick_folder();
            }
            UiAction::CloneRepository => {
                let Some(destination) = self.clone_destination.clone() else {
                    self.toast = Some("Choose a destination folder first".to_owned());
                    return;
                };
                let url = self.clone_url.trim().to_owned();
                if url.is_empty() {
                    self.toast = Some("Enter a repository URL".to_owned());
                    return;
                }
                self.submit_clone(url, destination);
            }
            UiAction::OpenExternalUrl(url) => {
                let result = if cfg!(target_os = "macos") {
                    Command::new("open").arg(&url).spawn()
                } else {
                    Command::new("xdg-open").arg(&url).spawn()
                };
                if let Err(error) = result {
                    self.toast = Some(format!("Open link: {error}"));
                }
            }
            UiAction::ToggleNotifications => {
                self.toggle_popup(Overlay::Notifications, FocusField::None)
            }
            UiAction::OpenPreferences => {
                self.preferences_open = true;
                self.close_overlay();
                self.focus = FocusField::None;
            }
            UiAction::ExitPreferences => self.preferences_open = false,
            UiAction::SelectPreferencePage(page) => {
                self.preference_page = page;
                self.preferences_scroll = 0.0;
            }
            UiAction::TogglePreference(key) => self.toggle_preference(&key),
            UiAction::AdjustPreference { key, delta } => self.adjust_preference(&key, delta),
            UiAction::TogglePathTree => self.path_tree = !self.path_tree,
            UiAction::ToggleViewAllFiles => {
                self.view_all_files = !self.view_all_files;
                // The current detail may lack the full tree listing.
                if self.selected_commits.len() <= 1
                    && let Some(id) = self.selected_commit.clone()
                {
                    self.request_detail(id);
                }
            }
            UiAction::CloseDetail => {
                self.detail = None;
                self.select_only(None);
            }
            UiAction::CloseDiff => {
                self.main_view = if self.selected_commit.is_some() {
                    MainView::Graph
                } else {
                    MainView::Wip
                };
                self.diff = None;
                self.selected_file = None;
            }
            UiAction::FocusPreferenceText(key) => {
                self.preference_text
                    .set_text(self.preference_text_value(&key));
                self.preference_text_key = Some(key);
                self.focus = FocusField::PreferenceText;
            }
            UiAction::AddCommitProfile => {
                let number = self.settings.profiles.len() + 1;
                let name = format!("Profile {number}");
                self.settings.profiles.push(crate::settings::CommitProfile {
                    name: name.clone(),
                    author_name: String::new(),
                    author_email: String::new(),
                });
                self.settings.selected_profile = Some(name);
                self.persist_settings();
            }
            UiAction::SelectCommitProfile(name) => {
                self.settings.selected_profile = Some(name);
                self.persist_settings();
            }
            UiAction::BrowsePreferencePath(key) => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.set_preference_text(&key, path.to_string_lossy().as_ref());
                }
            }
            UiAction::InitializeGitflow => self.submit_mutation(GitJobKind::InitGitflow {
                limit: self.requested_limit,
            }),
            UiAction::ApplySparseCheckout => self.submit_mutation(GitJobKind::SparseCheckout {
                paths: Some(self.settings.sparse_checkout_paths.clone()),
                limit: self.requested_limit,
            }),
            UiAction::DisableSparseCheckout => self.submit_mutation(GitJobKind::SparseCheckout {
                paths: None,
                limit: self.requested_limit,
            }),
            UiAction::AddLfsPattern => {
                let Some(pattern) = self.settings.lfs_patterns.last().cloned() else {
                    self.toast = Some("Enter an LFS pattern first".to_owned());
                    return;
                };
                self.submit_mutation(GitJobKind::TrackLfsPattern {
                    pattern,
                    limit: self.requested_limit,
                });
            }
            UiAction::OpenExternalEditor => self.open_external_tool(true),
            UiAction::OpenExternalTerminal => self.open_external_tool(false),
            UiAction::ToggleDiffLayout => self.diff_split = !self.diff_split,
            UiAction::ShowFileView => {
                self.diff_file_view = true;
                self.diff_scroll = 0.0;
                self.diff_scroll_target = 0.0;
                self.diff_scroll_updated = None;
            }
            UiAction::ShowDiffView => {
                self.diff_file_view = false;
                self.diff_scroll = 0.0;
                self.diff_scroll_target = 0.0;
                self.diff_scroll_updated = None;
            }
            UiAction::ToggleDiffScope => self.toggle_diff_scope(),
            UiAction::PreviousHunk => self.move_hunk(-1),
            UiAction::NextHunk => self.move_hunk(1),
            UiAction::SeekDiffRow(row) => self.seek_diff_row(row),
            UiAction::ScrollbarJump(target) => {
                self.set_scrollbar_fraction(
                    target,
                    self.scrollbar_fraction_at(target, self.mouse[1]),
                );
            }
            UiAction::BeginScrollbarDrag(target) => {
                self.scrollbar_drag = Some(target);
                self.set_scrollbar_fraction(
                    target,
                    self.scrollbar_fraction_at(target, self.mouse[1]),
                );
            }
            UiAction::ToggleFileHistory => self.file_history = !self.file_history,
            UiAction::RevealText => {}
            UiAction::ShowAiStatus => self.request_ai(),
            UiAction::DismissOverlay => {
                self.close_overlay();
                self.error = None;
                self.toast = None;
            }
            UiAction::BeginResize(target) => {
                let table = crate::views::Layout::for_state(self).center;
                let columns = crate::views::graph::column_layout(self, table);
                if matches!(
                    target,
                    ResizeTarget::RefColumn
                        | ResizeTarget::GraphColumn
                        | ResizeTarget::MessageColumn
                ) {
                    self.ref_column_width = columns.refs.width;
                    self.graph_column_width = columns.graph.width;
                    self.message_column_width = columns.message.width;
                }
                if let ResizeTarget::SidebarSection(index) = target {
                    self.sidebar_section_drag =
                        Some((index, self.mouse[1], self.sidebar_section_fractions));
                }
                self.drag = Some(target);
            }
        }
    }

    /// Records a click on a named ref and reports whether it completes a
    /// double click (same ref within the double-click window).
    fn take_ref_double_click(&mut self, kind: &str, name: &str) -> bool {
        const DOUBLE_CLICK_WINDOW: Duration = Duration::from_millis(450);
        let key = format!("{kind}:{name}");
        let now = Instant::now();
        if self
            .last_ref_click
            .take()
            .is_some_and(|(last, at)| last == key && now.duration_since(at) <= DOUBLE_CLICK_WINDOW)
        {
            true
        } else {
            self.last_ref_click = Some((key, now));
            false
        }
    }

    /// Dispatches the topmost left-click target under the pointer.
    pub(crate) fn click(&mut self) {
        if self.terminal_open
            && crate::views::Layout::for_state(self)
                .terminal
                .is_some_and(|rect| !rect.contains(self.mouse))
        {
            self.terminal_focused = false;
        }
        if self
            .active_popup_rect()
            .is_some_and(|rect| !rect.contains(self.mouse))
        {
            self.dispatch(UiAction::DismissOverlay);
            return;
        }

        let hovered = self
            .hits
            .iter()
            .rev()
            .filter(|hit| hit.rect.contains(self.mouse))
            .collect::<Vec<_>>();
        let target = hovered
            .iter()
            .find(|hit| {
                hit.action != UiAction::DismissOverlay && hit.action != UiAction::RevealText
            })
            .or_else(|| hovered.first())
            .map(|hit| (hit.action.clone(), hit.rect));
        if let Some((action, rect)) = target {
            if self.overlay == Overlay::None
                && matches!(action, UiAction::BranchClick(_) | UiAction::TagClick(_))
            {
                let (source, tag) = match &action {
                    UiAction::BranchClick(name) => (name.clone(), false),
                    UiAction::TagClick(name) => (name.clone(), true),
                    _ => unreachable!("guarded by the matches! above"),
                };
                self.ref_drag = Some(RefDrag {
                    source,
                    tag,
                    press: self.mouse,
                    active: false,
                    click: action,
                });
                return;
            }
            let focuses_text = matches!(
                action,
                UiAction::FocusCommitSummary
                    | UiAction::FocusCommitBody
                    | UiAction::FocusSearch
                    | UiAction::FocusBranchFilter
                    | UiAction::FocusTabFilter
                    | UiAction::FocusCreateBranch
                    | UiAction::FocusRenameBranch
                    | UiAction::FocusCreateTagName
                    | UiAction::FocusCreateTagMessage
                    | UiAction::FocusWelcomeSearch
                    | UiAction::FocusCloneUrl
                    | UiAction::FocusAddRemoteName
                    | UiAction::FocusAddRemoteUrl
                    | UiAction::FocusAddRemotePushUrl
                    | UiAction::FocusAddRemoteRepo
                    | UiAction::FocusAddRemoteHost
                    | UiAction::FocusPalette
                    | UiAction::FocusPreferenceText(_)
                    | UiAction::FocusEditMessageSummary
                    | UiAction::FocusEditMessageBody
            );
            self.dispatch(action);
            if focuses_text {
                self.place_caret_from_click(rect);
            }
        }
    }

    /// Places the caret at the pointer press inside a just-focused field.
    ///
    /// Column estimates mirror the fixed glyph advances the caret renderers
    /// use for each input surface, so the caret lands where it is drawn.
    fn place_caret_from_click(&mut self, rect: Rect) {
        let mouse = self.mouse;
        let editor_palette = self.overlay == Overlay::EditorPalette;
        let focus = self.focus;
        let Some(field) = self.focused_text() else {
            return;
        };
        // (text inset, estimated glyph advance) per input surface.
        let (inset, advance) = match focus {
            FocusField::BranchFilter if rect.height < 30.0 => (8.0, 6.0),
            FocusField::WelcomeSearch | FocusField::CloneUrl => (8.0, 6.0),
            FocusField::PreferenceText => (8.0, 6.9),
            FocusField::Palette => (12.0, 7.65),
            _ => (9.0, 7.1),
        };
        let mut column = ((mouse[0] - rect.x - inset) / advance).round().max(0.0) as usize;
        if focus == FocusField::Palette && editor_palette {
            // The editor palette renders a `>` prefix before the query.
            column = column.saturating_sub(1);
        }
        let line = match focus {
            FocusField::CommitBody => {
                (((mouse[1] - rect.y - 6.0) / 19.0).floor()).max(0.0) as usize
            }
            FocusField::EditMessageBody => {
                (((mouse[1] - rect.y - 7.0) / 19.0).floor()).max(0.0) as usize
            }
            _ => 0,
        };
        field.place_caret(line, column);
    }
    /// Opens branch actions for a right-clicked branch row.
    pub(crate) fn right_click(&mut self) {
        if self
            .active_popup_rect()
            .is_some_and(|rect| !rect.contains(self.mouse))
        {
            self.dispatch(UiAction::DismissOverlay);
            return;
        }
        let action = self.hits.iter().rev().find_map(|hit| {
            if !hit.rect.contains(self.mouse) {
                return None;
            }
            match &hit.action {
                UiAction::CheckoutBranch(branch) => {
                    Some(UiAction::OpenBranchContext(branch.clone()))
                }
                UiAction::BranchClick(branch) => Some(UiAction::OpenBranchContext(branch.clone())),
                UiAction::TagClick(tag) => Some(UiAction::OpenTagContext(tag.clone())),
                UiAction::OpenStashContext(index) => Some(UiAction::OpenStashContext(*index)),
                UiAction::OpenTagContext(tag) => Some(UiAction::OpenTagContext(tag.clone())),
                UiAction::SelectCommit(id) => Some(UiAction::OpenCommitContext(id.clone())),
                UiAction::SelectFile { path, scope } => Some(UiAction::OpenFileContext {
                    path: path.clone(),
                    scope: match scope {
                        DiffScope::Commit(id) => FileContextScope::Committed(id.clone()),
                        // Range rows act on the newest commit, whose tree
                        // provides the displayed content.
                        DiffScope::CommitRange { newest, .. } => {
                            FileContextScope::Committed(newest.clone())
                        }
                        DiffScope::Staged => FileContextScope::Staged,
                        DiffScope::Unstaged => FileContextScope::Unstaged,
                    },
                }),
                _ => None,
            }
        });
        if let Some(action) = action {
            self.dispatch(action);
        }
    }

    /// Returns the pointer cursor requested by the topmost hovered region;
    /// passive text-reveal regions never override an interactive cursor.
    pub(crate) fn cursor_hint(&self) -> CursorHint {
        self.hits
            .iter()
            .rev()
            .filter(|hit| hit.rect.contains(self.mouse))
            .find(|hit| hit.action != UiAction::RevealText)
            .map_or(CursorHint::Default, |hit| hit.cursor)
    }

    /// Returns a delayed-hover tooltip candidate for the current pointer.
    /// Scans down through stacked regions so passive truncation reveals under
    /// interactive rows still surface.
    pub(crate) fn tooltip(&self) -> Option<&str> {
        self.hits
            .iter()
            .rev()
            .filter(|hit| hit.rect.contains(self.mouse))
            .find_map(|hit| hit.tooltip.as_deref())
    }

    /// Updates an active splitter or diff-gutter selection drag.
    pub(crate) fn drag_to(&mut self, x: f32, y: f32) {
        if let Some(drag) = &mut self.ref_drag
            && !drag.active
            && (x - drag.press[0]).hypot(y - drag.press[1]) > 5.0
        {
            drag.active = true;
        }
        if let Some(start) = self.diff_drag_start
            && let Some(diff) = &self.diff
        {
            let row = ((y - CONTENT_TOP - 101.0 + self.diff_scroll)
                / crate::views::diff::ROW_HEIGHT)
                .floor()
                .to_usize()
                .unwrap_or(0)
                .min(diff.rows.len().saturating_sub(1));
            self.diff_selected_rows = (start.min(row)..=start.max(row)).collect();
        }
        if let Some((start_row, side, start_column)) = self.diff_text_drag
            && let Some(hit) = self.hits.iter().rev().find(|hit| hit.rect.contains([x, y]))
            && let UiAction::BeginDiffTextSelection {
                row,
                side: hit_side,
                ..
            } = hit.action
            && hit_side == side
        {
            let column = ((x - hit.rect.x) / 7.2)
                .max(0.0)
                .floor()
                .to_usize()
                .unwrap_or(0);
            self.diff_text_selection = Some(((start_row, side, start_column), (row, side, column)));
        }
        if let Some(target) = self.scrollbar_drag {
            self.set_scrollbar_fraction(target, self.scrollbar_fraction_at(target, y));
            return;
        }
        let Some(target) = self.drag else {
            return;
        };
        let width = px(self.width);
        let layout = crate::views::Layout::for_state(self);
        match target {
            ResizeTarget::Sidebar => {
                if !self.settings.sidebar_collapsed {
                    self.sidebar_width = x.clamp(190.0, width * 0.42);
                }
            }
            ResizeTarget::SidebarSection(index) => {
                if let Some((dragged, start_y, initial)) = self.sidebar_section_drag
                    && dragged == index
                    && let Some(next) = usize::from(index).checked_add(1).filter(|next| *next < 5)
                {
                    let pair = initial[usize::from(index)] + initial[next];
                    let delta = (y - start_y) / layout.sidebar.height.max(1.0);
                    let left = (initial[usize::from(index)] + delta).clamp(0.10, pair - 0.10);
                    self.sidebar_section_fractions[usize::from(index)] = left;
                    self.sidebar_section_fractions[next] = pair - left;
                }
            }
            ResizeTarget::DetailPanel => {
                let maximum = (width - layout.sidebar.width - 320.0).max(340.0);
                self.detail_width = (width - x).clamp(340.0, maximum);
            }
            ResizeTarget::RefColumn => {
                self.ref_column_width =
                    crate::views::graph::resize_preference(self, layout.center, target, x);
            }
            ResizeTarget::GraphColumn => {
                self.graph_column_width =
                    crate::views::graph::resize_preference(self, layout.center, target, x);
            }
            ResizeTarget::MessageColumn => {
                self.message_column_width =
                    crate::views::graph::resize_preference(self, layout.center, target, x);
            }
            ResizeTarget::TerminalPane => {
                if let Some(terminal) = layout.terminal {
                    let available_height = layout.center.height + terminal.height;
                    let font_size = f32::from(self.settings.terminal_font_size.max(8));
                    let minimum = (font_size * 1.2 * 3.0 + 24.0).min(available_height);
                    let maximum = (available_height * 0.8).max(minimum);
                    let pane_height = (layout.status.y - y).clamp(minimum, maximum);
                    self.terminal_height_fraction = pane_height / available_height.max(1.0);
                }
            }
            ResizeTarget::DetailMessage => {
                if let Some(detail) = layout.detail {
                    self.detail_message_height =
                        (y - detail.y).clamp(110.0, (detail.height * 0.7).max(110.0));
                }
            }
        }
        self.resize_terminal();
    }

    fn scrollbar_fraction_at(&self, target: ScrollTarget, y: f32) -> f32 {
        let (viewport, _, _) = self.scrollbar_metrics(target);
        ((y - viewport.y) / viewport.height.max(1.0)).clamp(0.0, 1.0)
    }

    fn set_scrollbar_fraction(&mut self, target: ScrollTarget, fraction: f32) {
        let (_, content_height, scroll) = self.scrollbar_metrics(target);
        *scroll(self) = (content_height - self.scrollbar_metrics(target).0.height).max(0.0)
            * fraction.clamp(0.0, 1.0);
    }

    fn sidebar_local_scroll_ref(state: &mut Self) -> &mut f32 {
        &mut state.sidebar_local_scroll
    }

    fn sidebar_remote_scroll_ref(state: &mut Self) -> &mut f32 {
        &mut state.sidebar_remote_scroll
    }

    fn sidebar_worktrees_scroll_ref(state: &mut Self) -> &mut f32 {
        &mut state.sidebar_worktrees_scroll
    }

    fn sidebar_stashes_scroll_ref(state: &mut Self) -> &mut f32 {
        &mut state.sidebar_stashes_scroll
    }

    fn sidebar_tags_scroll_ref(state: &mut Self) -> &mut f32 {
        &mut state.sidebar_tags_scroll
    }

    fn scrollbar_metrics(&self, target: ScrollTarget) -> (Rect, f32, fn(&mut Self) -> &mut f32) {
        let layout = crate::views::Layout::for_state(self);
        match target {
            ScrollTarget::Graph => {
                let body = Rect::new(
                    layout.center.x,
                    layout.center.y + COMMIT_HEADER_HEIGHT,
                    layout.center.width,
                    (layout.center.height - COMMIT_HEADER_HEIGHT).max(0.0),
                );
                let rows = self
                    .graph
                    .rows
                    .len()
                    .saturating_add(self.snapshot.as_ref().map_or(0, RepoSnapshot::wip_rows));
                (
                    body,
                    rows.to_f32().unwrap_or(0.0) * COMMIT_ROW_HEIGHT,
                    |state| &mut state.graph_scroll,
                )
            }
            ScrollTarget::Detail => {
                let rect = layout.detail.unwrap_or(layout.center);
                let header_height = self.detail.as_ref().map_or(39.0, |detail| {
                    let body = (!detail.body.is_empty())
                        .then(|| detail.body.lines().count().clamp(1, 10) as f32 * 18.0 + 18.0)
                        .unwrap_or(0.0);
                    let conflicts = (!detail.conflicts.is_empty())
                        .then(|| 32.0 + detail.conflicts.iter().take(5).count() as f32 * 18.0)
                        .unwrap_or(0.0);
                    39.0 + 18.0 + 55.0 + body + conflicts + 15.0
                });
                let viewport = Rect::new(
                    rect.x + 1.0,
                    rect.y + header_height,
                    rect.width - 2.0,
                    (rect.height - header_height).max(0.0),
                );
                let rows = crate::views::commit_detail::detail_row_count(self);
                (viewport, 430.0 + rows as f32 * 24.0, |state| {
                    &mut state.detail_scroll
                })
            }
            ScrollTarget::WipUnstaged | ScrollTarget::WipStaged => {
                let rect = layout.detail.unwrap_or(layout.center);
                let sections = crate::views::wip::section_layout(self, rect);
                if target == ScrollTarget::WipUnstaged {
                    (sections.unstaged_view, sections.unstaged_content, |state| {
                        &mut state.wip_unstaged_scroll
                    })
                } else {
                    (sections.staged_view, sections.staged_content, |state| {
                        &mut state.wip_staged_scroll
                    })
                }
            }
            ScrollTarget::Diff => {
                let viewport = Rect::new(
                    layout.center.x,
                    layout.center.y + 101.0,
                    layout.center.width,
                    (layout.center.height - 101.0).max(0.0),
                );
                let content = self.diff.as_ref().map_or(0.0, |diff| {
                    diff.rows.len() as f32 * crate::views::diff::ROW_HEIGHT
                });
                (viewport, content, |state| &mut state.diff_scroll)
            }
            ScrollTarget::SidebarLocal
            | ScrollTarget::SidebarRemote
            | ScrollTarget::SidebarWorktrees
            | ScrollTarget::SidebarStashes
            | ScrollTarget::SidebarTags => {
                let (viewport, content_height) =
                    crate::views::shell::sidebar_scrollbar_metrics(self, target)
                        .unwrap_or((layout.sidebar, 0.0));
                let scroll: fn(&mut Self) -> &mut f32 = match target {
                    ScrollTarget::SidebarLocal => Self::sidebar_local_scroll_ref,
                    ScrollTarget::SidebarRemote => Self::sidebar_remote_scroll_ref,
                    ScrollTarget::SidebarWorktrees => Self::sidebar_worktrees_scroll_ref,
                    ScrollTarget::SidebarStashes => Self::sidebar_stashes_scroll_ref,
                    ScrollTarget::SidebarTags => Self::sidebar_tags_scroll_ref,
                    _ => unreachable!("sidebar target already matched"),
                };
                (viewport, content_height, scroll)
            }
            ScrollTarget::Preferences => (layout.center, layout.center.height, |state| {
                &mut state.preferences_scroll
            }),
        }
    }

    pub(crate) fn is_dragging(&self) -> bool {
        self.drag.is_some()
            || self.scrollbar_drag.is_some()
            || self.diff_drag_start.is_some()
            || self.diff_text_drag.is_some()
            || self.ref_drag.is_some()
    }

    /// Ends a pointer-driven splitter or diff-gutter selection drag.
    pub(crate) fn end_drag(&mut self) {
        if self.diff_drag_start.take().is_some() && !self.diff_selected_rows.is_empty() {
            self.overlay_anchor = self.mouse;
            self.show_popup(Overlay::DiffSelection, FocusField::None);
        }
        if let Some(drag) = self.ref_drag.take() {
            if drag.active {
                let target = self.hits.iter().rev().find_map(|hit| {
                    if !hit.rect.contains(self.mouse) {
                        return None;
                    }
                    match &hit.action {
                        UiAction::BranchClick(name) if *name != drag.source => {
                            Some((name.clone(), false))
                        }
                        UiAction::TagClick(name) if *name != drag.source => {
                            Some((name.clone(), true))
                        }
                        _ => None,
                    }
                });
                if let Some((target, target_tag)) = target {
                    self.overlay_anchor = self.mouse;
                    self.show_popup(
                        Overlay::DropMenu {
                            source: drag.source,
                            source_tag: drag.tag,
                            target,
                            target_tag,
                        },
                        FocusField::None,
                    );
                }
            } else {
                self.dispatch(drag.click);
            }
        }
        self.diff_text_drag = None;
        if self.drag == Some(ResizeTarget::GraphColumn) {
            self.graph_column_explicit = true;
        }
        self.drag = None;
        self.scrollbar_drag = None;
        self.sidebar_section_drag = None;
    }

    /// Scrolls the panel under the pointer and requests older commits near graph end.
    pub(crate) fn scroll(&mut self, delta: f32) {
        if self.preferences_open {
            self.preferences_scroll = (self.preferences_scroll + delta).max(0.0);
            return;
        }
        if let Some(skin) = crate::views::palette::skin(&self.overlay) {
            if let Some(palette) = &mut self.palette {
                let count = crate::views::palette::filtered_indices(skin, &palette.query).len();
                let maximum = count.saturating_sub(8);
                let amount = (delta / 40.0).round() as isize;
                palette.scroll = palette.scroll.saturating_add_signed(amount).min(maximum);
            }
            return;
        }
        let top = CONTENT_TOP;
        if self.mouse[1] < top {
            return;
        }
        let layout = crate::views::Layout::for_state(self);
        if self.mouse[0] < layout.sidebar.width {
            if let Some(target) = crate::views::shell::sidebar_scroll_target_at(self, self.mouse) {
                let (_, content_height, scroll) = self.scrollbar_metrics(target);
                let maximum = (content_height - self.scrollbar_metrics(target).0.height).max(0.0);
                *scroll(self) = (*scroll(self) + delta).clamp(0.0, maximum);
            }
            return;
        }
        if let Some(detail_rect) = layout.detail
            && detail_rect.contains(self.mouse)
        {
            // The right panel owns the wheel: WIP file sections scroll
            // individually under the pointer, never the graph or diff behind.
            if crate::views::detail_shows_wip(self) {
                if let Some(target) = crate::views::wip::scroll_target_at(self, self.mouse) {
                    let (viewport, content_height, scroll) = self.scrollbar_metrics(target);
                    let maximum = (content_height - viewport.height).max(0.0);
                    *scroll(self) = (*scroll(self) + delta).clamp(0.0, maximum);
                }
            } else {
                self.detail_scroll = (self.detail_scroll + delta).max(0.0);
            }
            return;
        }
        if self.terminal_open
            && layout
                .terminal
                .is_some_and(|rect| rect.contains(self.mouse))
        {
            if let Some(terminal) = &self.terminal {
                terminal.scroll((-delta / 18.0).round() as i32);
            }
            return;
        }
        match self.main_view {
            MainView::Graph | MainView::Wip => {
                let viewport =
                    (px(self.height) - top - STATUS_BAR_HEIGHT - COMMIT_HEADER_HEIGHT).max(0.0);
                let wip_rows = self.snapshot.as_ref().map_or(0, RepoSnapshot::wip_rows);
                self.graph_scroll = (self.graph_scroll + delta).clamp(
                    0.0,
                    self.graph.max_scroll(viewport, COMMIT_ROW_HEIGHT, wip_rows),
                );
                let total_rows = self.graph.rows.len().saturating_add(wip_rows);
                let near_end = self.graph_scroll + viewport
                    >= total_rows.to_f32().unwrap_or(f32::MAX) * COMMIT_ROW_HEIGHT
                        - COMMIT_ROW_HEIGHT * 10.0;
                if near_end
                    && !self.loading_history
                    && self
                        .snapshot
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.has_more)
                    && self.settings.lazy_load_commits
                {
                    self.loading_history = true;
                    self.requested_limit = self.requested_limit.saturating_mul(2).min(100_000);
                    self.submit(GitJobKind::LoadHistory {
                        limit: self.requested_limit,
                    });
                }
            }
            MainView::Diff => self.scroll_diff_pixels(delta),
        }
    }

    /// Inserts committed IME or typed text into the focused field, replacing
    /// any selection. Newlines are kept in the commit body, mapped to spaces
    /// in the commit summary, and stripped everywhere else.
    pub(crate) fn insert_text(&mut self, text: &str) {
        let filtered = text
            .chars()
            .filter(|character| !character.is_control() || *character == '\n')
            .collect::<String>();
        let filtered = match self.focus {
            FocusField::CommitSummary => filtered.replace('\n', " "),
            FocusField::CommitBody | FocusField::EditMessageBody => filtered,
            _ => filtered.replace('\n', ""),
        };
        let Some(field) = self.focused_text() else {
            return;
        };
        field.insert(&filtered);
        self.after_edit();
    }

    /// The text field owning keyboard focus, when there is one.
    fn focused_text(&mut self) -> Option<&mut TextField> {
        match self.focus {
            FocusField::CommitSummary => Some(&mut self.commit_summary),
            FocusField::CommitBody => Some(&mut self.commit_body),
            FocusField::Search => Some(&mut self.search),
            FocusField::DiffSearch => Some(&mut self.diff_search),
            FocusField::BranchFilter => Some(&mut self.branch_filter),
            FocusField::CreateBranch => Some(&mut self.new_branch),
            FocusField::RenameBranch => Some(&mut self.renamed_branch),
            FocusField::CreateTagName => Some(&mut self.tag_name),
            FocusField::CreateTagMessage => Some(&mut self.tag_message),
            FocusField::EditMessageSummary => Some(&mut self.edit_summary),
            FocusField::EditMessageBody => Some(&mut self.edit_body),
            FocusField::TabFilter => Some(&mut self.tab_filter),
            FocusField::WelcomeSearch => Some(&mut self.welcome_search),
            FocusField::CloneUrl => Some(&mut self.clone_url),
            FocusField::AddRemoteName => Some(&mut self.add_remote_name),
            FocusField::AddRemoteUrl => Some(&mut self.add_remote_url),
            FocusField::AddRemotePushUrl => Some(&mut self.add_remote_push_url),
            FocusField::AddRemoteRepo => Some(&mut self.add_remote_repo),
            FocusField::AddRemoteHost => Some(&mut self.add_remote_host),
            FocusField::Palette => self.palette.as_mut().map(|palette| &mut palette.query),
            FocusField::PreferenceText => Some(&mut self.preference_text),
            FocusField::None => None,
        }
    }

    /// Reacts to a completed edit of the focused field: search results track
    /// their query, the palette resets its selection, and preference text is
    /// written through to the settings value it mirrors.
    fn after_edit(&mut self) {
        match self.focus {
            FocusField::Search => self.search_cursor = 0,
            FocusField::DiffSearch => self.diff_search_cursor = 0,
            FocusField::Palette => {
                if let Some(palette) = &mut self.palette {
                    crate::views::palette::reset_selection(palette);
                }
            }
            FocusField::PreferenceText => {
                if let Some(key) = self.preference_text_key.clone() {
                    let value = self.preference_text.text().to_owned();
                    self.set_preference_text(&key, &value);
                }
            }
            _ => {}
        }
    }

    /// Applies a caret, deletion, or clipboard keystroke to the focused field.
    ///
    /// Returns `false` when the key is not a text-editing command for the
    /// current focus so front-ends fall through to their global bindings.
    pub(crate) fn edit_key(&mut self, key: EditKey) -> bool {
        if self.focus == FocusField::None {
            return false;
        }
        let jump = if key.command {
            Jump::Edge
        } else if key.alt {
            Jump::Word
        } else {
            Jump::Char
        };
        match key.kind {
            EditKeyKind::Char(character) if key.command && !key.alt => {
                match character.to_ascii_lowercase() {
                    'a' if !key.shift => self.focused_text().map(TextField::select_all).is_some(),
                    'c' => {
                        let text = self
                            .focused_text()
                            .map(|field| field.selected_text().to_owned())
                            .unwrap_or_default();
                        if text.is_empty() {
                            return false;
                        }
                        self.copy_text(text);
                        true
                    }
                    'x' => {
                        let Some(text) = self
                            .focused_text()
                            .map(TextField::take_selection)
                            .filter(|text| !text.is_empty())
                        else {
                            return false;
                        };
                        self.copy_text(text);
                        self.after_edit();
                        true
                    }
                    'v' => {
                        if let Ok(text) =
                            arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
                        {
                            self.insert_text(&text);
                        }
                        true
                    }
                    _ => false,
                }
            }
            EditKeyKind::Char(_) => false,
            EditKeyKind::Up | EditKeyKind::Down
                if !matches!(
                    self.focus,
                    FocusField::CommitBody | FocusField::EditMessageBody
                ) =>
            {
                false
            }
            _ => {
                let Some(field) = self.focused_text() else {
                    return false;
                };
                match key.kind {
                    EditKeyKind::Left => field.move_left(jump, key.shift),
                    EditKeyKind::Right => field.move_right(jump, key.shift),
                    EditKeyKind::Home => field.move_left(Jump::Edge, key.shift),
                    EditKeyKind::End => field.move_right(Jump::Edge, key.shift),
                    EditKeyKind::Up => field.move_vertical(-1, key.shift),
                    EditKeyKind::Down => field.move_vertical(1, key.shift),
                    EditKeyKind::Backspace => field.backspace(jump),
                    EditKeyKind::Delete => field.delete_forward(jump),
                    EditKeyKind::Char(_) => unreachable!("handled above"),
                }
                if matches!(key.kind, EditKeyKind::Backspace | EditKeyKind::Delete) {
                    self.after_edit();
                }
                true
            }
        }
    }

    /// Executes the focused field's primary keyboard action.
    pub(crate) fn enter(&mut self, command: bool) {
        match self.focus {
            FocusField::DiffSearch => self.dispatch(UiAction::NextDiffSearch),
            FocusField::Palette => {
                let index = self.palette.as_ref().map_or(0, |palette| palette.cursor);
                self.dispatch(UiAction::ExecutePaletteCommand(index));
            }
            FocusField::CommitSummary | FocusField::CommitBody if command => {
                self.dispatch(UiAction::Commit)
            }
            FocusField::CommitBody => self.commit_body.insert("\n"),
            FocusField::CreateBranch => self.dispatch(UiAction::CreateBranch),
            FocusField::RenameBranch => self.dispatch(UiAction::RenameBranch),
            FocusField::CreateTagName | FocusField::CreateTagMessage => {
                self.dispatch(UiAction::CreateTag)
            }
            FocusField::EditMessageSummary | FocusField::EditMessageBody if command => {
                self.dispatch(UiAction::ConfirmEditMessage)
            }
            FocusField::EditMessageBody => self.edit_body.insert("\n"),
            FocusField::EditMessageSummary => self.dispatch(UiAction::ConfirmEditMessage),
            FocusField::Search => self.dispatch(UiAction::NextSearchResult),
            FocusField::CloneUrl => self.dispatch(UiAction::CloneRepository),
            FocusField::AddRemoteName
            | FocusField::AddRemoteUrl
            | FocusField::AddRemotePushUrl
            | FocusField::AddRemoteRepo
            | FocusField::AddRemoteHost => self.dispatch(UiAction::AddRemote),
            FocusField::BranchFilter
            | FocusField::TabFilter
            | FocusField::WelcomeSearch
            | FocusField::CommitSummary
            | FocusField::PreferenceText
            | FocusField::None => {}
        }
    }

    /// Closes the highest-level transient surface.
    pub(crate) fn escape(&mut self) {
        if self.preferences_open {
            self.preferences_open = false;
        } else if self.overlay != Overlay::None || self.error.is_some() {
            self.dispatch(UiAction::DismissOverlay);
        } else if self.focus == FocusField::DiffSearch {
            self.dispatch(UiAction::CloseDiffSearch);
        } else if self.focus == FocusField::Search {
            self.dispatch(UiAction::CloseSearch);
        } else if self.focus != FocusField::None {
            self.focus = FocusField::None;
        } else if self.main_view == MainView::Diff {
            self.dispatch(UiAction::CloseDiff);
        }
    }

    /// Returns case-insensitive graph search matches in commit order.
    pub(crate) fn search_results(&self) -> Vec<usize> {
        let query = self.search.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .commits
                    .iter()
                    .enumerate()
                    .filter_map(|(index, commit)| {
                        (commit.subject.to_lowercase().contains(&query)
                            || commit.author.to_lowercase().contains(&query)
                            || commit.id.contains(&query))
                        .then_some(index)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// True while inertial diff scrolling needs another animation frame.
    pub(crate) fn diff_scroll_animating(&self) -> bool {
        self.diff_scroll_updated.is_some()
    }

    pub(crate) fn advance_animations(&mut self) {
        let Some(updated) = self.diff_scroll_updated else {
            return;
        };
        let elapsed = updated.elapsed().as_secs_f32();
        let remaining = self.diff_scroll_target - self.diff_scroll;
        if elapsed >= 0.15 || remaining.abs() < 0.25 {
            self.diff_scroll = self.diff_scroll_target;
            self.diff_scroll_updated = None;
            return;
        }
        self.diff_scroll += remaining * (1.0 - (-18.0 * elapsed).exp());
        self.diff_scroll_updated = Some(Instant::now());
    }

    pub(crate) fn scroll_diff_lines(&mut self, delta: f32) {
        self.diff_scroll_target =
            (self.diff_scroll_target + delta).clamp(0.0, self.max_diff_scroll());
        self.diff_scroll_updated = Some(Instant::now());
    }

    pub(crate) fn scroll_diff_pixels(&mut self, delta: f32) {
        self.diff_scroll = (self.diff_scroll + delta).clamp(0.0, self.max_diff_scroll());
        self.diff_scroll_target = self.diff_scroll;
        self.diff_scroll_updated = None;
    }

    fn max_diff_scroll(&self) -> f32 {
        let content = self.diff.as_ref().map_or(0.0, |diff| {
            diff.rows.len().to_f32().unwrap_or(0.0) * crate::views::diff::ROW_HEIGHT
        });
        (content - (px(self.height) - CONTENT_TOP - 101.0).max(0.0)).max(0.0)
    }

    fn begin_diff_text_selection(&mut self, row: usize, side: u8, column: usize, clicks: u8) {
        let point = (row, side, column);
        let click_count = self
            .diff_last_click
            .as_ref()
            .map_or(clicks, |(last, at, count)| {
                if *last == point && at.elapsed().as_millis() < 450 {
                    count.saturating_add(1).min(3)
                } else {
                    clicks
                }
            });
        self.diff_last_click = Some((point, Instant::now(), click_count));
        let Some(text) = self.diff_text_at(row, side) else {
            return;
        };
        let count = text.chars().count();
        let selection = if click_count >= 3 {
            ((row, side, 0), (row, side, count))
        } else if click_count == 2 {
            let characters = text.chars().collect::<Vec<_>>();
            let pivot = column.min(count);
            let is_word = |character: char| character.is_alphanumeric() || character == '_';
            let start = characters[..pivot]
                .iter()
                .rposition(|c| !is_word(*c))
                .map_or(0, |index| index + 1);
            let end = characters[pivot..]
                .iter()
                .position(|&c| !is_word(c))
                .map_or(count, |index| pivot + index);
            ((row, side, start), (row, side, end))
        } else {
            (point, point)
        };
        self.diff_text_selection = Some(selection);
        self.diff_text_drag = Some(selection.0);
    }
    fn copy_text(&mut self, text: String) {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                if let Err(error) = clipboard.set_text(text) {
                    self.error = Some(format!("copy text: {error}"));
                }
            }
            Err(error) => self.error = Some(format!("open clipboard: {error}")),
        }
    }

    fn diff_text_at(&self, row: usize, side: u8) -> Option<&str> {
        let row = self.diff.as_ref()?.rows.get(row)?;
        Some(if side == 0 {
            &row.old_text
        } else {
            &row.new_text
        })
    }

    fn copy_diff_text(&mut self) {
        let Some(((start_row, side, start_column), (end_row, _, end_column))) =
            self.diff_text_selection
        else {
            return;
        };
        let Some(diff) = &self.diff else {
            return;
        };
        let (first_row, first_column, last_row, last_column) = if start_row <= end_row {
            (start_row, start_column, end_row, end_column)
        } else {
            (end_row, end_column, start_row, start_column)
        };
        let text = (first_row..=last_row)
            .filter_map(|row| {
                let line = diff.rows.get(row)?;
                let line = if side == 0 {
                    &line.old_text
                } else {
                    &line.new_text
                };
                let count = line.chars().count();
                let start = if row == first_row {
                    first_column.min(count)
                } else {
                    0
                };
                let end = if row == last_row {
                    last_column.min(count)
                } else {
                    count
                };
                Some(
                    line.chars()
                        .skip(start)
                        .take(end.saturating_sub(start))
                        .collect::<String>(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Ok(mut clipboard) = arboard::Clipboard::new()
            && let Err(error) = clipboard.set_text(text)
        {
            self.error = Some(format!("copy selected text: {error}"));
        }
    }

    fn move_diff_search(&mut self, delta: i32) {
        let matches = self.diff_search_results();
        if matches.is_empty() {
            self.diff_search_cursor = 0;
            return;
        }
        self.diff_search_cursor = if delta < 0 {
            self.diff_search_cursor
                .checked_sub(1)
                .unwrap_or(matches.len() - 1)
        } else {
            (self.diff_search_cursor + 1) % matches.len()
        };
        self.diff_scroll_target = (matches[self.diff_search_cursor].0.to_f32().unwrap_or(0.0)
            * crate::views::diff::ROW_HEIGHT
            - 80.0)
            .clamp(0.0, self.max_diff_scroll());
        self.diff_scroll_updated = Some(Instant::now());
    }

    pub(crate) fn diff_search_results(&self) -> Vec<(usize, u8, usize, usize)> {
        let query = self.diff_search.to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        let mut results = Vec::new();
        if let Some(diff) = &self.diff {
            for (row, line) in diff.rows.iter().enumerate() {
                let columns = if self.diff_split {
                    [
                        (0_u8, line.old_text.as_str()),
                        (1_u8, line.new_text.as_str()),
                    ]
                } else if line.kind == DiffRowKind::Deleted {
                    [(0_u8, line.old_text.as_str()), (0_u8, "")]
                } else {
                    [(1_u8, line.new_text.as_str()), (1_u8, "")]
                };
                for (side, text) in columns {
                    let lower = text.to_lowercase();
                    for (byte, _) in lower.match_indices(&query) {
                        let start = text[..byte].chars().count();
                        results.push((row, side, start, start + query.chars().count()));
                    }
                }
            }
        }
        results
    }

    fn move_search(&mut self, delta: i32) {
        let results = self.search_results();
        if results.is_empty() {
            self.search_cursor = 0;
            return;
        }
        if delta < 0 {
            self.search_cursor = self
                .search_cursor
                .checked_sub(1)
                .unwrap_or(results.len() - 1);
        } else {
            self.search_cursor = (self.search_cursor + 1) % results.len();
        }
        if let Some(index) = results.get(self.search_cursor) {
            self.graph_scroll = index.to_f32().unwrap_or(0.0) * 26.0;
            if let Some(id) = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.commits.get(*index))
                .map(|commit| commit.id.clone())
            {
                self.dispatch(UiAction::SelectCommit(id));
            }
        }
    }

    fn move_hunk(&mut self, delta: i32) {
        let Some(diff) = &self.diff else {
            return;
        };
        if diff.hunks.is_empty() {
            return;
        }
        if delta < 0 {
            self.current_hunk = self
                .current_hunk
                .checked_sub(1)
                .unwrap_or(diff.hunks.len() - 1);
        } else {
            self.current_hunk = (self.current_hunk + 1) % diff.hunks.len();
        }
        self.diff_scroll = seek_scroll(diff.hunks[self.current_hunk]);
        self.diff_scroll_target = self.diff_scroll;
        self.diff_scroll_updated = None;
    }

    /// Scrolls the diff canvas so `row` sits near the top and syncs hunk nav.
    fn seek_diff_row(&mut self, row: usize) {
        let Some(diff) = &self.diff else {
            return;
        };
        self.current_hunk = diff
            .hunks
            .iter()
            .rposition(|&hunk| hunk <= row)
            .unwrap_or(0);
        self.diff_scroll = seek_scroll(row);
        self.diff_scroll_target = self.diff_scroll;
        self.diff_scroll_updated = None;
    }

    fn toggle_diff_scope(&mut self) {
        let Some(mut request) = self.selected_file.clone() else {
            return;
        };
        request.scope = match request.scope {
            DiffScope::Staged => DiffScope::Unstaged,
            DiffScope::Unstaged => DiffScope::Staged,
            DiffScope::Commit(_) | DiffScope::CommitRange { .. } => return,
        };
        self.selected_file = Some(request.clone());
        self.diff = None;
        self.submit(GitJobKind::LoadDiff { request });
    }

    fn request_ai(&mut self) {
        self.show_popup(Overlay::Ai, FocusField::None);
        self.ai_message = None;
        if let Some(detail) = self.detail.clone() {
            self.ai_loading = true;
            if self.commit_is_local(&detail.id) {
                self.ai.recompose(detail);
            } else {
                self.ai.explain(detail);
            }
        } else {
            self.ai_message = Some("Select a commit before requesting an AI action.".to_owned());
        }
    }

    fn toggle_preference(&mut self, key: &str) {
        match key {
            "auto_prune" => self.settings.auto_prune = !self.settings.auto_prune,
            "keep_submodules_updated" => {
                self.settings.keep_submodules_updated = !self.settings.keep_submodules_updated;
            }
            "delete_orig_after_merge" => {
                self.settings.delete_orig_after_merge = !self.settings.delete_orig_after_merge;
            }
            "show_all_commits" => {
                self.settings.show_all_commits = !self.settings.show_all_commits;
            }
            "lazy_load_commits" => {
                self.settings.lazy_load_commits = !self.settings.lazy_load_commits;
            }
            "remember_tabs" => self.settings.remember_tabs = !self.settings.remember_tabs,
            "extended_logging" => {
                self.settings.extended_logging = !self.settings.extended_logging;
            }
            "proactive_conflict_detection" => {
                self.settings.proactive_conflict_detection =
                    !self.settings.proactive_conflict_detection;
            }
            "share_branch_status" => {
                self.settings.share_branch_status = !self.settings.share_branch_status;
            }
            "show_agents" => self.settings.show_agents = !self.settings.show_agents,
            "spell_check" => self.settings.spell_check = !self.settings.spell_check,
            "show_commit_author" => {
                self.settings.show_commit_author = !self.settings.show_commit_author;
            }
            "show_commit_date" => {
                self.settings.show_commit_date = !self.settings.show_commit_date;
            }
            "show_commit_sha" => {
                self.settings.show_commit_sha = !self.settings.show_commit_sha;
            }
            "use_local_ssh_agent" => {
                self.settings.use_local_ssh_agent = !self.settings.use_local_ssh_agent;
            }
            "show_external_tool_arguments" => {
                self.settings.show_external_tool_arguments =
                    !self.settings.show_external_tool_arguments;
            }
            "sign_commits_by_default" => {
                self.settings.sign_commits_by_default = !self.settings.sign_commits_by_default;
            }
            "sign_tags_by_default" => {
                self.settings.sign_tags_by_default = !self.settings.sign_tags_by_default;
            }
            "use_git_executable" => {
                self.settings.use_git_executable = !self.settings.use_git_executable;
            }
            "notify_operation_success" => {
                self.settings.notify_operation_success = !self.settings.notify_operation_success;
            }
            "notify_operation_failure" => {
                self.settings.notify_operation_failure = !self.settings.notify_operation_failure;
            }
            "notify_fetch_results" => {
                self.settings.notify_fetch_results = !self.settings.notify_fetch_results;
            }
            _ => return,
        }
        self.persist_settings();
    }

    fn adjust_preference(&mut self, key: &str, delta: i32) {
        match key {
            "auto_fetch_minutes" => {
                self.settings.auto_fetch_minutes =
                    add_signed(self.settings.auto_fetch_minutes, delta, 0, 120);
            }
            "initial_commits" => {
                let step = delta.saturating_mul(100);
                self.settings.initial_commits =
                    add_signed(self.settings.initial_commits, step, 100, 100_000);
            }
            "editor_font_size" => {
                self.settings.editor_font_size =
                    add_signed(self.settings.editor_font_size, delta, 8, 32);
            }
            "terminal_font_size" => {
                self.settings.terminal_font_size =
                    add_signed(self.settings.terminal_font_size, delta, 8, 32);
                self.resize_terminal();
            }
            _ => return,
        }
        self.persist_settings();
    }

    fn persist_settings(&mut self) {
        if let Err(error) = self.settings_store.save(&self.settings) {
            self.error = Some(format!("{error:#}"));
        }
    }

    pub(crate) fn preference_text_value(&self, key: &str) -> String {
        match key {
            "ssh_private_key" => self.settings.ssh_private_key.clone(),
            "ssh_public_key" => self.settings.ssh_public_key.clone(),
            "external_editor" => self.settings.external_editor.clone(),
            "external_terminal" => self.settings.external_terminal.clone(),
            "gpg_program" => self.settings.gpg_program.clone(),
            "gpg_key_id" => self.settings.gpg_key_id.clone(),
            "git_executable" => self.settings.git_executable.clone(),
            "default_encoding" => self.settings.default_encoding.clone(),
            "gitflow_main_branch" => self.settings.gitflow_main_branch.clone(),
            "gitflow_develop_branch" => self.settings.gitflow_develop_branch.clone(),
            "gitflow_feature_prefix" => self.settings.gitflow_feature_prefix.clone(),
            "gitflow_release_prefix" => self.settings.gitflow_release_prefix.clone(),
            "gitflow_hotfix_prefix" => self.settings.gitflow_hotfix_prefix.clone(),
            "sparse_checkout_paths" => self.settings.sparse_checkout_paths.clone(),
            "lfs_pattern" => self
                .settings
                .lfs_patterns
                .last()
                .cloned()
                .unwrap_or_default(),
            "profile_name" | "profile_author_name" | "profile_author_email" => self
                .settings
                .selected_profile
                .as_ref()
                .and_then(|name| {
                    self.settings
                        .profiles
                        .iter()
                        .find(|profile| &profile.name == name)
                })
                .map(|profile| match key {
                    "profile_name" => profile.name.clone(),
                    "profile_author_name" => profile.author_name.clone(),
                    _ => profile.author_email.clone(),
                })
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    fn set_preference_text(&mut self, key: &str, value: &str) {
        match key {
            "ssh_private_key" => self.settings.ssh_private_key = value.to_owned(),
            "ssh_public_key" => self.settings.ssh_public_key = value.to_owned(),
            "external_editor" => self.settings.external_editor = value.to_owned(),
            "external_terminal" => self.settings.external_terminal = value.to_owned(),
            "gpg_program" => self.settings.gpg_program = value.to_owned(),
            "gpg_key_id" => self.settings.gpg_key_id = value.to_owned(),
            "git_executable" => self.settings.git_executable = value.to_owned(),
            "default_encoding" => self.settings.default_encoding = value.to_owned(),
            "gitflow_main_branch" => self.settings.gitflow_main_branch = value.to_owned(),
            "gitflow_develop_branch" => self.settings.gitflow_develop_branch = value.to_owned(),
            "gitflow_feature_prefix" => self.settings.gitflow_feature_prefix = value.to_owned(),
            "gitflow_release_prefix" => self.settings.gitflow_release_prefix = value.to_owned(),
            "gitflow_hotfix_prefix" => self.settings.gitflow_hotfix_prefix = value.to_owned(),
            "sparse_checkout_paths" => self.settings.sparse_checkout_paths = value.to_owned(),
            "lfs_pattern" => {
                if let Some(pattern) = self.settings.lfs_patterns.last_mut() {
                    *pattern = value.to_owned();
                } else {
                    self.settings.lfs_patterns.push(value.to_owned());
                }
            }
            "profile_name" | "profile_author_name" | "profile_author_email" => {
                if let Some(selected) = self.settings.selected_profile.clone()
                    && let Some(profile) = self
                        .settings
                        .profiles
                        .iter_mut()
                        .find(|profile| profile.name == selected)
                {
                    if key == "profile_name" {
                        profile.name = value.to_owned();
                        self.settings.selected_profile = Some(value.to_owned());
                    } else if key == "profile_author_name" {
                        profile.author_name = value.to_owned();
                    } else {
                        profile.author_email = value.to_owned();
                    }
                }
            }
            _ => return,
        }
        self.persist_settings();
    }

    fn open_external_tool(&mut self, editor: bool) {
        let command = if editor {
            self.settings.external_editor.trim()
        } else {
            self.settings.external_terminal.trim()
        };
        if command.is_empty() {
            self.toast = Some("Configure the external tool in Preferences first".to_owned());
            return;
        }
        let path = self
            .repo_path
            .as_ref()
            .or_else(|| self.snapshot.as_ref().map(|snapshot| &snapshot.path));
        let Some(path) = path else {
            self.toast = Some("Open a repository first".to_owned());
            return;
        };
        let mut parts = command.split_whitespace();
        let Some(program) = parts.next() else { return };
        let mut child = Command::new(program);
        child.args(parts);
        if self.settings.show_external_tool_arguments {
            child.arg(path);
        } else if cfg!(target_os = "macos") && command == "open -a" {
            child.arg(path);
        } else {
            child.arg(path);
        }
        if let Err(error) = child.spawn() {
            self.error = Some(format!("Launch external tool: {error}"));
        }
    }

    /// Resolves a repository-relative path against the open worktree root.
    fn workdir_path(&self, path: &Path) -> PathBuf {
        self.repo_path
            .as_ref()
            .or_else(|| self.snapshot.as_ref().map(|snapshot| &snapshot.path))
            .map_or_else(|| path.to_path_buf(), |root| root.join(path))
    }

    /// Opens a worktree file with the platform handler, or reveals it in the
    /// file manager when `reveal` is set.
    fn open_path_externally(&mut self, path: &Path, reveal: bool) {
        let absolute = self.workdir_path(path);
        let mut command;
        if cfg!(target_os = "macos") {
            command = Command::new("open");
            if reveal {
                command.arg("-R");
            }
            command.arg(&absolute);
        } else {
            command = Command::new("xdg-open");
            command.arg(if reveal {
                absolute
                    .parent()
                    .map_or_else(|| absolute.clone(), Path::to_path_buf)
            } else {
                absolute.clone()
            });
        }
        if let Err(error) = command.spawn() {
            self.error = Some(format!("Open {}: {error}", path.display()));
        }
    }

    /// Launches the configured external editor with one worktree file.
    fn open_file_in_editor(&mut self, path: &Path) {
        let command = self.settings.external_editor.trim();
        if command.is_empty() {
            self.toast = Some("Configure the external editor in Preferences first".to_owned());
            return;
        }
        let absolute = self.workdir_path(path);
        let mut parts = command.split_whitespace();
        let Some(program) = parts.next() else { return };
        let mut child = Command::new(program);
        child.args(parts).arg(absolute);
        if let Err(error) = child.spawn() {
            self.error = Some(format!("Launch external editor: {error}"));
        }
    }

    /// Builds the GitKraken-style context-menu rows for the open file overlay.
    /// Rows that would be dead ends (no blame surface, unconfigured editor)
    /// are omitted entirely rather than rendered disabled.
    pub(crate) fn file_context_rows(&self) -> Vec<(String, UiAction)> {
        let Overlay::FileContext { path, scope } = &self.overlay else {
            return Vec::new();
        };
        let mut rows = Vec::new();
        match scope {
            FileContextScope::Unstaged => {
                rows.push(("Stage file".to_owned(), UiAction::FileContextStage));
                rows.push(("Discard changes".to_owned(), UiAction::FileContextDiscard));
            }
            FileContextScope::Staged => {
                rows.push(("Unstage file".to_owned(), UiAction::FileContextUnstage));
            }
            FileContextScope::Committed(_) => {}
        }
        let uncommitted = !matches!(scope, FileContextScope::Committed(_));
        if uncommitted {
            rows.push(("Stash file".to_owned(), UiAction::FileContextStashFile));
            rows.push((
                "Ignore file".to_owned(),
                UiAction::FileContextIgnore(path.display().to_string()),
            ));
            if let Some(extension) = path.extension().and_then(|extension| extension.to_str()) {
                rows.push((
                    format!("Ignore all .{extension} files"),
                    UiAction::FileContextIgnore(format!("*.{extension}")),
                ));
            }
        }
        rows.push(("File History".to_owned(), UiAction::FileContextHistory));
        if uncommitted
            && let Some(editor) = self
                .settings
                .external_editor
                .split_whitespace()
                .next()
                .filter(|program| !program.is_empty())
        {
            rows.push((format!("Open in {editor}"), UiAction::FileContextOpenEditor));
        }
        rows.push((
            "Open file in default program".to_owned(),
            UiAction::FileContextOpenDefault,
        ));
        rows.push((
            if cfg!(target_os = "macos") {
                "Show in Finder"
            } else {
                "Show in file manager"
            }
            .to_owned(),
            UiAction::FileContextReveal,
        ));
        rows.push(("Copy file path".to_owned(), UiAction::FileContextCopyPath));
        if uncommitted {
            rows.push(("Delete file".to_owned(), UiAction::FileContextDelete));
        }
        rows
    }

    fn toggle_palette(&mut self, overlay: Overlay) {
        if self.overlay == overlay {
            self.close_overlay();
            return;
        }
        self.palette = Some(crate::views::palette::PaletteState::default());
        self.show_popup(overlay, FocusField::Palette);
    }

    fn move_palette(&mut self, delta: i32) {
        let Some(skin) = crate::views::palette::skin(&self.overlay) else {
            return;
        };
        if let Some(palette) = &mut self.palette {
            crate::views::palette::move_cursor(palette, skin, delta);
        }
    }

    fn execute_palette_command(&mut self, index: usize) {
        let Some(skin) = crate::views::palette::skin(&self.overlay) else {
            return;
        };
        let action = self
            .palette
            .as_ref()
            .and_then(|palette| crate::views::palette::action_for(skin, index, &palette.query));
        self.close_overlay();
        if let Some(action) = action {
            self.dispatch(action);
        }
    }

    fn show_popup(&mut self, overlay: Overlay, focus: FocusField) {
        if self.overlay == Overlay::None {
            self.overlay_focus = self.focus;
        }
        self.overlay = overlay;
        self.focus = focus;
    }

    fn toggle_popup(&mut self, overlay: Overlay, focus: FocusField) {
        if self.overlay == overlay {
            self.close_overlay();
        } else {
            self.show_popup(overlay, focus);
        }
    }

    fn close_overlay(&mut self) {
        self.palette = None;
        self.overlay = Overlay::None;
        self.focus = std::mem::take(&mut self.overlay_focus);
    }

    /// Resolves the Add Remote form into a submittable remote, or `None`
    /// while required fields are missing. The remote name falls back to
    /// `origin` when left blank, mirroring the form placeholder.
    pub(crate) fn add_remote_submission(&self) -> Option<RemoteSubmission> {
        let name = match self.add_remote_name.trim() {
            "" => "origin".to_owned(),
            trimmed => trimmed.to_owned(),
        };
        match self.add_remote_provider {
            AddRemoteProvider::Url => {
                let url = self.add_remote_url.trim();
                (!url.is_empty()).then(|| RemoteSubmission {
                    name,
                    url: url.to_owned(),
                    push_url: Some(self.add_remote_push_url.trim())
                        .filter(|push| !push.is_empty())
                        .map(str::to_owned),
                })
            }
            AddRemoteProvider::GitHub => {
                hosted_remote_url("https://github.com", &self.add_remote_repo).map(|url| {
                    RemoteSubmission {
                        name,
                        url,
                        push_url: None,
                    }
                })
            }
            AddRemoteProvider::Gitea => {
                let host = self.add_remote_host.trim().trim_end_matches('/');
                if host.is_empty() {
                    return None;
                }
                let host = if host.contains("://") {
                    host.to_owned()
                } else {
                    format!("https://{host}")
                };
                hosted_remote_url(&host, &self.add_remote_repo).map(|url| RemoteSubmission {
                    name,
                    url,
                    push_url: None,
                })
            }
        }
    }

    /// Declarative spec for the active right-click menu, consumed by both the
    /// drawn overlay renderer and the native macOS presenter.
    pub(crate) fn context_menu(&self) -> Option<MenuSpec> {
        match &self.overlay {
            Overlay::StashContext(index) => {
                let title = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot
                            .stashes
                            .iter()
                            .find(|stash| stash.index == *index)
                            .map(|stash| stash.name.clone())
                    })
                    .unwrap_or_else(|| format!("stash@{{{index}}}"));
                Some(MenuSpec {
                    title,
                    entries: vec![
                        MenuEntry::item("Apply stash", UiAction::StashContextApply),
                        MenuEntry::item("Pop stash", UiAction::StashContextPop),
                        MenuEntry::Separator,
                        MenuEntry::item("Drop stash", UiAction::StashContextDrop),
                    ],
                })
            }
            Overlay::TagContext(tag) => Some(MenuSpec {
                title: tag.clone(),
                entries: vec![
                    MenuEntry::item("Checkout tag", UiAction::TagContextCheckout),
                    MenuEntry::item("Delete tag", UiAction::TagContextDelete),
                    MenuEntry::Separator,
                    MenuEntry::item("Copy tag name", UiAction::TagContextCopyName),
                ],
            }),
            Overlay::BranchContext(branch) => {
                let current = self
                    .snapshot
                    .as_ref()
                    .map_or("current", |snapshot| snapshot.head.as_str());
                Some(MenuSpec {
                    title: branch.clone(),
                    entries: vec![
                        MenuEntry::item("Checkout", UiAction::BranchContextCheckout),
                        MenuEntry::item(
                            format!("Fast-forward {current} to {branch}"),
                            UiAction::BranchContextFastForward,
                        ),
                        MenuEntry::item(
                            format!("Merge {branch} into {current}"),
                            UiAction::BranchContextMerge,
                        ),
                        MenuEntry::item(
                            format!("Rebase {current} onto {branch}"),
                            UiAction::BranchContextRebase,
                        ),
                        MenuEntry::Separator,
                        MenuEntry::item("Rename branch", UiAction::BranchContextRename),
                        MenuEntry::item("Delete branch", UiAction::BranchContextDelete),
                        MenuEntry::Separator,
                        MenuEntry::item("Copy branch name", UiAction::BranchContextCopyName),
                    ],
                })
            }
            Overlay::CommitContext(id) => {
                let head = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.head_id.as_deref())
                    == Some(id.as_str());
                let branch = self
                    .snapshot
                    .as_ref()
                    .map_or("HEAD", |snapshot| snapshot.head.as_str());
                let edit = MenuEntry::Item {
                    label: "Edit commit message".to_owned(),
                    action: UiAction::CommitContextEditMessage,
                    enabled: head,
                };
                Some(MenuSpec {
                    title: id.clone(),
                    entries: vec![
                        MenuEntry::item("Checkout this commit", UiAction::CommitContextCheckout),
                        MenuEntry::item("Create branch here", UiAction::CommitContextCreateBranch),
                        MenuEntry::item("Create tag here", UiAction::CommitContextCreateTag),
                        MenuEntry::Separator,
                        MenuEntry::Submenu {
                            label: format!("Reset {branch} to this commit"),
                            entries: vec![
                                (
                                    "Soft — keep all changes".to_owned(),
                                    UiAction::CommitContextReset("soft".to_owned()),
                                ),
                                (
                                    "Mixed — keep working copy, reset index".to_owned(),
                                    UiAction::CommitContextReset("mixed".to_owned()),
                                ),
                                (
                                    "Hard — discard all changes".to_owned(),
                                    UiAction::CommitContextReset("hard".to_owned()),
                                ),
                            ],
                        },
                        edit,
                        MenuEntry::item("Cherry-pick commit", UiAction::CommitContextCherryPick),
                        MenuEntry::item("Revert commit", UiAction::CommitContextRevert),
                        MenuEntry::Separator,
                        MenuEntry::item("Copy commit SHA", UiAction::CopyCommitSha),
                        MenuEntry::item("Copy commit message", UiAction::CopyCommitMessage),
                        MenuEntry::item(
                            "Create patch from commit",
                            UiAction::CommitContextCreatePatch,
                        ),
                    ],
                })
            }
            Overlay::FileContext { path, .. } => Some(MenuSpec {
                title: path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                ),
                entries: self
                    .file_context_rows()
                    .into_iter()
                    .map(|(label, action)| MenuEntry::item(label, action))
                    .collect(),
            }),
            Overlay::DropMenu {
                source,
                source_tag,
                target,
                target_tag,
            } => {
                let head = self
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.head.as_str());
                let local_target = !*target_tag
                    && self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot
                            .branches
                            .iter()
                            .any(|branch| !branch.remote && branch.name == *target)
                    });
                Some(MenuSpec {
                    title: format!("{source} → {target}"),
                    entries: vec![
                        MenuEntry::Item {
                            label: format!("Merge {source} into {target}"),
                            action: UiAction::DropMerge,
                            enabled: head == Some(target.as_str()) && source != target,
                        },
                        MenuEntry::Item {
                            label: format!("Rebase {source} onto {target}"),
                            action: UiAction::DropRebaseOnto,
                            enabled: !*source_tag && source != target,
                        },
                        MenuEntry::Item {
                            label: format!("Fast-forward {target} to {source}"),
                            action: UiAction::DropFastForward,
                            enabled: local_target && source != target,
                        },
                    ],
                })
            }
            _ => None,
        }
    }

    /// Drawn-layout bounds of the active context menu, including any open
    /// submenu; used for outside-click dismissal.
    fn context_menu_bounds(&self) -> Option<Rect> {
        let spec = self.context_menu()?;
        Some(
            crate::ui::menu::layout(
                &spec,
                self.overlay_anchor,
                [px(self.width), px(self.height)],
                self.mouse,
            )
            .bounds(),
        )
    }

    fn active_popup_rect(&self) -> Option<Rect> {
        let width = px(self.width);
        let height = px(self.height);
        if self.error.is_some() {
            return Some(Rect::new(
                width * 0.5 - 200.0,
                height * 0.5 - 120.0,
                400.0,
                240.0,
            ));
        }

        let layout = crate::views::Layout::for_state(self);
        match &self.overlay {
            Overlay::None => None,
            Overlay::Branches => Some(Rect::new(128.0, 48.0, 320.0, 520.0_f32.min(height - 64.0))),
            Overlay::Lfs => {
                let start = crate::views::shell::action_cluster_start(
                    self.tabs.len(),
                    layout.toolbar.width,
                );
                Some(Rect::new(
                    (start + 238.0).min(layout.toolbar.right() - 252.0),
                    layout.toolbar.bottom() + 4.0,
                    240.0,
                    168.0,
                ))
            }
            Overlay::Actions => Some(Rect::new(
                layout.toolbar.right() - 240.0,
                layout.toolbar.bottom() + 4.0,
                224.0,
                164.0,
            )),
            Overlay::PullOptions => Some(Rect::new(
                crate::views::shell::action_cluster_start(self.tabs.len(), layout.toolbar.width)
                    + 24.0,
                layout.toolbar.bottom() + 4.0,
                310.0,
                190.0,
            )),
            Overlay::CommitOptions => Some(Rect::new(
                self.overlay_anchor[0].clamp(12.0, width - 190.0),
                self.overlay_anchor[1].clamp(12.0, height - 52.0),
                178.0,
                38.0,
            )),
            Overlay::DiffSelection => {
                let menu_height = if self
                    .selected_file
                    .as_ref()
                    .is_some_and(|request| matches!(request.scope, DiffScope::Staged))
                {
                    76.0
                } else {
                    110.0
                };
                Some(Rect::new(
                    self.overlay_anchor[0].clamp(8.0, width - 210.0),
                    self.overlay_anchor[1].clamp(8.0, height - menu_height - 8.0),
                    202.0,
                    menu_height,
                ))
            }
            Overlay::Tabs => Some(Rect::new(width - 320.0, 48.0, 300.0, 240.0)),
            Overlay::Notifications => Some(Rect::new(width - 380.0, 48.0, 360.0, 400.0)),
            Overlay::CreateBranch => Some(Rect::new(
                width * 0.5 - 200.0,
                height * 0.5 - 100.0,
                400.0,
                200.0,
            )),
            Overlay::AddRemote => Some(add_remote_popup_rect(width, height)),
            Overlay::RenameBranch(_) => Some(Rect::new(
                width * 0.5 - 200.0,
                height * 0.5 - 100.0,
                400.0,
                200.0,
            )),
            Overlay::CreateTag(_) => Some(Rect::new(
                width * 0.5 - 200.0,
                height * 0.5 - 128.0,
                400.0,
                256.0,
            )),
            Overlay::StashContext(_)
            | Overlay::TagContext(_)
            | Overlay::BranchContext(_)
            | Overlay::CommitContext(_)
            | Overlay::FileContext { .. }
            | Overlay::DropMenu { .. } => self.context_menu_bounds(),
            Overlay::EditCommitMessage(_) => Some(Rect::new(
                width * 0.5 - 210.0,
                height * 0.5 - 130.0,
                420.0,
                260.0,
            )),
            Overlay::Ai => Some(Rect::new(
                width * 0.5 - 280.0,
                height * 0.5 - 180.0,
                560.0,
                360.0,
            )),
            Overlay::CommandPalette => Some(crate::views::palette::popup_rect(
                self,
                crate::views::palette::PaletteSkin::General,
            )),
            Overlay::EditorPalette => Some(crate::views::palette::popup_rect(
                self,
                crate::views::palette::PaletteSkin::Editor,
            )),
        }
    }

    fn submit_lfs(&mut self, operation: LfsOperation) {
        self.close_overlay();
        self.submit_mutation(GitJobKind::Lfs {
            operation,
            limit: self.requested_limit,
        });
    }

    fn submit_mutation(&mut self, kind: GitJobKind) {
        self.error = None;
        let pending = self
            .undo_candidate(&kind)
            .map_or(PendingMutation::Ignore, PendingMutation::Record);
        self.submit_mutation_pending(kind, pending);
    }

    fn submit_history_mutation(&mut self, kind: GitJobKind, record: UndoRecord, undo: bool) {
        self.error = None;
        self.submit_mutation_pending(kind, PendingMutation::History { record, undo });
    }

    fn submit_mutation_pending(&mut self, kind: GitJobKind, pending: PendingMutation) {
        if self.repo_path.is_none() {
            self.error = Some("No repository is open.".to_owned());
            return;
        }
        if let PendingMutation::History { undo, .. } = &pending {
            self.begin_op(if *undo {
                ToolbarOp::Undo
            } else {
                ToolbarOp::Redo
            });
        }
        self.pending_mutations.push(pending);
        self.submit(kind);
    }

    fn history_job(&self, record: &UndoRecord, undo: bool) -> GitJobKind {
        let limit = self.requested_limit;
        match record {
            UndoRecord::Head {
                before,
                after,
                mode,
                ..
            } => GitJobKind::Reset {
                target: if undo { before.clone() } else { after.clone() },
                mode: (*mode).to_owned(),
                limit,
            },
            UndoRecord::Checkout { previous, target } => GitJobKind::Checkout {
                branch: if undo {
                    previous.clone()
                } else {
                    target.clone()
                },
                limit,
            },
            UndoRecord::BranchCreate {
                branch,
                target,
                previous,
            } if undo => GitJobKind::UndoBranchCreate {
                branch: branch.clone(),
                target: target.clone(),
                previous: previous.clone(),
                limit,
            },
            UndoRecord::BranchCreate { branch, target, .. } => GitJobKind::CreateBranch {
                branch: branch.clone(),
                target: Some(target.clone()),
                limit,
            },
            UndoRecord::BranchDelete { branch, target } if undo => GitJobKind::RestoreBranch {
                branch: branch.clone(),
                target: target.clone(),
                limit,
            },
            UndoRecord::BranchDelete { branch, .. } => GitJobKind::DeleteBranch {
                branch: branch.clone(),
                limit,
            },
        }
    }

    fn undo_candidate(&self, kind: &GitJobKind) -> Option<UndoRecord> {
        let snapshot = self.snapshot.as_ref()?;
        let before_ref = head_target(snapshot)?;
        let before_head = snapshot.head_id.clone()?;
        match kind {
            GitJobKind::Commit { .. } => Some(UndoRecord::Head {
                before: before_head,
                after: String::new(),
                mode: "soft",
                label: "commit",
            }),
            GitJobKind::Merge { .. } => Some(UndoRecord::Head {
                before: before_head,
                after: String::new(),
                mode: "hard",
                label: "merge",
            }),
            GitJobKind::FastForward { .. } => Some(UndoRecord::Head {
                before: before_head,
                after: String::new(),
                mode: "hard",
                label: "fast-forward",
            }),
            GitJobKind::Reset { mode, .. } => Some(UndoRecord::Head {
                before: before_head,
                after: String::new(),
                mode: if mode == "hard" { "hard" } else { "mixed" },
                label: "reset",
            }),
            GitJobKind::Checkout { branch, .. } => Some(UndoRecord::Checkout {
                previous: before_ref,
                target: branch.clone(),
            }),
            GitJobKind::CreateBranch { branch, target, .. } => Some(UndoRecord::BranchCreate {
                branch: branch.clone(),
                target: target.clone().unwrap_or(before_head),
                previous: before_ref,
            }),
            GitJobKind::DeleteBranch { branch, .. } => snapshot
                .branches
                .iter()
                .find(|candidate| !candidate.remote && candidate.name == *branch)
                .map(|candidate| UndoRecord::BranchDelete {
                    branch: branch.clone(),
                    target: candidate.target.clone(),
                }),
            _ => None,
        }
    }

    fn complete_pending_mutation(&mut self, pending: PendingMutation) {
        match pending {
            PendingMutation::Ignore => {}
            PendingMutation::Record(mut record) => {
                if let UndoRecord::Head { after, .. } = &mut record {
                    let Some(head) = self
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.head_id.clone())
                    else {
                        return;
                    };
                    *after = head;
                }
                self.undo_stack.push(record);
                self.redo_stack.clear();
            }
            PendingMutation::History { record, undo } => {
                self.end_op(if undo {
                    ToolbarOp::Undo
                } else {
                    ToolbarOp::Redo
                });
                let label = undo_label(&record);
                if undo {
                    self.redo_stack.push(record);
                    self.toast = Some(format!("Undid {label}"));
                } else {
                    self.undo_stack.push(record);
                    self.toast = Some(format!("Redid {label}"));
                }
            }
        }
    }

    fn restore_failed_history(&mut self, pending: PendingMutation) {
        if let PendingMutation::History { record, undo } = pending {
            self.end_op(if undo {
                ToolbarOp::Undo
            } else {
                ToolbarOp::Redo
            });
            if undo {
                self.undo_stack.push(record);
            } else {
                self.redo_stack.push(record);
            }
        }
    }

    fn submit(&mut self, kind: GitJobKind) {
        let Some(path) = self.repo_path.clone() else {
            self.error = Some("No repository is open.".to_owned());
            return;
        };
        if let Some(op) = toolbar_op(&kind) {
            self.begin_op(op);
        }
        self.busy_jobs = self.busy_jobs.saturating_add(1);
        self.git.submit(GitJob {
            generation: self.generation,
            path,
            kind,
            settings: self.settings.clone(),
        });
    }

    fn submit_clone(&mut self, url: String, destination: PathBuf) {
        self.busy_jobs = self.busy_jobs.saturating_add(1);
        self.git.submit(GitJob {
            generation: self.generation,
            path: destination.clone(),
            kind: GitJobKind::Clone { url, destination },
            settings: self.settings.clone(),
        });
    }
}

fn head_target(snapshot: &RepoSnapshot) -> Option<String> {
    if snapshot.head == "HEAD" {
        snapshot.head_id.clone()
    } else {
        Some(snapshot.head.clone())
    }
}

fn undo_label(record: &UndoRecord) -> &'static str {
    match record {
        UndoRecord::Head { label, .. } => label,
        UndoRecord::Checkout { .. } => "checkout",
        UndoRecord::BranchCreate { .. } => "branch creation",
        UndoRecord::BranchDelete { .. } => "branch deletion",
    }
}

fn is_mutation_job(kind: &GitJobKind) -> bool {
    !matches!(
        kind,
        GitJobKind::LoadSnapshot { .. }
            | GitJobKind::LoadHistory { .. }
            | GitJobKind::LoadDetail { .. }
            | GitJobKind::LoadRangeDetail { .. }
            | GitJobKind::LoadDiff { .. }
    )
}

/// Scroll offset that rests `row` three lines below the top of the diff canvas.
fn seek_scroll(row: usize) -> f32 {
    row.saturating_sub(3).to_f32().unwrap_or(0.0) * crate::views::diff::ROW_HEIGHT
}

fn add_signed<T>(value: T, delta: i32, minimum: T, maximum: T) -> T
where
    T: num_traits::PrimInt + num_traits::NumCast,
{
    let current = num_traits::cast::<T, i64>(value).unwrap_or(0);
    let lower = num_traits::cast::<T, i64>(minimum).unwrap_or(i64::MIN);
    let upper = num_traits::cast::<T, i64>(maximum).unwrap_or(i64::MAX);
    num_traits::cast::<i64, T>((current + i64::from(delta)).clamp(lower, upper)).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    use git2::{Repository, Signature};

    use super::*;
    use crate::{ui::Theme, views};

    fn repository_with_working_file() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("temporary repository");
        let repository = Repository::init(directory.path()).expect("initialize repository");
        let mut config = repository.config().expect("open repository config");
        config
            .set_str("user.name", "Kraken UI Test")
            .expect("set user name");
        config
            .set_str("user.email", "ui@kraken.local")
            .expect("set user email");
        drop(config);
        std::fs::write(directory.path().join("base.txt"), "base\n").expect("write base file");
        let mut index = repository.index().expect("open initial index");
        index
            .add_path(Path::new("base.txt"))
            .expect("add base file");
        index.write().expect("persist initial index");
        let tree_id = index.write_tree().expect("write initial tree");
        let tree = repository.find_tree(tree_id).expect("load initial tree");
        let signature =
            Signature::now("Kraken UI Test", "ui@kraken.local").expect("create signature");
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "feat: initialized UI loop",
                &tree,
                &[],
            )
            .expect("create initial commit");
        drop(tree);
        drop(index);
        let working = PathBuf::from("ui-loop.rs");
        std::fs::write(
            directory.path().join(&working),
            "pub fn verified() -> bool {\n    true\n}\n",
        )
        .expect("write working file");
        (directory, working)
    }

    /// Commits one file on HEAD of an existing test repository.
    fn commit_file(directory: &Path, path: &str, content: &str, message: &str) -> String {
        let repository = Repository::open(directory).expect("open test repository");
        std::fs::write(directory.join(path), content).expect("write committed file");
        let mut index = repository.index().expect("open index");
        index.add_path(Path::new(path)).expect("add committed path");
        index.write().expect("persist index");
        let tree_id = index.write_tree().expect("write tree");
        let tree = repository.find_tree(tree_id).expect("load tree");
        let signature =
            Signature::now("Kraken UI Test", "ui@kraken.local").expect("create signature");
        let parent = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok());
        let parents = parent.iter().collect::<Vec<_>>();
        repository
            .commit(Some("HEAD"), &signature, &signature, message, &tree, &parents)
            .expect("create commit")
            .to_string()
    }

    fn wait_until(state: &mut AppState, condition: impl Fn(&AppState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition(state) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for Git event: {:?}",
                state.error
            );
            thread::sleep(Duration::from_millis(5));
            state.process_events();
        }
    }

    #[test]
    fn menu_commands_dismiss_their_owning_menu() {
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());

        state.overlay = Overlay::Actions;
        state.dispatch(UiAction::Fetch);
        assert_eq!(state.overlay, Overlay::None);

        state.overlay = Overlay::Lfs;
        state.dispatch(UiAction::LfsCheckout);
        assert_eq!(state.overlay, Overlay::None);
    }
    #[test]
    fn popup_click_outside_dismisses_without_dispatching_the_underlying_action() {
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.tabs[0].path = Some(std::env::temp_dir());
        state.focus = FocusField::CommitSummary;
        state.dispatch(UiAction::ToggleActionsMenu);

        let scene = views::build_scene(&state, &Theme::dark());
        state.adopt_scene(&scene);
        let outside = state
            .hits
            .iter()
            .find(|hit| hit.action == UiAction::ToggleBranchMenu)
            .expect("branch menu target remains outside actions popup")
            .rect;
        state.mouse = [
            outside.x + outside.width * 0.5,
            outside.y + outside.height * 0.5,
        ];
        state.click();

        assert_eq!(state.overlay, Overlay::None);
        assert_eq!(state.focus, FocusField::CommitSummary);

        state.dispatch(UiAction::ToggleActionsMenu);
        state.escape();
        assert_eq!(state.overlay, Overlay::None);
        assert_eq!(state.focus, FocusField::CommitSummary);
    }

    #[test]
    fn modifier_clicks_assemble_multi_selection_and_combined_range() {
        let (repository_directory, _) = repository_with_working_file();
        commit_file(repository_directory.path(), "b.txt", "bee\n", "feat: added b");
        commit_file(repository_directory.path(), "c.txt", "sea\n", "feat: added c");
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.commits.len() == 3)
        });
        let ids = state
            .snapshot
            .as_ref()
            .expect("loaded snapshot")
            .commits
            .iter()
            .map(|commit| commit.id.clone())
            .collect::<Vec<_>>();
        // The newest commit was auto-selected with the snapshot.
        assert_eq!(state.selected_commit.as_deref(), Some(ids[0].as_str()));

        // Primary-click the root commit: two selected, endpoints span the graph.
        state.modifier_primary = true;
        state.dispatch(UiAction::SelectCommit(ids[2].clone()));
        state.modifier_primary = false;
        assert_eq!(state.selected_commits.len(), 2);
        assert_eq!(
            state.selection_endpoints(),
            Some((ids[2].clone(), ids[0].clone()))
        );
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.range_detail.is_some()
        });
        let range = state.range_detail.clone().expect("combined range detail");
        assert_eq!(range.oldest, ids[2]);
        assert_eq!(range.newest, ids[0]);
        // The root commit's own changes participate in the union.
        let paths = range
            .files
            .iter()
            .map(|file| file.path.display().to_string())
            .collect::<Vec<_>>();
        assert!(paths.contains(&"base.txt".to_owned()));
        assert!(paths.contains(&"c.txt".to_owned()));

        // Shift-click the newest row keeps the anchor and selects everything
        // in between; primary-click removal shrinks the selection again.
        state.modifier_shift = true;
        state.dispatch(UiAction::SelectCommit(ids[0].clone()));
        state.modifier_shift = false;
        assert_eq!(state.selected_commits.len(), 3);
        state.modifier_primary = true;
        state.dispatch(UiAction::SelectCommit(ids[1].clone()));
        state.modifier_primary = false;
        assert_eq!(state.selected_commits.len(), 2);

        // A plain click collapses back to a single selection.
        state.dispatch(UiAction::SelectCommit(ids[1].clone()));
        assert_eq!(state.selected_commits.len(), 1);
        assert_eq!(state.selected_commit.as_deref(), Some(ids[1].as_str()));
        assert!(state.range_detail.is_none());
    }

    #[test]
    fn reselecting_a_commit_reuses_the_cached_detail_without_a_job() {
        let (repository_directory, _) = repository_with_working_file();
        commit_file(repository_directory.path(), "b.txt", "bee\n", "feat: added b");
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state
                    .detail
                    .as_ref()
                    .zip(state.selected_commit.as_ref())
                    .is_some_and(|(detail, selected)| &detail.id == selected)
        });
        let head = state.selected_commit.clone().expect("selected head");
        let ids = state
            .snapshot
            .as_ref()
            .expect("loaded snapshot")
            .commits
            .iter()
            .map(|commit| commit.id.clone())
            .collect::<Vec<_>>();
        state.dispatch(UiAction::SelectCommit(ids[1].clone()));
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.id == ids[1])
        });

        // Both details are cached now; re-selecting resolves synchronously.
        state.dispatch(UiAction::SelectCommit(head.clone()));
        assert_eq!(state.busy_jobs, 0, "cache hit must not submit a job");
        assert!(
            state
                .detail
                .as_ref()
                .is_some_and(|detail| detail.id == head)
        );
    }

    #[test]
    fn external_worktree_edit_refreshes_wip_without_user_action() {
        let (repository_directory, _) = repository_with_working_file();
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.snapshot.is_some()
        });

        std::fs::write(
            repository_directory.path().join("base.txt"),
            "externally changed\n",
        )
        .expect("edit tracked file outside application");
        wait_until(&mut state, |state| {
            state.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.working.files.iter().any(|file| {
                    file.path == Path::new("base.txt")
                        && file.unstaged == Some(crate::git::models::ChangeKind::Modified)
                })
            })
        });
    }

    #[test]
    fn semantic_ui_actions_stage_diff_and_commit_real_file() {
        let (repository_directory, working) = repository_with_working_file();
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .working
                        .files
                        .iter()
                        .any(|file| file.path == working)
                })
        });

        state.dispatch(UiAction::SelectFile {
            path: working.clone(),
            scope: DiffScope::Unstaged,
        });
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.diff.is_some()
        });
        state.dispatch(UiAction::StageFile(working.clone()));
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state.selected_file.as_ref().is_some_and(|request| {
                    request.path == working && matches!(request.scope, DiffScope::Staged)
                })
                && state.diff.is_some()
        });

        state.dispatch(UiAction::SelectFile {
            path: working.clone(),
            scope: DiffScope::Staged,
        });
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.diff.is_some()
        });
        assert!(state.diff.as_ref().is_some_and(|diff| {
            diff.content
                .as_deref()
                .is_some_and(|content| content.contains("verified"))
                && diff
                    .rows
                    .iter()
                    .any(|row| row.kind == crate::git::models::DiffRowKind::Added)
        }));

        state
            .commit_summary
            .set_text("feat(core): committed through semantic UI");
        state
            .commit_body
            .set_text("Exercised stage, diff, and commit actions.");
        state.mouse = [900.0, 740.0];
        state.adopt_scene(&views::build_scene(&state, &Theme::dark()));
        let action = state
            .hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(state.mouse))
            .map(|hit| hit.action.clone())
            .expect("hover commit action");
        assert_eq!(action, UiAction::Commit);
        state.click();
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state
                    .toast
                    .as_deref()
                    .is_some_and(|toast| toast.starts_with("Created commit"))
        });
        assert!(state.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.working.files.is_empty()
                && snapshot.commits.first().is_some_and(|commit| {
                    commit.subject == "feat(core): committed through semantic UI"
                })
        }));
        let repository = Repository::open(repository_directory.path()).expect("reopen repository");
        let head = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .expect("load UI-created commit");
        assert_eq!(
            head.summary(),
            Some("feat(core): committed through semantic UI")
        );
    }

    #[test]
    fn semantic_line_stage_keeps_file_in_both_working_scopes() {
        let (repository_directory, _working) = repository_with_working_file();
        let path = PathBuf::from("base.txt");
        std::fs::write(
            repository_directory.path().join(&path),
            "first change\nsecond change\n",
        )
        .expect("write two-line worktree change");
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.snapshot.is_some()
        });
        state.dispatch(UiAction::SelectFile {
            path: path.clone(),
            scope: DiffScope::Unstaged,
        });
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.diff.is_some()
        });
        let row = state
            .diff
            .as_ref()
            .and_then(|diff| diff.rows.iter().find(|row| row.new_text == "first change"))
            .expect("first changed row");
        state.dispatch(UiAction::StageDiffLines {
            path: path.clone(),
            lines: vec![crate::git::models::DiffLineSelection {
                old_line: row.old_number,
                new_line: row.new_number,
            }],
        });
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot.working.files.iter().any(|file| {
                        file.path == path && file.staged.is_some() && file.unstaged.is_some()
                    })
                })
        });
        state.dispatch(UiAction::SelectFile {
            path: path.clone(),
            scope: DiffScope::Staged,
        });
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.diff.is_some()
        });
        assert!(state.diff.as_ref().is_some_and(|diff| {
            diff.rows.iter().any(|row| row.new_text == "first change")
                && !diff.rows.iter().any(|row| row.new_text == "second change")
        }));
    }

    #[test]
    fn commit_and_push_commits_before_submitting_push() {
        let (repository_directory, working) = repository_with_working_file();
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.snapshot.is_some()
        });
        state.dispatch(UiAction::StageFile(working));
        wait_until(&mut state, |state| state.busy_jobs == 0);
        state.commit_summary.set_text("feat(core): commit and push");
        state.dispatch(UiAction::CommitAndPush);
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state.snapshot.as_ref().is_some_and(|snapshot| {
                    snapshot
                        .commits
                        .first()
                        .is_some_and(|commit| commit.subject == "feat(core): commit and push")
                })
                && state.error.is_some()
        });
        assert!(
            state.error.is_some(),
            "push without a configured remote must report an error"
        );
    }

    #[test]
    fn search_navigation_and_preferences_persist_real_state() {
        let (repository_directory, _working) = repository_with_working_file();
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let settings_path = settings_directory.path().join("settings.toml");
        let store = SettingsStore::at(&settings_path);
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.snapshot.is_some()
        });

        state.dispatch(UiAction::ToggleSearch);
        state.insert_text("initialized");
        assert_eq!(state.search_results(), [0]);
        state.dispatch(UiAction::NextSearchResult);
        assert_eq!(state.search_cursor, 0);
        assert_eq!(
            state.selected_commit,
            state
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.commits.first())
                .map(|commit| commit.id.clone())
        );

        state.adopt_scene(&views::build_scene(&state, &Theme::dark()));
        let controls = state
            .hits
            .iter()
            .filter(|hit| {
                matches!(
                    hit.action,
                    UiAction::PreviousSearchResult
                        | UiAction::NextSearchResult
                        | UiAction::CloseSearch
                )
            })
            .map(|hit| hit.rect)
            .collect::<Vec<_>>();
        assert_eq!(controls.len(), 3);
        assert!(
            controls
                .windows(2)
                .all(|pair| pair[0].intersection(pair[1]).is_none()),
            "search navigation and close hit regions must not overlap"
        );
        let close = state
            .hits
            .iter()
            .find(|hit| hit.action == UiAction::CloseSearch)
            .map(|hit| hit.rect)
            .expect("close search target");
        state.mouse = [close.x + close.width * 0.5, close.y + close.height * 0.5];
        state.click();
        assert_eq!(state.focus, FocusField::None);
        assert!(state.search.is_empty());
        assert!(state.search_results().is_empty());
        let scene = views::build_scene(&state, &Theme::dark());
        assert!(
            !scene
                .hits
                .iter()
                .any(|hit| hit.action == UiAction::CloseSearch),
            "closed search must remove its overlay controls"
        );

        state.dispatch(UiAction::ToggleSearch);
        state.insert_text("initialized");
        state.escape();
        assert_eq!(state.focus, FocusField::None);
        assert!(state.search.is_empty());

        state.dispatch(UiAction::OpenPreferences);
        state.dispatch(UiAction::TogglePreference("show_commit_sha".to_owned()));
        state.dispatch(UiAction::AdjustPreference {
            key: "initial_commits".to_owned(),
            delta: 1,
        });
        let persisted = SettingsStore::at(settings_path)
            .load()
            .expect("load persisted preferences");
        assert!(!persisted.show_commit_sha);
        assert_eq!(persisted.initial_commits, 600);
    }

    #[test]
    fn commit_undo_redo_restores_head_and_staged_change() {
        let (repository_directory, working) = repository_with_working_file();
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.snapshot.is_some()
        });
        let before = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head_id.clone())
            .expect("initial head");
        state.dispatch(UiAction::StageFile(working));
        wait_until(&mut state, |state| state.busy_jobs == 0);
        state.commit_summary.set_text("feat: reversible commit");
        state.dispatch(UiAction::Commit);
        wait_until(&mut state, |state| state.busy_jobs == 0 && state.can_undo());
        let committed = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head_id.clone())
            .expect("committed head");
        state.dispatch(UiAction::Undo);
        wait_until(&mut state, |state| state.busy_jobs == 0 && state.can_redo());
        let undone = state.snapshot.as_ref().expect("undo snapshot");
        assert_eq!(undone.head_id.as_deref(), Some(before.as_str()));
        assert_eq!(undone.working.staged_count(), 1);
        state.dispatch(UiAction::Redo);
        wait_until(&mut state, |state| state.busy_jobs == 0 && state.can_undo());
        assert_eq!(
            state
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.head_id.as_deref()),
            Some(committed.as_str())
        );
    }
    #[test]
    fn dragging_graph_columns_tracks_effective_dividers() {
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_600, 900, store, Settings::default());
        state.tabs[0].path = Some(std::env::temp_dir());
        state.ref_column_width = 110.0;
        state.graph_column_width = 140.0;
        state.message_column_width = 300.0;
        state.graph.max_lanes = 14;

        let table = crate::views::Layout::for_state(&state).center;
        let before_ref = crate::views::graph::column_layout(&state, table);
        state.drag = Some(ResizeTarget::RefColumn);
        state.drag_to(before_ref.refs.right() + 20.0, 0.0);
        let after_ref = crate::views::graph::column_layout(
            &state,
            crate::views::Layout::for_state(&state).center,
        );
        assert!((after_ref.refs.right() - before_ref.refs.right() - 20.0).abs() < f32::EPSILON);

        state.drag = Some(ResizeTarget::GraphColumn);
        let before_graph = crate::views::graph::column_layout(
            &state,
            crate::views::Layout::for_state(&state).center,
        );
        state.drag_to(before_graph.graph.right() + 100.0, 0.0);
        let after_graph = crate::views::graph::column_layout(
            &state,
            crate::views::Layout::for_state(&state).center,
        );
        assert!(
            (after_graph.graph.right() - before_graph.graph.right() - 100.0).abs() < f32::EPSILON
        );

        state.drag = Some(ResizeTarget::MessageColumn);
        let before_message = crate::views::graph::column_layout(
            &state,
            crate::views::Layout::for_state(&state).center,
        );
        state.drag_to(before_message.message.right() + 100.0, 0.0);
        let after_message = crate::views::graph::column_layout(
            &state,
            crate::views::Layout::for_state(&state).center,
        );
        assert!(
            (after_message.message.right() - before_message.message.right() - 100.0).abs()
                < f32::EPSILON
        );

        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut constrained = AppState::base(1_600, 900, store, Settings::default());
        constrained.tabs[0].path = Some(std::env::temp_dir());
        constrained.selected_commit = Some("selected".to_owned());
        constrained.ref_column_width = 100.0;

        let table = crate::views::Layout::for_state(&constrained).center;
        assert!((table.width - 650.0).abs() < f32::EPSILON);
        let before = crate::views::graph::column_layout(&constrained, table);
        assert!((before.graph.width - 120.0).abs() < f32::EPSILON);
        assert!((before.message.width - 220.0).abs() < f32::EPSILON);

        constrained.dispatch(UiAction::BeginResize(ResizeTarget::GraphColumn));
        let dragging = crate::views::graph::column_layout(
            &constrained,
            crate::views::Layout::for_state(&constrained).center,
        );
        constrained.drag_to(dragging.graph.right() + 60.0, 0.0);
        let after = crate::views::graph::column_layout(
            &constrained,
            crate::views::Layout::for_state(&constrained).center,
        );
        assert!((after.graph.right() - dragging.graph.right() - 60.0).abs() < f32::EPSILON);
        assert!((80.0..220.0).contains(&after.message.width));
        constrained.end_drag();
        let resting = crate::views::graph::column_layout(
            &constrained,
            crate::views::Layout::for_state(&constrained).center,
        );
        assert!((resting.graph.right() - dragging.graph.right() - 60.0).abs() < f32::EPSILON);
        assert!((80.0..220.0).contains(&resting.message.width));
    }

    #[test]
    fn branch_click_selects_tip_and_double_click_checks_out() {
        let (repository_directory, _working) = repository_with_working_file();
        {
            let repository =
                Repository::open(repository_directory.path()).expect("reopen repository");
            let head = repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .expect("head commit");
            repository
                .branch("feature", &head, true)
                .expect("create feature branch");
        }
        let settings_directory = tempfile::tempdir().expect("temporary settings directory");
        let store = SettingsStore::at(settings_directory.path().join("settings.toml"));
        let mut state = AppState::base(1_200, 800, store, Settings::default());
        state.open_repository(repository_directory.path().to_path_buf());
        wait_until(&mut state, |state| {
            state.busy_jobs == 0 && state.snapshot.is_some()
        });
        let original_head = state.snapshot.as_ref().expect("snapshot").head.clone();
        assert_ne!(original_head, "feature");
        let tip = state
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .branches
                    .iter()
                    .find(|branch| branch.name == "feature")
                    .map(|branch| branch.target.clone())
            })
            .expect("feature branch tip");

        // A single click only selects/jumps to the branch tip.
        state.dispatch(UiAction::BranchClick("feature".to_owned()));
        assert_eq!(state.selected_commit.as_deref(), Some(tip.as_str()));
        wait_until(&mut state, |state| state.busy_jobs == 0);
        assert_eq!(
            state.snapshot.as_ref().expect("snapshot").head,
            original_head,
            "single branch click must not check out"
        );

        // Let the pending single click expire, then double-click to check out.
        thread::sleep(Duration::from_millis(500));
        state.dispatch(UiAction::BranchClick("feature".to_owned()));
        state.dispatch(UiAction::BranchClick("feature".to_owned()));
        wait_until(&mut state, |state| {
            state.busy_jobs == 0
                && state
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.head == "feature")
        });
    }
}
