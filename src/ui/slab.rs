use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

use chrono::{DateTime, Local, Utc};
use num_traits::ToPrimitive;
use slab_kernel::{
    dispatch::{Effects, Event},
    flatten::Frame,
};
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

use crate::{
    app::{
        palette,
        state::{AppState, FocusField, MainView, Overlay},
    },
    git::models::{
        ChangeKind, CommitSummary, DiffRow, DiffRowKind, DiffScope, FileChange, RefKind,
        RepoSnapshot, WorkingFile, WorktreeInfo,
    },
    graph::avatars,
    settings::Settings,
    ui::{
        action::{FileContextScope, ResizeTarget, TextFieldTarget, UiAction},
        geometry::{COMMIT_HEADER_HEIGHT, COMMIT_ROW_HEIGHT},
        icons, layout,
        menu::MenuEntry,
    },
};

slab_macro::include_doc!(
    generated,
    "ui/app.slab",
    "Instrument Sans" = "assets/fonts/InstrumentSans.ttf",
    "JetBrainsMono Nerd Font Mono" = "assets/fonts/JetBrainsMonoNerdFontMono-Regular.ttf",
);

use generated::{
    BranchRowsItem, DetailCommitsItem, DetailConflictsItem, DetailFilesItem, DetailParentsItem,
    DiffMapItem, DiffRowsItem, DiffRowsOldMarksItem, DiffRowsOldRunsItem, GraphRowsItem,
    GraphRowsRefsItem, OverlayRowsChildrenItem, OverlayRowsItem, PaletteRowsItem, PrefProfilesItem,
    PreferenceNavItem, PreferenceRowsItem, RecentReposItem, SidebarRailItem, SidebarSectionsItem,
    SidebarSectionsRowsItem, StagedFilesItem, TabsItem, UnstagedFilesItem,
};

const TRANSPARENT: u32 = rgba(0, 0, 0, 0);
const TEXT: u32 = rgba(229, 229, 229, 255);
const MUTED: u32 = rgba(163, 163, 163, 255);
const DIM: u32 = rgba(115, 115, 115, 255);
const GREEN: u32 = rgba(76, 183, 130, 255);
const ORANGE: u32 = rgba(247, 165, 80, 255);
const RED: u32 = rgba(235, 87, 87, 255);
const PURPLE: u32 = rgba(168, 120, 245, 255);
const RED_SOFT: u32 = rgba(58, 22, 24, 255);
const GREEN_SOFT: u32 = rgba(14, 42, 30, 255);
const PURPLE_SOFT: u32 = rgba(42, 33, 70, 255);
const ORANGE_SOFT: u32 = rgba(52, 38, 15, 255);
const PANEL: u32 = rgba(12, 12, 12, 255);
const BORDER_STRONG: u32 = rgba(48, 48, 48, 255);
const BORDER: u32 = rgba(34, 34, 34, 255);
const ACCENT: u32 = rgba(237, 237, 237, 255);
const GRAPH_LANE_ORIGIN: f32 = 24.0;
const GRAPH_LANE_SPACING: f32 = 22.0;
const GRAPH_TRAIL_LIFT_WIDTH: f32 = 32.0;
const GRAPH_TRAIL_SHADOW_WIDTH: f32 = 12.0;
const GRAPH_AVATAR_SIZE: u32 = 64;
const GRAPH_OVERSCAN: usize = 4;
const YELLOW_MARK: u32 = rgba(56, 49, 18, 220);
const YELLOW_MARK_CURRENT: u32 = rgba(86, 75, 22, 245);
const TEXT_SELECTION: u32 = rgba(237, 237, 237, 56);
const RED_INTRALINE: u32 = rgba(235, 87, 87, 64);
const GREEN_INTRALINE: u32 = rgba(76, 183, 130, 64);
const ORANGE_INTRALINE: u32 = rgba(247, 165, 80, 64);
const FILE_ROW_ALT: u32 = rgba(17, 17, 17, 153);
const DIFF_CHAR_WIDTH: f64 = 6.6;
const MAX_INLINE_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_INLINE_DIFF_ROWS: usize = 100_000;
const GRAPH_COLORS: [u32; 10] = [
    rgba(64, 186, 205, 255),
    rgba(106, 130, 232, 255),
    rgba(150, 102, 240, 255),
    rgba(196, 90, 222, 255),
    rgba(235, 86, 148, 255),
    rgba(232, 92, 85, 255),
    rgba(240, 170, 54, 255),
    rgba(222, 196, 80, 255),
    rgba(150, 214, 80, 255),
    rgba(66, 214, 160, 255),
];

const GLOBAL_PREFERENCE_PAGES: &[&str] = &[
    "General",
    "Profiles",
    "SSH",
    "External Tools",
    "Commit Signing",
    "Notifications",
    "Experimental",
    "UI Customization",
    "Editor",
    "In-App Terminal",
];
const REPO_PREFERENCE_PAGES: &[&str] = &["Encoding", "Gitflow", "LFS", "Sparse Checkout"];

/// Native window operation emitted by an authored Slab signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SlabHostCommand {
    Close,
    Minimize,
    ToggleMaximize,
    DragWindow,
}

/// Host-visible result of one event dispatched through the generated document.
pub(crate) struct SlabDispatch {
    pub(crate) effects: Effects,
    pub(crate) host_commands: Vec<SlabHostCommand>,
}

#[derive(PartialEq)]
struct GraphProjectionKey {
    snapshot_revision: u64,
    graph_width: f32,
    ref_width: f32,
    message_width: f32,
    date_width: f32,
    main_view: MainView,
    selected_commit: Option<String>,
    selected_commits: Vec<String>,
    search: String,
    search_cursor: usize,
    show_commit_author: bool,
    show_commit_date: bool,
    show_commit_sha: bool,
}

#[derive(PartialEq)]
struct SidebarProjectionKey {
    snapshot_revision: u64,
    height: f32,
    filter: String,
    collapsed: Vec<String>,
    fractions: [f32; 5],
}

#[derive(PartialEq)]
struct BranchProjectionKey {
    snapshot_revision: u64,
    filter: String,
}

/// Owns the typed Slab document and projects application state into it.
pub(crate) struct SlabDocument {
    pub(crate) doc: generated::Doc,
    pushed_diff_scroll: Option<f32>,
    last_detail_avatar: Option<String>,
    last_detail_avatar_pixels: Vec<u8>,
    graph_projection_key: Option<GraphProjectionKey>,
    sidebar_projection_key: Option<SidebarProjectionKey>,
    branch_projection_key: Option<BranchProjectionKey>,
    graph_avatar_versions: BTreeMap<String, u64>,
}

impl SlabDocument {
    /// Wraps a decoded document whose fonts and renderer resources are registered.
    pub(crate) fn new(doc: generated::Doc) -> Self {
        Self {
            doc,
            pushed_diff_scroll: None,
            last_detail_avatar: None,
            last_detail_avatar_pixels: Vec::new(),
            graph_projection_key: None,
            sidebar_projection_key: None,
            branch_projection_key: None,
            graph_avatar_versions: BTreeMap::new(),
        }
    }

    /// Dispatches one real host event through Slab and translates every signal.
    pub(crate) fn dispatch(&mut self, state: &mut AppState, event: &Event) -> SlabDispatch {
        let (effects, signals) = self.doc.dispatch(event);
        for scroll in &effects.scrolls {
            // Kernel keys are full instantiation paths ("#app/.../#diff-scroll");
            // state matches the authored leaf id.
            let leaf = scroll.key.rsplit('/').next().unwrap_or(&scroll.key);
            let leaf = leaf.strip_prefix('#').unwrap_or(leaf);
            state.apply_slab_scroll(leaf, scroll.axis, scroll.off);
        }
        let diff_scroll_x = self.doc.get_scroll("diff-scroll", 1);
        let mut host_commands = Vec::new();
        for signal in signals {
            dispatch_signal(state, signal, &mut host_commands, diff_scroll_x);
        }
        // The kernel divider has no begin/end signals: `resize_to` arms
        // `state.drag` on the first live resize; release it with the pointer
        // so column_layout leaves its explicit-drag floors (HEAD end_drag).
        if matches!(
            event.etype,
            slab_kernel::dispatch::E_POINTER_UP | slab_kernel::dispatch::E_POINTER_DOWN
        ) {
            state.end_column_drag();
        }
        SlabDispatch {
            effects,
            host_commands,
        }
    }

    /// Returns the selected text in the currently focused Slab editor.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let focused = self.doc.inst.ds.fs.focus;
        let index = slab_kernel::dispatch::ed_ix(&self.doc.inst.ds, focused);
        let edit = self.doc.inst.ds.ed.get(usize::try_from(index).ok()?)?;
        let lo = slab_kernel::edit::sel_lo(edit);
        let hi = slab_kernel::edit::sel_hi(edit);
        (lo < hi).then(|| slab_kernel::rt::str_slice(&edit.text, lo, hi))
    }

    /// Synchronizes every visible surface and solves one owned paint frame.
    pub(crate) fn frame(&mut self, state: &AppState) -> Frame {
        self.sync(state);
        self.doc.frame(f64::from(state.animation_time()) * 1_000.0)
    }

    /// Updates a retained paint frame only when synchronized state changes output.
    pub(crate) fn update_frame(&mut self, state: &AppState, frame: &mut Frame) -> bool {
        self.sync(state);
        slab_kernel::frame::inst_frame_update(
            &mut self.doc.inst,
            f64::from(state.animation_time()) * 1_000.0,
            frame,
        )
    }

    fn sync(&mut self, state: &AppState) {
        self.sync_scalars(state);
        self.sync_lists(state);
        self.sync_scroll(state);
        self.doc
            .set_env(f64::from(state.width), f64::from(state.height), true, false);
    }

    fn sync_scalars(&mut self, state: &AppState) {
        let welcome = state
            .tabs
            .get(state.active_tab)
            .is_some_and(|tab| tab.path.is_none());
        let layout = layout::Layout::for_state(state);
        let wip_detail = layout.detail.is_some() && layout::detail_shows_wip(state);
        let multi_detail = layout.detail.is_some()
            && !wip_detail
            && state.selected_commits.len() > 1
            && state.range_detail.is_some();
        let commit_detail = layout.detail.is_some() && !wip_detail && !multi_detail;
        let snapshot = state.snapshot.as_ref();
        let working_count = snapshot.map_or(0, |snapshot| snapshot.working.files.len());
        let search_results = state.search_results();
        let diff_search_results = state.diff_search_results();
        let selected_file = state.selected_file.as_ref();
        let diff = state.diff.as_ref();

        self.doc.set_show_preferences(state.preferences_open);
        self.doc.set_show_welcome(welcome);
        self.doc.set_show_workspace(!welcome);
        self.doc
            .set_show_graph(matches!(state.main_view, MainView::Graph | MainView::Wip));
        self.doc.set_show_diff(state.main_view == MainView::Diff);
        self.doc.set_show_terminal(state.terminal_open && !welcome);
        self.doc.set_show_detail(layout.detail.is_some());
        self.doc.set_show_wip_detail(wip_detail);
        self.doc.set_show_commit_detail(commit_detail);
        self.doc.set_show_multi_detail(multi_detail);
        self.doc
            .set_sidebar_collapsed(state.settings.sidebar_collapsed);
        self.doc.set_sidebar_width(f64::from(layout.sidebar.width));
        self.doc
            .set_detail_width(f64::from(layout.detail.map_or(0.0, |rect| rect.width)));
        self.doc
            .set_terminal_height(f64::from(layout.terminal.map_or(0.0, |rect| rect.height)));
        let width = f64::from(state.width);
        let height = f64::from(state.height);
        let rail = f64::from(layout::SIDEBAR_RAIL_WIDTH);
        let (sidebar_min, sidebar_max) = if state.settings.sidebar_collapsed {
            (rail, rail)
        } else {
            (150.0, (width - 320.0).max(150.0))
        };
        let sidebar_width = f64::from(layout.sidebar.width);

        let detail_min = 200.0;
        let detail_max = (width - sidebar_width - 320.0).max(detail_min);

        let content_height = (height - 44.0 - 22.0).max(0.0);
        let font_size = f64::from(state.settings.terminal_font_size.max(8));
        let terminal_min = font_size * 1.2 * 3.0 + 24.0;
        let terminal_max = (content_height * 0.8).max(terminal_min);

        self.doc.set_sidebar_min(sidebar_min);
        self.doc.set_sidebar_max(sidebar_max);
        self.doc.set_detail_min(detail_min);
        self.doc.set_detail_max(detail_max);
        self.doc.set_terminal_min(terminal_min);
        self.doc.set_terminal_max(terminal_max);
        let graph_columns = layout::column_layout(state, layout.center);
        let ref_width = graph_columns.refs.width;
        let graph_width = graph_columns.graph.width;
        let message_width = graph_columns.message.width;
        let sha_width = graph_columns.sha.width;
        let date_width = graph_columns.date.width;
        self.doc.set_ref_width(f64::from(ref_width));
        self.doc.set_graph_width(f64::from(graph_width));
        self.doc.set_message_width(f64::from(message_width));
        self.doc.set_date_width(f64::from(date_width));
        self.doc.set_sha_width(f64::from(sha_width));
        self.doc
            .set_ref_resize_width(f64::from((ref_width - 4.0).max(0.0)));
        self.doc
            .set_graph_resize_width(f64::from((graph_width - 4.0).max(0.0)));
        self.doc
            .set_message_resize_width(f64::from((message_width - 4.0).max(0.0)));
        // HEAD's drag maxima (ui/layout.rs resize_preference) depend on the
        // live table budget; publish them so the kernel clamps a drag exactly
        // where HEAD's drag_to did. `resize_preference` with an infinite edge
        // collapses to its per-column maximum.
        let column_max = |target| {
            let maximum = layout::resize_preference(state, layout.center, target, f32::INFINITY);
            f64::from((maximum - 4.0).max(0.0))
        };
        self.doc
            .set_ref_resize_max(column_max(ResizeTarget::RefColumn));
        self.doc
            .set_graph_resize_max(column_max(ResizeTarget::GraphColumn));
        self.doc
            .set_message_resize_max(column_max(ResizeTarget::MessageColumn));
        // A pointer drag leaves a sticky per-divider extent overlay in the
        // kernel that overrides the spacer params above. Mirror the
        // state-owned column widths back into the overlays every frame so the
        // header dividers track the same clamped widths the body rows render
        // (window resizes and column_layout's floors included). Bare "#id"
        // keys resolve via node_by_key's final-segment fallback.
        for (key, width) in [
            ("#graph-ref-divider", ref_width),
            ("#graph-lane-divider", graph_width),
            ("#graph-message-divider", message_width),
        ] {
            self.doc.set_divider(key, f64::from((width - 4.0).max(0.0)));
        }

        self.doc
            .set_repo_name(snapshot.map_or("Kraken", |snapshot| &snapshot.name));
        self.doc
            .set_branch_name(snapshot.map_or("No repository", |snapshot| &snapshot.head));
        self.doc.set_commit_count(&format!(
            "Viewing {} commits",
            snapshot.map_or(0, |snapshot| snapshot.commits.len())
        ));
        self.doc.set_status_version(env!("CARGO_PKG_VERSION"));
        self.doc.set_status_commits(&snapshot.map_or_else(
            || "Opening repository".to_owned(),
            |snapshot| format!("{} commits loaded", snapshot.commits.len()),
        ));
        self.doc.set_graph_header_count(&snapshot.map_or_else(
            || "0".to_owned(),
            |snapshot| snapshot.commits.len().to_string(),
        ));
        let selected_file_name = selected_file
            .map(|request| request.path.display().to_string())
            .unwrap_or_default();
        self.doc.set_selected_file_name(&selected_file_name);
        self.doc
            .set_selected_file_encoding(&state.settings.default_encoding);
        self.doc.set_diff_scope(
            selected_file.map_or("DIFF", |request| match &request.scope {
                DiffScope::Commit(_) => "COMMIT",
                DiffScope::CommitRange { .. } => "RANGE",
                DiffScope::Staged => "STAGED",
                DiffScope::Unstaged => "UNSTAGED",
            }),
        );
        self.doc
            .set_diff_old_label(diff.map_or("", |document| &document.old_label));
        self.doc
            .set_diff_new_label(diff.map_or("", |document| &document.new_label));
        self.doc.set_diff_split(state.diff_split);
        self.doc.set_diff_file_view(state.diff_file_view);
        self.doc.set_diff_history(state.file_history);
        self.doc
            .set_diff_search_open(state.focus == FocusField::DiffSearch);
        self.doc.set_diff_search(state.diff_search.text());
        self.doc
            .set_diff_search_count(&match diff_search_results.len() {
                0 => String::new(),
                count => format!(
                    "{} / {count}",
                    state.diff_search_cursor.saturating_add(1).min(count)
                ),
            });
        let diff_viewport_width =
            layout.center.width - if state.diff_file_view { 0.0 } else { 20.0 };
        self.sync_diff_scalars(state, diff_viewport_width.max(0.0));

        self.sync_detail_scalars(state);
        self.doc.set_wip_title("Working Tree");
        self.doc.set_wip_subtitle(&match working_count {
            0 => "No file changes".to_owned(),
            1 => "1 changed file".to_owned(),
            count => format!("{count} changed files"),
        });
        let unstaged_count = snapshot.map_or(0, |snapshot| snapshot.working.unstaged_count());
        let staged_count = snapshot.map_or(0, |snapshot| snapshot.working.staged_count());
        self.doc.set_unstaged_count(&unstaged_count.to_string());
        self.doc.set_staged_count(&staged_count.to_string());
        self.doc
            .set_wip_stage_label(&match state.selected_working_files.len() {
                count if count > 1 => format!("Stage {count} Files"),
                _ => "Stage All Changes".to_owned(),
            });
        self.doc.set_wip_unstaged_empty(unstaged_count == 0);
        self.doc.set_wip_staged_empty(staged_count == 0);
        self.doc.set_commit_summary(state.commit_summary.text());
        self.doc.set_commit_body(state.commit_body.text());
        self.doc.set_commit_amend(state.amend);
        self.doc.set_can_commit(
            snapshot.is_some_and(|snapshot| snapshot.working.staged_count() > 0)
                && !state.commit_summary.trim().is_empty()
                && state.busy_jobs == 0,
        );

        self.doc
            .set_search_open(state.focus == FocusField::Search || !state.search.text().is_empty());
        self.doc.set_search_text(state.search.text());
        self.doc
            .set_search_count(&format!("{} RESULTS", search_results.len()));
        self.doc.set_branch_filter(state.branch_filter.text());
        self.doc
            .set_branch_filter_empty(state.branch_filter.text().is_empty());
        self.doc
            .set_branch_filter_focused(state.focus == FocusField::BranchFilter);
        self.doc.set_welcome_search(state.welcome_search.text());
        self.doc.set_clone_open(state.clone_form);
        self.doc.set_clone_url(state.clone_url.text());
        let clone_destination = state.clone_destination.as_ref().map_or_else(
            || "Choose a destination".to_owned(),
            |path| path.display().to_string(),
        );
        self.doc.set_clone_destination(&clone_destination);
        self.doc.set_path_tree(state.path_tree);
        self.doc.set_view_all_files(state.view_all_files);

        self.sync_preference_scalars(state);
        self.sync_overlay_scalars(state);
        self.doc.set_toast_open(state.toast.is_some());
        self.doc
            .set_toast_text(state.toast.as_deref().unwrap_or_default());
        self.doc.set_error_open(state.error.is_some());
        self.doc
            .set_error_text(state.error.as_deref().unwrap_or_default());
        self.doc.set_ai_open(state.overlay == Overlay::Ai);
        self.doc.set_ai_loading(state.ai_loading);
        self.doc.set_ai_text(
            state
                .ai_message
                .as_deref()
                .unwrap_or("Ask Kraken AI about this repository."),
        );
    }

    fn sync_diff_scalars(&mut self, state: &AppState, viewport_width: f32) {
        let (has_rows, empty_message) = diff_render_status(state);
        self.doc.set_diff_has_rows(has_rows);
        self.doc.set_diff_show_labels(has_rows);
        self.doc.set_diff_empty_message(empty_message);
        self.doc
            .set_diff_content_width(diff_content_width(state, viewport_width));

        let (action_visible, action_label) =
            state
                .selected_file
                .as_ref()
                .map_or((false, "Stage File"), |request| match request.scope {
                    DiffScope::Staged => (true, "Unstage File"),
                    DiffScope::Unstaged => (true, "Stage File"),
                    DiffScope::Commit(_) | DiffScope::CommitRange { .. } => (false, ""),
                });
        self.doc.set_diff_file_action_visible(action_visible);
        self.doc.set_diff_file_action_label(action_label);
    }

    fn sync_detail_scalars(&mut self, state: &AppState) {
        let selected_count = state.selected_commits.len();
        self.doc
            .set_detail_selection_count(&format!("{selected_count} commits selected"));
        self.doc
            .set_detail_selection_short(&format!("{selected_count} selected"));
        let overflow = selected_count.saturating_sub(8);
        self.doc.set_detail_overflow(
            &(overflow > 0)
                .then(|| format!("… and {overflow} more"))
                .unwrap_or_default(),
        );
        // HEAD build_multi: 18 + title 30 + range 22 + rows*18 (+ overflow 18)
        // + 15 below the 39px header = 85 + rows*18.
        self.doc.set_detail_multi_height(
            85.0 + selected_count.min(8).to_f64().unwrap_or(0.0) * 18.0
                + if selected_count > 8 { 18.0 } else { 0.0 },
        );

        if let Some(range) = state.range_detail.as_ref().filter(|_| selected_count > 1) {
            self.doc.set_detail_loading(false);
            self.doc.set_detail_title("Selected commit range");
            self.doc.set_detail_body("");
            self.doc.set_detail_sha("");
            self.doc.set_detail_author("");
            self.doc.set_detail_email("");
            self.doc.set_detail_time("");
            self.doc.set_detail_has_body(false);
            self.doc.set_detail_has_conflicts(false);
            self.doc.set_detail_ai_label("Explain selection");
            self.doc.set_detail_message_height(110.0);
            self.doc
                .set_detail_range(&format!("{} … {}", range.oldest_short, range.newest_short));
            self.set_detail_stats(&range.files);
            self.sync_detail_avatar(None);
            return;
        }

        let Some(detail) = &state.detail else {
            self.doc.set_detail_loading(true);
            self.doc.set_detail_title("Select a commit");
            self.doc.set_detail_body("");
            self.doc.set_detail_sha("");
            self.doc.set_detail_author("");
            self.doc.set_detail_email("");
            self.doc.set_detail_time("");
            self.doc.set_detail_has_body(false);
            self.doc.set_detail_has_conflicts(false);
            self.doc.set_detail_ai_label("Explain commit");
            self.doc.set_detail_message_height(120.0);
            self.doc.set_detail_range("");
            self.doc.set_detail_modified("");
            self.doc.set_detail_added("");
            self.sync_detail_avatar(None);
            return;
        };

        self.doc.set_detail_loading(false);
        self.doc.set_detail_title(&detail.subject);
        self.doc.set_detail_body(&detail.body);
        self.doc.set_detail_sha(&detail.short_id);
        self.doc.set_detail_author(&detail.author);
        self.doc.set_detail_email(&detail.email);
        self.doc.set_detail_time(&format!(
            "AUTHORED {}",
            format_timestamp(detail.authored_seconds).to_uppercase()
        ));
        self.doc.set_detail_has_body(!detail.body.is_empty());
        self.doc
            .set_detail_has_conflicts(!detail.conflicts.is_empty());
        self.doc
            .set_detail_ai_label(if state.commit_is_local(&detail.id) {
                "Recompose with AI"
            } else {
                "Explain commit"
            });
        let body_height = detail
            .body
            .lines()
            .count()
            .clamp(1, 10)
            .to_f32()
            .unwrap_or(1.0)
            * 18.0;
        let conflict_height = detail.conflicts.len().min(5).to_f32().unwrap_or(0.0) * 18.0;
        // HEAD's auto height (commit_detail::build): message_bg = 39 header
        // + 18 + 55 subject + body (lines*18 + 18) + conflicts (32 + n*18)
        // + 15, with the resize strip inside the block. The slab content col
        // excludes the 40px header row and the 14px divider strip, so its
        // auto height is 18 + 55 + body + conflicts + 15 - 14 = 74 + rest.
        let automatic_height =
            74.0 + if detail.body.is_empty() {
                0.0
            } else {
                body_height + 18.0
            } + if detail.conflicts.is_empty() {
                0.0
            } else {
                conflict_height + 32.0
            };
        // Stored detail_message_height keeps HEAD's meaning: block extent
        // from the panel top (header + message + strip). HEAD clamps it to
        // (110, panel_height * 0.7); the content col is 54 shorter.
        let authored_height = if state.detail_message_height > 0.0 {
            state.detail_message_height - 54.0
        } else {
            automatic_height
        };
        self.doc
            .set_detail_message_height(f64::from(authored_height.clamp(
                56.0,
                (state.height.to_f32().unwrap_or(f32::MAX) * 0.7 - 103.0).max(56.0),
            )));
        self.doc.set_detail_range("");
        self.set_detail_stats(&detail.files);
        self.sync_detail_avatar(Some(&detail.email));
    }

    fn set_detail_stats(&mut self, files: &[FileChange]) {
        let (modified, added) = detail_counts(files);
        self.doc
            .set_detail_modified(&format!("{modified} MODIFIED"));
        self.doc.set_detail_added(&format!("+ {added} ADDED"));
    }

    fn sync_detail_avatar(&mut self, email: Option<&str>) {
        const EMPTY: [u8; 4] = [0, 0, 0, 0];
        let (name, width, height, pixels): (String, u32, u32, Cow<'_, [u8]>) =
            if let Some(email) = email {
                let key = avatars::request(email);
                (
                    format!("detail-avatar:{key}"),
                    64,
                    64,
                    Cow::Owned(avatars::pixels(&key)),
                )
            } else {
                (
                    "detail-avatar:empty".to_owned(),
                    1,
                    1,
                    Cow::Borrowed(&EMPTY),
                )
            };
        if self.last_detail_avatar.as_deref() == Some(&name)
            && self.last_detail_avatar_pixels.as_slice() == pixels.as_ref()
        {
            return;
        }
        let _ = self
            .doc
            .img_register(&name, width, height, 1, pixels.as_ref());
        self.doc.set_detail_avatar(&name);
        self.last_detail_avatar = Some(name);
        self.last_detail_avatar_pixels = pixels.into_owned();
    }

    fn sync_preference_scalars(&mut self, state: &AppState) {
        let all_pages = GLOBAL_PREFERENCE_PAGES
            .iter()
            .chain(REPO_PREFERENCE_PAGES.iter())
            .copied()
            .collect::<Vec<_>>();
        let page_number = all_pages
            .iter()
            .position(|page| *page == state.preference_page)
            .unwrap_or(0)
            .saturating_add(1);
        self.doc.set_preference_page(&state.preference_page);
        self.doc.set_preference_eyebrow(&format!(
            "{page_number:02} · {}",
            state.preference_page.to_uppercase()
        ));
        self.doc.set_preference_repo(&format!(
            "REPO: {}",
            state
                .snapshot
                .as_ref()
                .map_or("NO REPOSITORY", |snapshot| snapshot.name.as_str())
                .to_uppercase()
        ));
    }

    fn sync_overlay_scalars(&mut self, state: &AppState) {
        let palette_skin = palette::skin(&state.overlay);
        self.doc.set_palette_open(palette_skin.is_some());
        self.doc
            .set_palette_editor(palette_skin == Some(palette::PaletteSkin::Editor));
        self.doc.set_palette_query(
            state
                .palette
                .as_ref()
                .map_or("", |palette| palette.query.text()),
        );
        self.doc
            .set_palette_hint(if palette_skin == Some(palette::PaletteSkin::Editor) {
                "Search file and diff commands"
            } else {
                "Search repository commands"
            });

        let tabs_overlay = state.overlay == Overlay::Tabs;
        let notifications_overlay = state.overlay == Overlay::Notifications;
        let add_remote_open = matches!(state.overlay, Overlay::AddRemote);
        let generic = !matches!(
            state.overlay,
            Overlay::None
                | Overlay::Ai
                | Overlay::Branches
                | Overlay::CommandPalette
                | Overlay::EditorPalette
                | Overlay::Tabs
                | Overlay::Notifications
                | Overlay::AddRemote
        );
        self.doc.set_tabs_overlay(tabs_overlay);
        self.doc.set_notifications_overlay(notifications_overlay);
        self.doc.set_add_remote_open(add_remote_open);
        self.doc
            .set_branches_overlay(state.overlay == Overlay::Branches);
        self.doc.set_tab_filter(state.tab_filter.text());
        let overlay_modal = matches!(
            state.overlay,
            Overlay::CreateBranch
                | Overlay::RenameBranch(_)
                | Overlay::CreateTag(_)
                | Overlay::EditCommitMessage(_)
        );
        self.doc.set_overlay_open(generic);
        self.doc.set_overlay_modal(overlay_modal);

        let (fallback_target, overlay_gravity_end) = match state.overlay {
            Overlay::Lfs => ("#app/#shell/#topbar/#bar/row@3/#lfs-anchor", true),
            Overlay::PullOptions => ("#app/#shell/#topbar/#bar/row@3/#pull-anchor", false),
            Overlay::Actions => ("#app/#shell/#topbar/#bar/row@4/#actions-anchor", true),
            Overlay::Tabs => ("#app/#shell/#topbar/#bar/row@4/#tabs-anchor", true),
            Overlay::Notifications => {
                ("#app/#shell/#topbar/#bar/row@4/#notifications-anchor", true)
            }
            _ => ("", false),
        };
        let overlay_target = if state.overlay_target.is_empty() {
            fallback_target
        } else {
            state.overlay_target.as_str()
        };
        self.doc.set_overlay_target(overlay_target);
        self.doc.set_overlay_gravity_end(overlay_gravity_end);
        let overlay_has_target = !overlay_target.is_empty();
        self.doc.set_overlay_has_target(overlay_has_target);
        self.doc
            .set_overlay_fallback(!overlay_has_target && !overlay_modal);
        let (overlay_x, overlay_y) = (
            f64::from(state.overlay_anchor[0].max(8.0)),
            f64::from(state.overlay_anchor[1].max(48.0)),
        );
        self.doc.set_overlay_x(overlay_x);
        self.doc.set_overlay_y(overlay_y);

        let (title, subtitle, width, height) = overlay_header(state);
        self.doc.set_overlay_title(title.as_ref());
        self.doc.set_overlay_subtitle(subtitle);
        self.doc.set_overlay_width(width);
        self.doc.set_overlay_height(height);

        let (one, two, three, primary, secondary) = overlay_fields(state);
        self.doc.set_overlay_field_one(one.unwrap_or_default());
        self.doc.set_overlay_field_two(two.unwrap_or_default());
        self.doc.set_overlay_field_three(three.unwrap_or_default());
        self.doc.set_overlay_field_one_visible(one.is_some());
        self.doc.set_overlay_field_two_visible(two.is_some());
        self.doc.set_overlay_field_three_visible(three.is_some());
        self.doc
            .set_overlay_primary_label(primary.unwrap_or("Done"));
        self.doc.set_overlay_primary_visible(primary.is_some());
        self.doc
            .set_overlay_secondary_label(secondary.unwrap_or("Cancel"));
        self.doc.set_overlay_secondary_visible(secondary.is_some());
        self.doc
            .set_overlay_footer_visible(primary.is_some() || secondary.is_some());

        self.doc.set_add_remote_is_url(matches!(
            state.add_remote_provider,
            crate::ui::action::AddRemoteProvider::Url
        ));
        self.doc.set_add_remote_is_hosted(matches!(
            state.add_remote_provider,
            crate::ui::action::AddRemoteProvider::GitHub
        ));
        self.doc.set_add_remote_is_gitea(matches!(
            state.add_remote_provider,
            crate::ui::action::AddRemoteProvider::Gitea
        ));
        self.doc.set_add_remote_name(state.add_remote_name.text());
        self.doc.set_add_remote_url(state.add_remote_url.text());
        self.doc
            .set_add_remote_push_url(state.add_remote_push_url.text());
        self.doc.set_add_remote_repo(state.add_remote_repo.text());
        self.doc.set_add_remote_host(state.add_remote_host.text());
    }

    fn sync_preference_values(&mut self, state: &AppState) {
        let profiles = state.preference_page == "Profiles";
        self.doc.set_pref_is_profiles_page(profiles);
        self.doc.set_pref_is_not_profiles_page(!profiles);
    }

    fn sync_lists(&mut self, state: &AppState) {
        self.doc.set_tabs(&tabs(state));
        let welcome = state
            .tabs
            .get(state.active_tab)
            .is_some_and(|tab| tab.path.is_none());
        let workspace_visible = !welcome && !state.preferences_open;

        if workspace_visible {
            let sidebar_key = sidebar_projection_key(state);
            if self.sidebar_projection_key.as_ref() != Some(&sidebar_key) {
                self.doc.set_sidebar_sections(&sidebar_sections(state));
                self.doc.set_sidebar_rail(&sidebar_rail(state));
                self.sidebar_projection_key = Some(sidebar_key);
            }
            self.doc.set_sidebar_loading(state.snapshot.is_none());
        }

        if workspace_visible && matches!(state.main_view, MainView::Graph | MainView::Wip) {
            let graph_key = graph_projection_key(state);
            if self.graph_projection_key.as_ref() != Some(&graph_key) {
                self.doc.set_graph_rows(&graph_rows(state));
                self.graph_projection_key = Some(graph_key);
            }
            self.sync_graph_avatars(state);
        }

        let layout = layout::Layout::for_state(state);
        let detail = layout.detail.unwrap_or_default();
        if workspace_visible && layout.detail.is_some() && layout::detail_shows_wip(state) {
            let section = layout::wip_section_layout(state, detail);
            let header_height = f64::from(layout::WIP_HEADER_HEIGHT);
            self.doc
                .set_wip_unstaged_height(f64::from(section.unstaged_view.height) + header_height);
            self.doc
                .set_wip_staged_height(f64::from(section.staged_view.height) + header_height);
            let (unstaged, staged) = working_rows(state);
            self.doc.set_unstaged_files(&unstaged);
            self.doc.set_staged_files(&staged);
        }
        if workspace_visible && layout.detail.is_some() && !layout::detail_shows_wip(state) {
            self.doc.set_detail_files(&detail_rows(state));
            self.doc.set_detail_parents(&detail_parent_rows(state));
            self.doc.set_detail_conflicts(&detail_conflict_rows(state));
            self.doc.set_detail_commits(&detail_commit_rows(state));
        }
        if workspace_visible && state.main_view == MainView::Diff {
            self.doc.set_diff_rows(&diff_rows(state));
            self.doc
                .set_diff_map(&diff_map_rows(state, layout.center.height));
        }
        if welcome {
            self.doc.set_recent_repos(&recent_rows(state));
        }
        if state.preferences_open {
            self.sync_preference_values(state);
            self.doc.set_preference_nav(&preference_nav(state));
            self.doc.set_pref_profiles(&pref_profiles(state));
            self.doc
                .set_preference_rows(&preference_rows(&state.preference_page, &state.settings));
        }
        if state.overlay != Overlay::None && state.overlay != Overlay::Branches {
            self.doc.set_overlay_rows(&overlay_rows(state));
        }
        if state.overlay == Overlay::Branches {
            let branch_key = branch_projection_key(state);
            if self.branch_projection_key.as_ref() != Some(&branch_key) {
                let branch_rows = branch_menu_rows(state);
                self.doc.set_branch_rows_empty(branch_rows.is_empty());
                self.doc.set_branch_rows(&branch_rows);
                self.branch_projection_key = Some(branch_key);
            }
        }
        if palette::skin(&state.overlay).is_some() {
            self.doc.set_palette_rows(&palette_rows(state));
        }
    }

    fn sync_graph_avatars(&mut self, state: &AppState) {
        if !state.settings.show_commit_author {
            return;
        }
        let Some(snapshot) = state.snapshot.as_ref() else {
            return;
        };
        let layout = layout::Layout::for_state(state);
        let wip_rows = snapshot.wip_rows();
        let avatar_window = graph_virtual_window(
            state,
            layout.center.height,
            snapshot.commits.len().saturating_add(wip_rows),
        );
        let start = avatar_window
            .start
            .saturating_sub(wip_rows)
            .min(snapshot.commits.len());
        let end = avatar_window
            .end
            .saturating_sub(wip_rows)
            .min(snapshot.commits.len());
        for commit in &snapshot.commits[start..end] {
            if commit.parents.len() >= 2 {
                continue;
            }
            let key = avatars::request(&commit.email);
            let (version, pixels) = avatars::versioned_pixels(&key);
            if self.graph_avatar_versions.get(&key) == Some(&version) {
                continue;
            }
            let name = format!("graph-avatar:{key}");
            let _ = self
                .doc
                .img_register(&name, GRAPH_AVATAR_SIZE, GRAPH_AVATAR_SIZE, 1, &pixels);
            self.graph_avatar_versions.insert(key, version);
        }
    }

    /// Pushes state-owned programmatic diff scrolling (initial hunk seek,
    /// PreviousHunk/NextHunk, SeekDiffRow, animated search seeks) into the
    /// kernel's `diff-scroll` node. Kernel-side wheel scrolling flows the
    /// other way through `apply_slab_scroll`, so state and kernel agree and
    /// this write is a no-op unless `AppState` moved the offset itself.
    /// `set_scroll` retains pre-solve writes, so the seek that lands with a
    /// fresh `GitPayload::Diff` applies on the first frame the rows exist.
    fn sync_scroll(&mut self, state: &AppState) {
        let target = state.diff_scroll;
        if self.pushed_diff_scroll != Some(target) {
            self.doc.set_scroll("diff-scroll", 0, f64::from(target));
            self.pushed_diff_scroll = Some(target);
        }
    }
}

fn tabs(state: &AppState) -> Vec<TabsItem> {
    state
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| TabsItem {
            key: Some(tab.path.as_ref().map_or_else(
                || format!("welcome:{}", index),
                |p| format!("tab:{}:{}", index, p.display()),
            )),
            label: tab.title.clone(),
            active: index == state.active_tab,
            glyph: if tab.path.is_some() {
                crate::ui::icons::REPOSITORY
            } else {
                crate::ui::icons::HOME
            }
            .to_owned(),
        })
        .collect()
}

const SIDEBAR_HEADER_HEIGHT: f32 = 28.0;
const SIDEBAR_ROW_HEIGHT: f32 = 24.0;
const SIDEBAR_MIN_ROWS_HEIGHT: f32 = SIDEBAR_ROW_HEIGHT * 3.0;
/// Vertical chrome above the section body plus the bottom margin, mirroring
/// the pre-slab `sidebar_body_rect` (toggle strip, commit count, filter).
const SIDEBAR_BODY_CHROME: f32 = 108.0;

/// Branch-name tree mirroring the pre-slab sidebar: names split on `/` into
/// folders whose expansion is tracked through `collapsed_sections`.
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

/// One remote's branches grouped under the remote name (`origin`, ...).
struct RemoteTree {
    name: String,
    children: Vec<BranchTreeNode>,
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

fn build_remote_tree(branches: &[&crate::git::models::BranchInfo]) -> Vec<RemoteTree> {
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

/// One leaf row (branch, worktree, stash, tag). The 13u glyph sits in a 20u
/// cell at indent `10 + 12 * depth` inside the inset row and the label 20u
/// in, matching the pre-slab sidebar_row.
fn sidebar_leaf_item(
    key: String,
    icon: &str,
    label: &str,
    depth: usize,
    selected: bool,
) -> SidebarSectionsRowsItem {
    SidebarSectionsRowsItem {
        key: Some(key),
        caret: String::new(),
        icon: icon.to_owned(),
        label: label.to_owned(),
        indent: (10 + 12 * depth).to_f64().unwrap_or(10.0),
        caret_w: 0.0,
        icon_w: 20.0,
        gsize: 13.0,
        selected,
        tone: if selected { ACCENT } else { DIM },
    }
}

/// One folder row with an expand/collapse caret keyed into
/// `collapsed_sections` ("local/foo", "remote/origin", ...). The 11u caret
/// sits in a 13u cell at indent `8 + 12 * depth`, the 12u type glyph in a
/// 19u cell after it, and the label 32u in — pre-slab sidebar_folder_row.
fn sidebar_folder_item(
    state: &AppState,
    key: &str,
    icon: &str,
    label: &str,
    depth: usize,
) -> SidebarSectionsRowsItem {
    let collapsed = state.collapsed_sections.contains(key);
    SidebarSectionsRowsItem {
        key: Some(format!("fold:{key}")),
        caret: if collapsed {
            icons::CHEVRON_RIGHT
        } else {
            icons::CHEVRON_DOWN
        }
        .to_owned(),
        icon: icon.to_owned(),
        label: label.to_owned(),
        indent: (8 + 12 * depth).to_f64().unwrap_or(8.0),
        caret_w: 13.0,
        icon_w: 19.0,
        gsize: 12.0,
        selected: false,
        tone: DIM,
    }
}

fn sidebar_branch_item(
    branch: &crate::git::models::BranchInfo,
    label: &str,
    depth: usize,
) -> SidebarSectionsRowsItem {
    let icon = if branch.current {
        icons::CHECK
    } else if branch.remote {
        icons::REMOTE
    } else {
        icons::BRANCH
    };
    let prefix = if branch.remote { "remote" } else { "local" };
    sidebar_leaf_item(
        format!("{prefix}:{}", branch.name),
        icon,
        label,
        depth,
        branch.current,
    )
}

fn sidebar_branch_tree_rows(
    rows: &mut Vec<SidebarSectionsRowsItem>,
    state: &AppState,
    nodes: &[BranchTreeNode],
    key_prefix: &str,
    depth: usize,
    branches: &[&crate::git::models::BranchInfo],
) {
    for node in nodes {
        match node {
            BranchTreeNode::Folder { name, children } => {
                let key = format!("{key_prefix}/{name}");
                rows.push(sidebar_folder_item(state, &key, icons::FOLDER, name, depth));
                if !state.collapsed_sections.contains(&key) {
                    sidebar_branch_tree_rows(rows, state, children, &key, depth + 1, branches);
                }
            }
            BranchTreeNode::Leaf { name, branch_name } => {
                if let Some(branch) = branches.iter().find(|branch| branch.name == *branch_name) {
                    rows.push(sidebar_branch_item(branch, name, depth));
                }
            }
        }
    }
}

/// Distributes the sidebar body height across sections exactly like the
/// pre-slab `sidebar_section_layouts`: every open section gets at most three
/// rows up front, then leftover space grows sections proportionally to the
/// user-dragged `sidebar_section_fractions`.
fn sidebar_section_heights(state: &AppState, sections: &[(&'static str, usize, f32)]) -> Vec<f32> {
    let body_height =
        (layout::Layout::for_state(state).sidebar.height - SIDEBAR_BODY_CHROME).max(0.0);
    let available =
        (body_height - sections.len().to_f32().unwrap_or(0.0) * SIDEBAR_HEADER_HEIGHT).max(0.0);
    let mut heights = vec![0.0_f32; sections.len()];
    for (index, (title, _, content_height)) in sections.iter().enumerate() {
        if state.collapsed_sections.contains(*title) || *content_height <= 0.0 {
            continue;
        }
        heights[index] = content_height.min(SIDEBAR_MIN_ROWS_HEIGHT);
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
            .filter(|(index, (title, _, content_height))| {
                !state.collapsed_sections.contains(*title)
                    && *content_height > heights[*index] + 0.5
            })
            .map(|(index, (_, _, content_height))| (index, *content_height))
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
        for (index, content_height) in eligible {
            let share = remaining
                * state
                    .sidebar_section_fractions
                    .get(index)
                    .copied()
                    .unwrap_or(1.0)
                / weight;
            let addition = share.min(content_height - heights[index]);
            heights[index] += addition;
            used += addition;
        }
        if used <= 0.5 {
            break;
        }
        remaining -= used;
    }
    heights
}
fn graph_projection_key(state: &AppState) -> GraphProjectionKey {
    let layout = layout::Layout::for_state(state);
    let columns = layout::column_layout(state, layout.center);
    let mut selected_commits = state.selected_commits.iter().cloned().collect::<Vec<_>>();
    selected_commits.sort_unstable();
    GraphProjectionKey {
        snapshot_revision: state.snapshot_revision(),
        graph_width: columns.graph.width,
        ref_width: columns.refs.width,
        message_width: columns.message.width,
        date_width: columns.date.width,
        main_view: state.main_view,
        selected_commit: state.selected_commit.clone(),
        selected_commits,
        search: state.search.text().to_owned(),
        search_cursor: state.search_cursor,
        show_commit_author: state.settings.show_commit_author,
        show_commit_date: state.settings.show_commit_date,
        show_commit_sha: state.settings.show_commit_sha,
    }
}

fn sidebar_projection_key(state: &AppState) -> SidebarProjectionKey {
    let mut collapsed = state.collapsed_sections.iter().cloned().collect::<Vec<_>>();
    collapsed.sort_unstable();
    SidebarProjectionKey {
        snapshot_revision: state.snapshot_revision(),
        height: layout::Layout::for_state(state).sidebar.height,
        filter: state.branch_filter.text().to_lowercase(),
        collapsed,
        fractions: state.sidebar_section_fractions,
    }
}

fn branch_projection_key(state: &AppState) -> BranchProjectionKey {
    BranchProjectionKey {
        snapshot_revision: state.snapshot_revision(),
        filter: state.branch_filter.trim().to_lowercase(),
    }
}

/// Builds the ten sidebar sections (headers, counts, fold-aware tree rows)
/// from the snapshot, mirroring the pre-slab `build_sidebar` content.
fn sidebar_sections(state: &AppState) -> Vec<SidebarSectionsItem> {
    let Some(snapshot) = &state.snapshot else {
        return Vec::new();
    };
    let query = state.branch_filter.text().to_lowercase();
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
    let rows_height = |count: usize| count.to_f32().unwrap_or(0.0) * SIDEBAR_ROW_HEIGHT;
    let sections: [(&'static str, usize, f32); 10] = [
        (
            "LOCAL",
            local.len(),
            rows_height(if filtered {
                local.len()
            } else {
                tree_visible_rows(&local_tree, "local", state)
            }),
        ),
        (
            "REMOTE",
            remote.len(),
            rows_height(if filtered {
                remote.len()
            } else {
                remote_tree_visible_rows(&remote_tree, state)
            }),
        ),
        (
            "WORKTREES",
            snapshot.worktrees.len(),
            rows_height(snapshot.worktrees.len()),
        ),
        (
            "STASHES",
            snapshot.stashes.len(),
            rows_height(snapshot.stashes.len()),
        ),
        ("CLOUD PATCHES", 0, 0.0),
        ("PULL REQUESTS", 0, 0.0),
        ("GITHUB ISSUES", 0, 0.0),
        ("TEAMS", 0, 0.0),
        (
            "TAGS",
            snapshot.tags.len(),
            rows_height(snapshot.tags.len()),
        ),
        ("SUBMODULES", 0, 0.0),
    ];
    let heights = sidebar_section_heights(state, &sections);
    sections
        .iter()
        .zip(heights)
        .map(|((title, count, _), height)| {
            let collapsed = state.collapsed_sections.contains(*title);
            let open = !collapsed && height > 0.0;
            let mut rows = Vec::new();
            if open {
                match *title {
                    "LOCAL" if filtered => {
                        rows.extend(
                            local
                                .iter()
                                .map(|branch| sidebar_branch_item(branch, &branch.name, 0)),
                        );
                    }
                    "LOCAL" => {
                        sidebar_branch_tree_rows(&mut rows, state, &local_tree, "local", 0, &local);
                    }
                    "REMOTE" if filtered => {
                        rows.extend(
                            remote
                                .iter()
                                .map(|branch| sidebar_branch_item(branch, &branch.name, 0)),
                        );
                    }
                    "REMOTE" => {
                        for group in &remote_tree {
                            let key = format!("remote/{}", group.name);
                            rows.push(sidebar_folder_item(
                                state,
                                &key,
                                icons::REMOTE,
                                &group.name,
                                0,
                            ));
                            if !state.collapsed_sections.contains(&key) {
                                sidebar_branch_tree_rows(
                                    &mut rows,
                                    state,
                                    &group.children,
                                    &key,
                                    1,
                                    &remote,
                                );
                            }
                        }
                    }
                    "WORKTREES" => {
                        rows.extend(snapshot.worktrees.iter().map(|worktree| {
                            sidebar_leaf_item(
                                format!("worktree:{}", worktree.name),
                                icons::WORKSPACE,
                                &worktree.name,
                                0,
                                false,
                            )
                        }));
                    }
                    "STASHES" => {
                        rows.extend(snapshot.stashes.iter().map(|stash| {
                            sidebar_leaf_item(
                                format!("stash:{}", stash.index),
                                icons::ARCHIVE,
                                &stash.name,
                                0,
                                false,
                            )
                        }));
                    }
                    "TAGS" => {
                        rows.extend(snapshot.tags.iter().map(|tag| {
                            sidebar_leaf_item(
                                format!("tag:{}", tag.name),
                                icons::TAG,
                                &tag.name,
                                0,
                                false,
                            )
                        }));
                    }
                    _ => {}
                }
            }
            SidebarSectionsItem {
                key: Some(format!("section:{title}")),
                title: (*title).to_owned(),
                count: count.to_string(),
                caret: if collapsed {
                    icons::CHEVRON_RIGHT
                } else {
                    icons::CHEVRON_DOWN
                }
                .to_owned(),
                open,
                add_remote: *title == "REMOTE",
                body_height: f64::from(height),
                rows,
            }
        })
        .collect()
}

/// Collapsed-rail entries mirroring the expanded sidebar sections, with the
/// same live counts the expanded headers show.
fn sidebar_rail(state: &AppState) -> Vec<SidebarRailItem> {
    let snapshot = state.snapshot.as_ref();
    let branches = |remote: bool| {
        snapshot.map_or(0, |snapshot| {
            snapshot
                .branches
                .iter()
                .filter(|branch| branch.remote == remote)
                .count()
        })
    };
    [
        ("local", icons::BRANCH, "LOCAL", branches(false)),
        ("remote", icons::REMOTE, "REMOTE", branches(true)),
        (
            "worktrees",
            icons::WORKSPACE,
            "WORKTREES",
            snapshot.map_or(0, |snapshot| snapshot.worktrees.len()),
        ),
        (
            "stashes",
            icons::ARCHIVE,
            "STASHES",
            snapshot.map_or(0, |snapshot| snapshot.stashes.len()),
        ),
        ("cloud_patches", icons::CLOUD, "CLOUD PATCHES", 0),
        ("pull_requests", icons::GIT_PULL_REQUEST, "PULL REQUESTS", 0),
        ("github_issues", icons::ISSUES, "GITHUB ISSUES", 0),
        ("teams", icons::ORGANIZATION, "TEAMS", 0),
        (
            "tags",
            icons::TAG,
            "TAGS",
            snapshot.map_or(0, |snapshot| snapshot.tags.len()),
        ),
        ("submodules", icons::SUBMODULE, "SUBMODULES", 0),
    ]
    .into_iter()
    .map(|(key, icon, title, count)| SidebarRailItem {
        key: Some(key.to_owned()),
        icon: icon.to_owned(),
        title: title.to_owned(),
        count: count.to_string(),
        show_count: count > 0,
    })
    .collect()
}

fn graph_rows(state: &AppState) -> Vec<GraphRowsItem> {
    let Some(snapshot) = &state.snapshot else {
        return Vec::new();
    };
    let layout = layout::Layout::for_state(state);
    let columns = layout::column_layout(state, layout.center);
    let graph_width = columns.graph.width;
    let ref_width = columns.refs.width;
    let date_width = columns.date.width;
    let message_width = columns.message.width;
    let wip_rows = snapshot.wip_rows();
    let total_rows = snapshot.commits.len().saturating_add(wip_rows);
    let search_results = state.search_results();
    let current_search = search_results.get(state.search_cursor).copied();
    let mut rows = Vec::with_capacity(total_rows);
    // Links leaving the previous row downward; the current row paints their
    // top halves so no lane ink ever crosses a row boundary.
    let mut incoming: Vec<LaneLink> = Vec::new();

    if snapshot.working.is_dirty() {
        let head_graph = snapshot.head_id.as_ref().and_then(|head_id| {
            snapshot
                .commits
                .iter()
                .position(|commit| commit.id == *head_id)
                .and_then(|index| state.graph.rows.get(index))
        });
        let lane = head_graph.map_or(0, |row| row.lane);
        let tone = graph_color(head_graph.map_or(0, |row| row.color));
        let node_x = graph_lane_x(lane);
        let mut item = graph_item("wip".to_owned(), node_x, tone, graph_width);
        item.lane_extra = format!("M{node_x:.1} 13 L{graph_width:.1} 13");
        item.lane_extra_tone = color_alpha(tone, 230);
        item.lane_dashed = format!("M{node_x:.1} 23 L{node_x:.1} 26");
        item.lane_dashed_tone = tone;
        // HEAD renders the boxed `// WIP` placeholder in the monospace face;
        // the chip's lead slot is the mono run, so the label stays empty.
        let mut wip_chip = graph_ref_chip(
            "wip".to_owned(),
            String::new(),
            82.0,
            PURPLE,
            PURPLE,
            color_alpha(PURPLE, 89),
            PURPLE_SOFT,
        );
        wip_chip.lead = "// WIP".to_owned();
        wip_chip.show_lead = true;
        wip_chip.lead_size = 11.0;
        wip_chip.lead_tone = PURPLE;
        // HEAD's `// WIP` chip is static ink; the whole row carries SelectWip.
        wip_chip.passive = true;
        item.message_chips = vec![wip_chip];
        let (modified, added) = working_change_counts(snapshot);
        item.has_modified = modified > 0;
        item.modified_count = modified.to_string();
        item.has_added = added > 0;
        item.added_count = added.to_string();
        item.dot_visible = false;
        item.wip_node = true;
        item.selected = state.main_view == MainView::Wip && state.selected_commit.is_none();
        rows.push(item);
        incoming = vec![LaneLink {
            from_x: node_x,
            to_x: node_x,
            style: LaneStyle::Dashed(tone),
        }];
    }

    for worktree in snapshot
        .worktrees
        .iter()
        .filter(|worktree| worktree.changes > 0)
    {
        let target_graph = worktree.target.as_ref().and_then(|target| {
            snapshot
                .commits
                .iter()
                .position(|commit| commit.id == *target)
                .and_then(|index| state.graph.rows.get(index))
        });
        let node_x = graph_lane_x(target_graph.map_or(0, |row| row.lane));
        let mut item = graph_item(worktree_wip_key(worktree), node_x, PURPLE, graph_width);
        apply_lane_ink(&mut item, &incoming, &[]);
        push_lane_route(
            &mut item,
            LaneStyle::Solid(PURPLE),
            &format!("M{node_x:.1} 13 L{node_x:.1} 26"),
        );
        let label = worktree
            .branch
            .as_deref()
            .unwrap_or(worktree.name.as_str())
            .to_uppercase();
        // HEAD draws the whole worktree chip run (workspace glyph + name)
        // in the 10px monospace face; route it through the mono lead slot.
        let mut chip = graph_ref_chip(
            format!("worktree:{}", worktree.name),
            String::new(),
            (ref_width - 16.0).clamp(36.0, 190.0),
            PURPLE,
            PURPLE,
            color_alpha(PURPLE, 89),
            PURPLE_SOFT,
        );
        chip.lead = format!("{}  {label}", crate::ui::icons::WORKSPACE);
        chip.show_lead = true;
        chip.lead_size = 10.0;
        chip.lead_tone = PURPLE;
        // HEAD draws this chip as static ink (the row itself is the hit target).
        chip.passive = true;
        item.refs = vec![chip];
        item.show_graph_label = true;
        item.graph_label = format!(
            "// WIP  {} {}  ·  {}",
            crate::ui::icons::DIFF_ADDED,
            worktree.changes,
            worktree.name
        );
        item.show_trail = false;
        item.dot_visible = false;
        item.worktree_node = true;
        item.worktree_wip = true;
        item.selected = state.selected_commit.as_deref() == worktree.target.as_deref();
        rows.push(item);
        incoming = vec![LaneLink {
            from_x: node_x,
            to_x: node_x,
            style: LaneStyle::Solid(PURPLE),
        }];
    }

    for (index, commit) in snapshot.commits.iter().enumerate() {
        let graph = state.graph.rows.get(index);
        let lane = graph.map_or(0, |row| row.lane);
        let tone = graph_color(graph.map_or(0, |row| row.color));
        let node_x = graph_lane_x(lane);
        let outgoing = graph.map_or_else(Vec::new, lane_links);
        let mut item = graph_item(commit.id.clone(), node_x, tone, graph_width);
        apply_lane_ink(&mut item, &incoming, &outgoing);
        incoming = outgoing;
        item.refs = commit_ref_chips(commit, ref_width);
        item.subject.clone_from(&commit.subject);
        item.description.clone_from(&commit.description);
        item.band_mark = true;
        item.date = if state.settings.show_commit_date {
            // Matches HEAD's format_time_for_width: the column text bounds
            // (width - 8) must reach 150px before the full date@time form.
            if date_width - 8.0 >= 150.0 {
                format_timestamp(commit.authored_seconds)
            } else {
                format_date(commit.authored_seconds)
            }
        } else {
            String::new()
        };
        item.sha = state
            .settings
            .show_commit_sha
            .then(|| commit.short_id.clone())
            .unwrap_or_default();
        item.selected = state.selected_commits.contains(&commit.id)
            || state.selected_commit.as_deref() == Some(commit.id.as_str());
        item.local = commit.is_local;
        item.matched = search_results.binary_search(&index).is_ok();
        item.current_match = current_search == Some(index);

        // HEAD policy: every non-merge commit carries its author avatar;
        // merge commits and the avatars-off setting use a small lane dot.
        let avatar_visible = state.settings.show_commit_author && commit.parents.len() < 2;
        item.avatar_visible = avatar_visible;
        item.dot_visible = !avatar_visible;
        if avatar_visible {
            item.avatar = format!("graph-avatar:{}", graph_avatar_key(&commit.email));
        }
        rows.push(item);
    }
    rows
}

fn graph_item(key: String, node_x: f32, node_tone: u32, graph_width: f32) -> GraphRowsItem {
    let trail = graph_trail(node_x, graph_width);
    GraphRowsItem {
        key: Some(key),
        lane_c0: String::new(),
        lane_c1: String::new(),
        lane_c2: String::new(),
        lane_c3: String::new(),
        lane_c4: String::new(),
        lane_c5: String::new(),
        lane_c6: String::new(),
        lane_c7: String::new(),
        lane_c8: String::new(),
        lane_c9: String::new(),
        lane_extra: String::new(),
        lane_extra_tone: PURPLE,
        lane_dashed: String::new(),
        lane_dashed_tone: node_tone,
        refs: Vec::new(),
        message_chips: Vec::new(),
        subject: String::new(),
        description: String::new(),
        show_graph_label: false,
        graph_label: String::new(),
        date: String::new(),
        sha: String::new(),
        node_x: f64::from(node_x),
        node_tone,
        band_mark: false,
        show_trail: true,
        trail_base_width: f64::from(trail.base_width),
        trail_lift_x: f64::from(trail.lift_x),
        trail_lift_width: f64::from(trail.lift_width),
        trail_shadow_x: f64::from(trail.shadow_x),
        trail_shadow_width: f64::from(trail.shadow_width),
        trail_end_x: f64::from(trail.end_x),
        trail_base_tone: color_alpha(node_tone, 13),
        trail_lift_tone: color_alpha(node_tone, 15),
        trail_end_tone: color_alpha(node_tone, 230),
        avatar: String::new(),
        avatar_visible: false,
        dot_visible: true,
        wip_node: false,
        worktree_node: false,
        selected: false,
        local: false,
        worktree_wip: false,
        matched: false,
        current_match: false,
        has_modified: false,
        modified_count: String::new(),
        has_added: false,
        added_count: String::new(),
    }
}


#[derive(Clone, Copy)]
struct GraphTrail {
    base_width: f32,
    lift_x: f32,
    lift_width: f32,
    shadow_x: f32,
    shadow_width: f32,
    end_x: f32,
}

fn graph_trail(node_x: f32, graph_width: f32) -> GraphTrail {
    let lift_x = (graph_width - GRAPH_TRAIL_LIFT_WIDTH).max(node_x);
    let base_width = (lift_x - node_x).max(0.0);
    let shadow_width = GRAPH_TRAIL_SHADOW_WIDTH.min(base_width);
    GraphTrail {
        base_width,
        lift_x,
        lift_width: (graph_width - lift_x).max(0.0),
        shadow_x: lift_x - shadow_width,
        shadow_width,
        end_x: (graph_width - 2.0).max(0.0),
    }
}

/// Stroke selection for one lane link: a palette slot index, an
/// arbitrary-tone solid stroke, or an arbitrary-tone dashed stroke.
#[derive(Clone, Copy)]
enum LaneStyle {
    Palette(usize),
    Solid(u32),
    Dashed(u32),
}

/// One lane connection leaving a row downward, in resolved pixel space.
#[derive(Clone, Copy)]
struct LaneLink {
    from_x: f32,
    to_x: f32,
    style: LaneStyle,
}

fn lane_links(row: &crate::graph::layout::GraphRow) -> Vec<LaneLink> {
    row.segments
        .iter()
        .map(|segment| LaneLink {
            from_x: graph_lane_x(segment.from),
            to_x: graph_lane_x(segment.to),
            style: LaneStyle::Palette(segment.color % GRAPH_COLORS.len()),
        })
        .collect()
}

/// Appends `route` to a slot's multi-subpath string.
fn append_route(slot: &mut String, route: &str) {
    if !slot.is_empty() {
        slot.push(' ');
    }
    slot.push_str(route);
}

/// Routes one link's ink into the row's palette, extra, or dashed slot.
fn push_lane_route(item: &mut GraphRowsItem, style: LaneStyle, route: &str) {
    match style {
        LaneStyle::Palette(slot) => append_route(
            match slot {
                0 => &mut item.lane_c0,
                1 => &mut item.lane_c1,
                2 => &mut item.lane_c2,
                3 => &mut item.lane_c3,
                4 => &mut item.lane_c4,
                5 => &mut item.lane_c5,
                6 => &mut item.lane_c6,
                7 => &mut item.lane_c7,
                8 => &mut item.lane_c8,
                _ => &mut item.lane_c9,
            },
            route,
        ),
        LaneStyle::Solid(tone) => {
            item.lane_extra_tone = tone;
            append_route(&mut item.lane_extra, route);
        }
        LaneStyle::Dashed(tone) => {
            item.lane_dashed_tone = tone;
            append_route(&mut item.lane_dashed, route);
        }
    }
}

/// Paints a row's lane ink: the top halves of the links arriving from the
/// row above plus the bottom halves of the links leaving this row. Every
/// route stays inside the row's 26px band, so later-painted row backgrounds
/// (hover, selection, tints) can never cover a neighbour's lanes, and the
/// clipping canvas keeps lanes out of the adjacent columns.
fn apply_lane_ink(item: &mut GraphRowsItem, incoming: &[LaneLink], outgoing: &[LaneLink]) {
    for link in incoming {
        let from_x = link.from_x;
        let to_x = link.to_x;
        let route = if (from_x - to_x).abs() < 0.5 {
            format!("M{from_x:.1} 0 L{from_x:.1} 13")
        } else {
            let corner_x = from_x + (to_x - from_x).signum() * 5.0;
            format!("M{from_x:.1} 0 L{from_x:.1} 8 Q{from_x:.1} 13 {corner_x:.1} 13 L{to_x:.1} 13")
        };
        push_lane_route(item, link.style, &route);
    }
    for link in outgoing {
        let from_x = link.from_x;
        push_lane_route(
            item,
            link.style,
            &format!("M{from_x:.1} 13 L{from_x:.1} 26"),
        );
    }
}

/// Mono lead advance for the 10.5px check glyph (JetBrains mono = 0.6em),
/// carved out of HEAD's 6.2/char label estimate when the chip is checked out.
const CHECK_LEAD_WIDTH: f32 = 6.3;

fn commit_ref_chips(commit: &CommitSummary, column_width: f32) -> Vec<GraphRowsRefsItem> {
    let mut chips = Vec::new();
    // HEAD's draw_ref_chips cursor: chips start at column.x + 7 and every
    // chip width is budgeted against column_clip.right() - x - 4.
    let mut x = 7.0f32;
    let mut branches = commit
        .branch_refs
        .iter()
        .filter(|reference| !reference.is_tag)
        .collect::<Vec<_>>();
    branches.sort_by(|left, right| {
        right
            .is_head
            .cmp(&left.is_head)
            .then_with(|| left.branch_short_name.cmp(&right.branch_short_name))
    });
    let any_head = branches.iter().any(|reference| reference.is_head);
    // Matches HEAD's draw_ref_chips: one consolidated chip for the leading
    // branch (check glyph when checked out, trailing branch/remote glyphs
    // for its locations) plus a "+N" overflow chip for the rest.
    if let Some(branch) = branches.first() {
        let icon_count = usize::from(branch.is_local) + branch.remote_names.len();
        let label_chars =
            branch.branch_short_name.chars().count() + if branch.is_head { 2 } else { 0 };
        let label_estimate = label_chars.to_f32().unwrap_or(0.0) * 6.2;
        let width = (label_estimate + icon_count.to_f32().unwrap_or(0.0) * 11.0 + 12.0)
            .clamp(38.0, 180.0)
            .min(column_width - x - 4.0);
        // HEAD's primary_branch_chip returns None below 4px of room.
        if width <= 4.0 {
            return chips;
        }
        // The chip key IS the drag/drop payload: graph_ref_identity expects the
        // `local:`/`remote:` prefix and HEAD's BranchClick target (short name
        // for locals, `remote/short` for remote-only branches).
        let identity = if branch.is_local {
            format!("local:{}", branch.branch_short_name)
        } else {
            branch.remote_names.first().map_or_else(
                || format!("remote:{}", branch.branch_short_name),
                |remote| format!("remote:{remote}/{}", branch.branch_short_name),
            )
        };
        let mut chip = graph_ref_chip(
            identity,
            branch.branch_short_name.clone(),
            width,
            TEXT,
            ACCENT,
            BORDER_STRONG,
            PANEL,
        );
        // HEAD draws the label at badge.x + 6, clipped at badge.right() - 2.
        chip.label_inset = true;
        let mut lead_w = 0.0f32;
        if branch.is_head {
            // HEAD's label is one "✓ name" run; glyphon shapes the check in
            // the icon face and " name" in Sans. Mirror the same split: mono
            // check lead, Sans label with the leading space kept.
            chip.lead = crate::ui::icons::CHECK.to_owned();
            chip.show_lead = true;
            chip.lead_size = 10.5;
            chip.lead_tone = TEXT;
            chip.label = format!(" {}", branch.branch_short_name);
            lead_w = CHECK_LEAD_WIDTH;
        }
        // Fixed label width pins the trail glyphs at HEAD's icon_x
        // (badge.x + 6 + chars*6.2) and hard-truncates squeezed labels at
        // HEAD's clip edge instead of shrinking them for the icons.
        chip.label_w = f64::from((label_estimate - lead_w).min(width - 8.0 - lead_w).max(0.0));
        let mut trail = String::new();
        // HEAD paints location glyphs on an 11px pitch from icon_x, clipped
        // at badge.right() - 2: a glyph starting past that edge never shows.
        let mut icon_x = 6.0 + label_estimate;
        let mut push_icon = |trail: &mut String, glyph: &str| {
            if icon_x < width - 2.0 {
                if !trail.is_empty() {
                    trail.push(' ');
                }
                trail.push_str(glyph);
            }
            icon_x += 11.0;
        };
        if branch.is_local {
            push_icon(&mut trail, crate::ui::icons::BRANCH);
        }
        for _ in &branch.remote_names {
            push_icon(&mut trail, crate::ui::icons::REMOTE);
        }
        if !trail.is_empty() {
            chip.trail = trail;
            chip.show_trail_icons = true;
        }
        chips.push(chip);
        x += width + 4.0;
        if branches.len() > 1 {
            // HEAD: +N overflow chip is 24 wide (column-budgeted), label
            // drawn theme.text at badge.x + 6, border theme.border.
            let extra_width = 24.0f32.min(column_width - x - 4.0);
            if extra_width > 4.0 {
                let mut extra_chip = graph_ref_chip(
                    "extra".to_owned(),
                    format!("+{}", branches.len() - 1),
                    extra_width,
                    TEXT,
                    MUTED,
                    BORDER,
                    PANEL,
                );
                extra_chip.label_inset = true;
                extra_chip.label_w = f64::from(
                    (extra_chip.label.chars().count().to_f32().unwrap_or(0.0) * 6.2)
                        .min(extra_width - 8.0)
                        .max(0.0),
                );
                // HEAD's +N overflow chip has no action: never hovers, drags, or drops.
                extra_chip.passive = true;
                chips.push(extra_chip);
                x += extra_width + 4.0;
            }
        }
    }
    if !any_head && commit.refs.iter().any(|label| label.kind == RefKind::Head) {
        // HEAD: detached-HEAD chip is 40 wide (column-budgeted).
        let head_width = 40.0f32.min(column_width - x - 4.0);
        if head_width > 4.0 {
            let mut head_chip = graph_ref_chip(
                "head:HEAD".to_owned(),
                "HEAD".to_owned(),
                head_width,
                TEXT,
                ACCENT,
                ACCENT,
                PANEL,
            );
            head_chip.label_inset = true;
            head_chip.label_w = f64::from((4.0_f32 * 6.2).min(head_width - 8.0).max(0.0));
            // HEAD's detached-HEAD chip has no action: never hovers, drags, or drops.
            head_chip.passive = true;
            chips.push(head_chip);
            x += head_width + 4.0;
        }
    }
    for label in commit
        .refs
        .iter()
        .filter(|label| matches!(label.kind, RefKind::Tag | RefKind::Worktree))
    {
        let (prefix, glyph, tone, border) = match label.kind {
            RefKind::Tag => ("tag", crate::ui::icons::TAG, ORANGE, ORANGE_SOFT),
            RefKind::Worktree => ("worktree", crate::ui::icons::WORKSPACE, PURPLE, PURPLE_SOFT),
            _ => unreachable!("filtered to tag and worktree refs"),
        };
        // HEAD: full = chars*6.2 + 24, clamped to [36, 160], then budgeted
        // against the remaining column room; a chip below 4px ends the run.
        let width = (label.name.chars().count().to_f32().unwrap_or(0.0) * 6.2 + 24.0)
            .clamp(36.0, 160.0)
            .min(column_width - x - 4.0);
        if width <= 4.0 {
            break;
        }
        let mut chip = graph_ref_chip(
            format!("{prefix}:{}", label.name),
            label.name.clone(),
            width,
            tone,
            tone,
            border,
            PANEL,
        );
        // HEAD: glyph at badge.x + 5 (9.5px terminal face), name at
        // badge.x + 16, both clipped at badge.right() - 2. The trailing
        // space widens the mono lead to 11.4 so the label lands at ~16.
        chip.lead_inset = true;
        chip.lead = format!("{glyph} ");
        chip.show_lead = true;
        chip.label_w = f64::from(
            (label.name.chars().count().to_f32().unwrap_or(0.0) * 6.2)
                .min(width - 18.4)
                .max(0.0),
        );
        // Only tag chips carry TagClick at HEAD; worktree chips are static ink.
        chip.passive = label.kind == RefKind::Worktree;
        chips.push(chip);
        x += width + 4.0;
    }
    chips
}

#[allow(clippy::too_many_arguments)]
fn graph_ref_chip(
    key: String,
    label: String,
    width: f32,
    tone: u32,
    hover_stroke: u32,
    border: u32,
    fill: u32,
) -> GraphRowsRefsItem {
    GraphRowsRefsItem {
        key: Some(key),
        // Lead-only chips (WIP / worktree) skip the label slot entirely so the
        // empty text child never contributes a phantom gap.
        show_label: !label.is_empty(),
        label,
        label_w: 0.0,
        lead: String::new(),
        show_lead: false,
        lead_size: 9.5,
        lead_tone: DIM,
        trail: String::new(),
        show_trail_icons: false,
        width: f64::from(width),
        label_inset: false,
        lead_inset: false,
        tone,
        hover_stroke,
        border,
        chip_bg: fill,
        passive: false,
    }
}

fn graph_virtual_window(
    state: &AppState,
    center_height: f32,
    total: usize,
) -> std::ops::Range<usize> {
    let viewport = (center_height - COMMIT_HEADER_HEIGHT).max(0.0);
    if viewport <= 0.0 {
        return 0..total.min(GRAPH_OVERSCAN.saturating_mul(2));
    }
    let first = (state.graph_scroll.max(0.0) / COMMIT_ROW_HEIGHT)
        .floor()
        .to_usize()
        .unwrap_or(0);
    let last = ((state.graph_scroll.max(0.0) + viewport) / COMMIT_ROW_HEIGHT)
        .ceil()
        .to_usize()
        .unwrap_or(total);
    first.saturating_sub(GRAPH_OVERSCAN).min(total)..last.saturating_add(GRAPH_OVERSCAN).min(total)
}

fn working_change_counts(snapshot: &RepoSnapshot) -> (usize, usize) {
    let mut modified = 0;
    let mut added = 0;
    for file in &snapshot.working.files {
        let kind = file.staged.or(file.unstaged);
        if matches!(kind, Some(ChangeKind::Added)) {
            added += 1;
        } else if kind.is_some() {
            modified += 1;
        }
    }
    (modified, added)
}

fn graph_lane_x(lane: usize) -> f32 {
    GRAPH_LANE_ORIGIN + lane.to_f32().unwrap_or(0.0) * GRAPH_LANE_SPACING
}

fn graph_color(index: usize) -> u32 {
    GRAPH_COLORS[index % GRAPH_COLORS.len()]
}

fn graph_avatar_key(email: &str) -> String {
    let email = email.trim().to_lowercase();
    format!("{:x}", md5::compute(email.as_bytes()))
}

fn worktree_wip_key(worktree: &WorktreeInfo) -> String {
    format!("worktree-wip:{}", worktree.path.display())
}

fn sidebar_rail_section(key: &str) -> Option<&'static str> {
    key.split('/').find_map(|segment| match segment {
        "local" => Some("LOCAL"),
        "remote" => Some("REMOTE"),
        "worktrees" => Some("WORKTREES"),
        "stashes" => Some("STASHES"),
        "cloud_patches" => Some("CLOUD PATCHES"),
        "pull_requests" => Some("PULL REQUESTS"),
        "github_issues" => Some("GITHUB ISSUES"),
        "teams" => Some("TEAMS"),
        "tags" => Some("TAGS"),
        "submodules" => Some("SUBMODULES"),
        _ => None,
    })
}

/// Exhaustive generated-signal adapter. Slab decides geometry and gesture
/// semantics; this is the sole bridge into the application's semantic actions.
fn dispatch_signal(
    state: &mut AppState,
    signal: generated::Signal,
    host: &mut Vec<SlabHostCommand>,
    diff_scroll_x: f64,
) {
    use generated::Signal;

    let action = match signal {
        Signal::WindowClose { .. } => {
            host.push(SlabHostCommand::Close);
            None
        }
        Signal::WindowMinimize { .. } => {
            host.push(SlabHostCommand::Minimize);
            None
        }
        Signal::WindowMaximize { .. } => {
            host.push(SlabHostCommand::ToggleMaximize);
            None
        }
        Signal::WindowDrag { .. } => {
            host.push(SlabHostCommand::DragWindow);
            None
        }
        Signal::DismissOverlay { .. } | Signal::OverlaySecondary { .. } => {
            Some(UiAction::DismissOverlay)
        }
        Signal::SelectTab { item, .. } => tab_index(&item).map(UiAction::SelectTab),
        Signal::CloseTab { item, .. } => tab_index(&item).map(UiAction::CloseTab),
        Signal::NewTab { .. } => Some(UiAction::NewTab),
        Signal::ToggleBranchMenu { .. } => Some(UiAction::ToggleBranchMenu),
        Signal::ToggleActions { meta, .. } | Signal::ToggleActionsMenu { meta, .. } => {
            state.overlay_target = meta.key;
            Some(UiAction::ToggleActionsMenu)
        }
        Signal::Undo { .. } => Some(UiAction::Undo),
        Signal::Redo { .. } => Some(UiAction::Redo),
        Signal::Pull { .. } => Some(UiAction::Pull),
        Signal::TogglePullOptions { meta, .. } => {
            state.overlay_target = meta.key;
            Some(UiAction::TogglePullOptions)
        }
        Signal::Push { .. } => Some(UiAction::Push),
        Signal::ToggleCreateBranch { .. } => Some(UiAction::ToggleCreateBranch),
        Signal::Stash { .. } => Some(UiAction::Stash),
        Signal::PopStash { .. } => Some(UiAction::PopStash),
        Signal::ToggleLfsMenu { meta, .. } => {
            state.overlay_target = meta.key;
            Some(UiAction::ToggleLfsMenu)
        }
        Signal::OpenTerminal { .. } => Some(UiAction::OpenTerminal),
        Signal::ToggleSearch { .. } => Some(UiAction::ToggleSearch),
        Signal::ToggleTabSwitcher { meta, .. } => {
            state.overlay_target = meta.key;
            Some(UiAction::ToggleTabSwitcher)
        }
        Signal::ToggleNotifications { meta, .. } => {
            state.overlay_target = meta.key;
            Some(UiAction::ToggleNotifications)
        }
        Signal::OpenPreferences { .. } => Some(UiAction::OpenPreferences),
        Signal::ToggleSidebarCollapse { .. } => Some(UiAction::ToggleSidebarCollapse),
        Signal::ExpandSidebarSection { meta, .. } => sidebar_rail_section(&meta.key)
            .map(|section| UiAction::ExpandSidebarSection(section.to_owned())),
        Signal::ShowList { .. } => Some(UiAction::SetShowAgents(false)),
        Signal::ShowAgents { .. } => Some(UiAction::SetShowAgents(true)),
        Signal::FocusBranchFilter { .. } => Some(UiAction::FocusBranchFilter),
        Signal::ChangeBranchFilter { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::BranchFilter,
            text,
        }),
        Signal::SidebarRow { item, .. } => sidebar_action(&item),
        Signal::SidebarContext { item, .. } => sidebar_context_action(&item),
        Signal::OpenAddRemote { .. } => Some(UiAction::OpenAddRemote),
        Signal::SidebarResize { text, .. } => resize_action(ResizeTarget::Sidebar, &text),
        Signal::SidebarReset { .. } => Some(UiAction::ResetResize(ResizeTarget::Sidebar)),
        Signal::GraphRow { item, .. } => graph_row_action(state, &item),
        Signal::GraphRowContext { item, .. } => graph_row_context_action(state, &item),
        Signal::GraphRef { item, .. } => graph_ref_action(&item),
        Signal::GraphRefContext { item, .. } => graph_ref_context_action(&item),
        Signal::GraphRefDrag { .. } => None,
        Signal::GraphRefDrop { item, meta, .. } => graph_drop_action(&item, &meta.src_item),
        Signal::ResizeRefColumn { text, .. } => resize_action(ResizeTarget::RefColumn, &text),
        Signal::ResizeGraphColumn { text, .. } => resize_action(ResizeTarget::GraphColumn, &text),
        Signal::ResizeMessageColumn { text, .. } => {
            resize_action(ResizeTarget::MessageColumn, &text)
        }
        Signal::FocusSearch { .. } => Some(UiAction::FocusSearch),
        Signal::ChangeSearch { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::Search,
            text,
        }),
        Signal::SubmitSearch { .. } | Signal::NextSearch { .. } => Some(UiAction::NextSearchResult),
        Signal::PreviousSearch { .. } => Some(UiAction::PreviousSearchResult),
        Signal::CloseSearch { .. } => Some(UiAction::CloseSearch),
        Signal::UnstageAll { .. } => Some(UiAction::UnstageAll),
        Signal::StageSelection { .. } => Some(if state.selected_working_files.len() > 1 {
            UiAction::StageSelection
        } else {
            UiAction::StageAll
        }),
        Signal::WipFile { item, .. } => working_file_action(state, &item),
        Signal::WipFileContext { item, .. } => working_file_context_action(state, &item),
        Signal::ToggleFileSelection { item, .. } => {
            working_path(&item).map(UiAction::ToggleFileSelection)
        }
        Signal::StageFile { item, .. } => {
            item_path_or_selected(state, &item).map(UiAction::StageFile)
        }
        Signal::UnstageFile { item, .. } => working_path(&item).map(UiAction::UnstageFile),
        Signal::ToggleAmend { .. } => Some(UiAction::ToggleAmend),
        Signal::FocusCommitSummary { .. } => Some(UiAction::FocusCommitSummary),
        Signal::ChangeCommitSummary { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::CommitSummary,
            text,
        }),
        Signal::FocusCommitBody { .. } => Some(UiAction::FocusCommitBody),
        Signal::ChangeCommitBody { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::CommitBody,
            text,
        }),
        Signal::Commit { .. } => Some(UiAction::Commit),
        Signal::ToggleCommitOptions { .. } => Some(UiAction::ToggleCommitOptions),
        Signal::CloseDiff { .. } => Some(UiAction::CloseDiff),
        Signal::ToggleDiffScope { .. } => Some(UiAction::ToggleDiffScope),
        Signal::ShowFileView { .. } => Some(UiAction::ShowFileView),
        Signal::ShowDiffView { .. } => Some(UiAction::ShowDiffView),
        Signal::ToggleFileHistory { .. } => Some(UiAction::ToggleFileHistory),
        Signal::PreviousHunk { .. } => Some(UiAction::PreviousHunk),
        Signal::NextHunk { .. } => Some(UiAction::NextHunk),
        Signal::ToggleDiffLayout { .. } => Some(UiAction::ToggleDiffLayout),
        Signal::DiffRow { item, .. } => diff_row_index(&item).map(UiAction::SelectDiffRow),
        Signal::DiffRowContext { item, .. } => {
            diff_row_index(&item).map(UiAction::OpenDiffSelection)
        }
        Signal::BeginDiffSelection { item, .. } => {
            diff_row_index(&item).map(UiAction::BeginDiffSelection)
        }
        Signal::BeginOldDiffTextSelection { item, meta } => {
            diff_text_selection_action(state, &item, Some(0), &meta, diff_scroll_x)
        }
        Signal::BeginNewDiffTextSelection { item, meta } => {
            diff_text_selection_action(state, &item, Some(1), &meta, diff_scroll_x)
        }
        Signal::BeginUnifiedDiffTextSelection { item, meta } => {
            diff_text_selection_action(state, &item, None, &meta, diff_scroll_x)
        }
        Signal::DiffSelectionDrag { .. } | Signal::DiffTextDrag { .. } => None,
        Signal::DiffSelectionDragUpdate { meta, .. } => {
            if let Some(row) = diff_row_at(state, meta.y) {
                state.update_diff_line_drag(row);
            }
            None
        }
        Signal::DiffTextDragUpdate { meta, .. } => {
            if let Some((row, side, column)) = diff_text_point(state, &meta, diff_scroll_x) {
                state.update_diff_text_drag(row, side, column);
            }
            None
        }
        Signal::DiffSelectionDragEnd { .. } | Signal::DiffTextDragEnd { .. } => {
            state.end_drag();
            None
        }
        Signal::SeekDiffRow { item, .. } => diff_row_index(&item).map(UiAction::SeekDiffRow),
        Signal::FocusDiffSearch { .. } => Some(UiAction::ToggleDiffSearch),
        Signal::ChangeDiffSearch { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::DiffSearch,
            text,
        }),
        Signal::SubmitDiffSearch { .. } | Signal::NextDiffSearch { .. } => {
            Some(UiAction::NextDiffSearch)
        }
        Signal::PreviousDiffSearch { .. } => Some(UiAction::PreviousDiffSearch),
        Signal::CloseDiffSearch { .. } => Some(UiAction::CloseDiffSearch),
        Signal::FocusTerminal { .. } => Some(UiAction::FocusTerminal),
        Signal::TerminalResize { text, .. } => resize_action(ResizeTarget::TerminalPane, &text),
        Signal::TerminalReset { .. } => Some(UiAction::ResetResize(ResizeTarget::TerminalPane)),
        Signal::TogglePathTree { .. } => Some(UiAction::TogglePathTree),
        Signal::ShowAi { .. } => Some(UiAction::ShowAiStatus),
        Signal::CloseDetail { .. } => Some(UiAction::CloseDetail),
        Signal::ToggleViewAllFiles { .. } => Some(UiAction::ToggleViewAllFiles),
        Signal::DetailFile { item, .. } => detail_file_action(state, &item),
        Signal::DetailFileContext { item, .. } => detail_file_context_action(state, &item),
        Signal::DetailParent { item, .. } => detail_commit_id(&item).map(UiAction::JumpToCommit),
        Signal::DetailCurrentCommit { .. } => {
            state.selected_commit.clone().map(UiAction::JumpToCommit)
        }
        Signal::DetailResize { text, .. } => resize_action(ResizeTarget::DetailPanel, &text),
        Signal::DetailReset { .. } => Some(UiAction::ResetResize(ResizeTarget::DetailPanel)),
        Signal::ResizeDetailMessage { text, .. } => {
            resize_action(ResizeTarget::DetailMessage, &text)
        }
        Signal::OpenRepositoryPicker { .. } => Some(UiAction::OpenRepositoryPicker),
        Signal::CreateRepositoryPicker { .. } => Some(UiAction::CreateRepositoryPicker),
        Signal::ToggleClone { .. } => Some(UiAction::ToggleCloneForm),
        Signal::FocusWelcomeSearch { .. } => Some(UiAction::FocusWelcomeSearch),
        Signal::ChangeWelcomeSearch { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::WelcomeSearch,
            text,
        }),
        Signal::OpenRecentRepo { item, .. } => {
            (!item.is_empty()).then(|| UiAction::OpenRepository(PathBuf::from(item)))
        }
        Signal::FocusCloneUrl { .. } => Some(UiAction::FocusCloneUrl),
        Signal::ChangeCloneUrl { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::CloneUrl,
            text,
        }),
        Signal::SubmitClone { .. } | Signal::CloneRepository { .. } => {
            Some(UiAction::CloneRepository)
        }
        Signal::PickCloneDestination { .. } => Some(UiAction::PickCloneDestination),
        Signal::OpenTutorial { .. } => Some(UiAction::OpenExternalUrl(
            "https://help.gitkraken.com/".to_owned(),
        )),
        Signal::OpenReleaseNotes { .. } => Some(UiAction::OpenExternalUrl(
            "https://help.gitkraken.com/gitkraken-client/current-release-notes/".to_owned(),
        )),
        Signal::OpenDocumentation { .. } | Signal::OpenSupport { .. } => Some(
            UiAction::OpenExternalUrl("https://help.gitkraken.com/".to_owned()),
        ),
        Signal::ExitPreferences { .. } => Some(UiAction::ExitPreferences),
        Signal::SelectPreferencePage { item, .. } => item
            .strip_prefix("pref_page:")
            .map(|page| UiAction::SelectPreferencePage(page.to_owned())),
        Signal::PreferenceToggle { item, .. } => Some(UiAction::TogglePreference(item)),
        Signal::PreferenceDec { item, .. } => Some(UiAction::AdjustPreference {
            key: item,
            delta: -1,
        }),
        Signal::PreferenceInc { item, .. } => Some(UiAction::AdjustPreference {
            key: item,
            delta: 1,
        }),
        Signal::PreferenceField { item, text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::Preference(item),
            text,
        }),
        Signal::FocusPreferenceField { item, .. } => Some(UiAction::FocusPreferenceText(item)),
        Signal::PreferenceBrowse { item, .. } => Some(UiAction::BrowsePreferencePath(item)),
        Signal::PreferenceAction { item, .. } => preference_action(&item),
        Signal::PreferenceActionAddProfile { .. } => Some(UiAction::AddCommitProfile),
        Signal::PreferenceSelectProfile { item, .. } => Some(UiAction::SelectCommitProfile(item)),
        Signal::OverlayRow { item, .. } | Signal::OverlaySubmenuRow { item, .. } => {
            overlay_row_action(state, &item)
        }
        Signal::OverlayPrimary { .. } | Signal::SubmitOverlayField { .. } => {
            overlay_submit_action(state)
        }
        Signal::FocusOverlayFieldOne { .. } => overlay_focus_action(state, 1),
        Signal::FocusOverlayFieldTwo { .. } => overlay_focus_action(state, 2),
        Signal::FocusOverlayFieldThree { .. } => overlay_focus_action(state, 3),
        Signal::ChangeOverlayFieldOne { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::OverlayField(1),
            text,
        }),
        Signal::ChangeOverlayFieldTwo { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::OverlayField(2),
            text,
        }),
        Signal::ChangeOverlayFieldThree { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::OverlayField(3),
            text,
        }),
        Signal::FocusTabFilter { .. } => Some(UiAction::FocusTabFilter),
        Signal::ChangeTabFilter { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::TabFilter,
            text,
        }),
        Signal::PaletteRow { item, .. } => {
            palette_index(&item).map(UiAction::ExecutePaletteCommand)
        }
        Signal::FocusPalette { .. } => Some(UiAction::FocusPalette),
        Signal::ChangePalette { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::Palette,
            text,
        }),
        Signal::SubmitPalette { .. } => state
            .palette
            .as_ref()
            .map(|palette| UiAction::ExecutePaletteCommand(palette.cursor)),
        Signal::SelectAddRemoteUrl { .. } => Some(UiAction::SelectAddRemoteProvider(
            crate::ui::action::AddRemoteProvider::Url,
        )),
        Signal::SelectAddRemoteGithub { .. } => Some(UiAction::SelectAddRemoteProvider(
            crate::ui::action::AddRemoteProvider::GitHub,
        )),
        Signal::SelectAddRemoteGitea { .. } => Some(UiAction::SelectAddRemoteProvider(
            crate::ui::action::AddRemoteProvider::Gitea,
        )),
        Signal::FocusAddRemoteName { .. } => Some(UiAction::FocusAddRemoteName),
        Signal::FocusAddRemoteUrl { .. } => Some(UiAction::FocusAddRemoteUrl),
        Signal::FocusAddRemotePushUrl { .. } => Some(UiAction::FocusAddRemotePushUrl),
        Signal::FocusAddRemoteRepo { .. } => Some(UiAction::FocusAddRemoteRepo),
        Signal::FocusAddRemoteHost { .. } => Some(UiAction::FocusAddRemoteHost),
        Signal::ChangeAddRemoteName { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::AddRemoteName,
            text,
        }),
        Signal::ChangeAddRemoteUrl { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::AddRemoteUrl,
            text,
        }),
        Signal::ChangeAddRemotePushUrl { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::AddRemotePushUrl,
            text,
        }),
        Signal::ChangeAddRemoteRepo { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::AddRemoteRepo,
            text,
        }),
        Signal::ChangeAddRemoteHost { text, .. } => Some(UiAction::SetText {
            target: TextFieldTarget::AddRemoteHost,
            text,
        }),
        Signal::SubmitAddRemote { .. } | Signal::AddRemote { .. } => Some(UiAction::AddRemote),
        Signal::RequestAi { .. } => Some(UiAction::ShowAiStatus),
    };
    if let Some(action) = action {
        state.dispatch(action);
    }
}

fn resize_action(target: ResizeTarget, text: &str) -> Option<UiAction> {
    text.parse::<f32>()
        .ok()
        .map(|extent| UiAction::ResizeTo { target, extent })
}

fn tab_index(item: &str) -> Option<usize> {
    item.strip_prefix("tab:")
        .or_else(|| item.strip_prefix("welcome:"))
        .and_then(|rest| rest.split(':').next())
        .and_then(|index| index.parse().ok())
}

fn sidebar_action(item: &str) -> Option<UiAction> {
    if let Some(section) = item.strip_prefix("section:") {
        Some(UiAction::ToggleSection(section.to_owned()))
    } else if let Some(key) = item.strip_prefix("fold:") {
        Some(UiAction::ToggleSection(key.to_owned()))
    } else if let Some(branch) = item
        .strip_prefix("local:")
        .or_else(|| item.strip_prefix("remote:"))
    {
        Some(UiAction::BranchClick(branch.to_owned()))
    } else if let Some(index) = item
        .strip_prefix("stash:")
        .and_then(|value| value.parse().ok())
    {
        Some(UiAction::OpenStashContext(index))
    } else {
        item.strip_prefix("tag:")
            .map(|tag| UiAction::TagClick(tag.to_owned()))
    }
}

fn sidebar_context_action(item: &str) -> Option<UiAction> {
    if let Some(branch) = item
        .strip_prefix("local:")
        .or_else(|| item.strip_prefix("remote:"))
    {
        Some(UiAction::OpenBranchContext(branch.to_owned()))
    } else if let Some(index) = item
        .strip_prefix("stash:")
        .and_then(|value| value.parse().ok())
    {
        Some(UiAction::OpenStashContext(index))
    } else {
        item.strip_prefix("tag:")
            .map(|tag| UiAction::OpenTagContext(tag.to_owned()))
    }
}

fn working_path(item: &str) -> Option<PathBuf> {
    item.strip_prefix("unstaged:")
        .or_else(|| item.strip_prefix("staged:"))
        .map(PathBuf::from)
}

fn working_file_action(_state: &AppState, item: &str) -> Option<UiAction> {
    let path = working_path(item)?;
    let scope = if item.starts_with("staged:") {
        DiffScope::Staged
    } else {
        DiffScope::Unstaged
    };
    Some(UiAction::SelectFile { path, scope })
}

fn working_file_context_action(state: &AppState, item: &str) -> Option<UiAction> {
    let path = item_path_or_selected(state, item)?;
    let scope = if item.starts_with("staged:") {
        FileContextScope::Staged
    } else {
        FileContextScope::Unstaged
    };
    Some(UiAction::OpenFileContext { path, scope })
}

fn item_path_or_selected(state: &AppState, item: &str) -> Option<PathBuf> {
    working_path(item).or_else(|| {
        state
            .selected_file
            .as_ref()
            .map(|request| request.path.clone())
    })
}

fn diff_row_index(item: &str) -> Option<usize> {
    item.rsplit_once(':')?.1.parse().ok()
}

fn diff_row_at(state: &AppState, y: f64) -> Option<usize> {
    if state.diff_file_view {
        return None;
    }
    let total = state.diff.as_ref()?.rows.len();
    if total == 0 {
        return None;
    }
    let layout = layout::Layout::for_state(state);
    let search_height = if state.focus == FocusField::DiffSearch {
        38.0
    } else {
        0.0
    };
    let top = f64::from(layout.center.y + 78.0 + 24.0 + search_height);
    ((y - top + f64::from(state.diff_scroll)) / 20.0)
        .floor()
        .max(0.0)
        .to_usize()
        .map(|row| row.min(total.saturating_sub(1)))
}

fn diff_text_side(state: &AppState, row: usize, x: f64, scroll_x: f64) -> Option<u8> {
    let diff = state.diff.as_ref()?;
    let line = diff.rows.get(row)?;
    if !state.diff_split {
        return Some(if line.kind == DiffRowKind::Deleted {
            0
        } else {
            1
        });
    }
    let layout = layout::Layout::for_state(state);
    let width = diff_content_width(state, (layout.center.width - 20.0).max(0.0));
    let start = f64::from(layout.center.x) - scroll_x;
    Some(u8::from(x >= start + width * 0.5))
}

fn diff_text_column(state: &AppState, side: u8, x: f64, scroll_x: f64) -> usize {
    let layout = layout::Layout::for_state(state);
    let width = diff_content_width(state, (layout.center.width - 20.0).max(0.0));
    let start = f64::from(layout.center.x) - scroll_x;
    let text_start = if state.diff_split {
        start + if side == 0 { 44.0 } else { width * 0.5 + 45.0 }
    } else {
        start + 90.0
    };
    ((x - text_start) / f64::from(DIFF_CHAR_WIDTH))
        .floor()
        .max(0.0)
        .to_usize()
        .unwrap_or(0)
}

fn diff_text_point(
    state: &AppState,
    meta: &generated::SignalMeta,
    scroll_x: f64,
) -> Option<(usize, u8, usize)> {
    let row = diff_row_at(state, meta.y)?;
    let side = diff_text_side(state, row, meta.x, scroll_x)?;
    let column = diff_text_column(state, side, meta.x, scroll_x);
    Some((row, side, column))
}

fn diff_text_selection_action(
    state: &AppState,
    item: &str,
    side: Option<u8>,
    meta: &generated::SignalMeta,
    scroll_x: f64,
) -> Option<UiAction> {
    let row = diff_row_index(item)?;
    let side = side.or_else(|| diff_text_side(state, row, meta.x, scroll_x))?;
    Some(UiAction::BeginDiffTextSelection {
        row,
        side,
        column: diff_text_column(state, side, meta.x, scroll_x),
        clicks: u8::try_from(meta.clicks).unwrap_or(u8::MAX).max(1),
    })
}

fn detail_scope(state: &AppState) -> Option<DiffScope> {
    if let Some((oldest, newest)) = state.selection_endpoints() {
        Some(DiffScope::CommitRange { oldest, newest })
    } else {
        state.selected_commit.clone().map(DiffScope::Commit)
    }
}

fn detail_file_action(state: &AppState, item: &str) -> Option<UiAction> {
    let path = item.strip_prefix("detail:file:").map(PathBuf::from)?;
    Some(UiAction::SelectFile {
        path,
        scope: detail_scope(state)?,
    })
}

fn detail_file_context_action(state: &AppState, item: &str) -> Option<UiAction> {
    let path = item.strip_prefix("detail:file:").map(PathBuf::from)?;
    let commit = state
        .selected_commit
        .clone()
        .or_else(|| state.selection_endpoints().map(|(_oldest, newest)| newest))?;
    Some(UiAction::OpenFileContext {
        path,
        scope: FileContextScope::Committed(commit),
    })
}

fn detail_commit_id(item: &str) -> Option<String> {
    item.strip_prefix("detail:parent:")
        .or_else(|| item.strip_prefix("detail:commit:"))
        .map(str::to_owned)
}

fn graph_drop_action(item: &str, source_item: &str) -> Option<UiAction> {
    let (target, target_tag) = graph_ref_identity(item)?;
    let (source, source_tag) = graph_ref_identity(source_item)?;
    (source != target).then(|| UiAction::OpenDropMenu {
        source: source.to_owned(),
        source_tag,
        target: target.to_owned(),
        target_tag,
    })
}

fn palette_index(item: &str) -> Option<usize> {
    item.strip_prefix("palette:")?
        .split(':')
        .next()?
        .parse()
        .ok()
}

fn preference_action(item: &str) -> Option<UiAction> {
    match item {
        "initialize_gitflow" => Some(UiAction::InitializeGitflow),
        "apply_sparse_checkout" => Some(UiAction::ApplySparseCheckout),
        "disable_sparse_checkout" => Some(UiAction::DisableSparseCheckout),
        "add_lfs_pattern" => Some(UiAction::AddLfsPattern),
        "open_external_editor" => Some(UiAction::OpenExternalEditor),
        "open_external_terminal" => Some(UiAction::OpenExternalTerminal),
        pattern if pattern.starts_with("remove_lfs_pattern:") => Some(UiAction::RemoveLfsPattern(
            pattern["remove_lfs_pattern:".len()..].to_owned(),
        )),
        _ => None,
    }
}

fn overlay_row_action(state: &AppState, item: &str) -> Option<UiAction> {
    if let Some(label) = item
        .strip_prefix("menu:")
        .or_else(|| item.strip_prefix("submenu:"))
    {
        return state
            .context_menu()?
            .entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Item {
                    label: candidate,
                    action,
                    enabled,
                } => (*enabled && candidate == label).then(|| action.clone()),
                MenuEntry::Submenu { entries, .. } => entries
                    .iter()
                    .find_map(|(candidate, action)| (candidate == label).then(|| action.clone())),
                MenuEntry::Separator => None,
            });
    }
    if let Some(branch) = item.strip_prefix("branch:") {
        return Some(UiAction::CheckoutBranch(branch.to_owned()));
    }
    if let Some(index) = item
        .strip_prefix("tab:")
        .and_then(|value| value.parse().ok())
    {
        return Some(UiAction::SelectTab(index));
    }
    let label = item.split_once(':')?.1;
    match item.split_once(':')?.0 {
        "lfs" => match label {
            "Checkout all LFS files" => Some(UiAction::LfsCheckout),
            "Pull all LFS files" => Some(UiAction::LfsPull),
            "Push all LFS files" => Some(UiAction::LfsPush),
            "Prune local LFS" => Some(UiAction::LfsPrune),
            _ => None,
        },
        "actions" => match label {
            "Fetch all remotes" => Some(UiAction::Fetch),
            "Create branch" => Some(UiAction::ToggleCreateBranch),
            "Stash changes" => Some(UiAction::Stash),
            _ => None,
        },
        "commit" => Some(UiAction::CommitAndPush),
        "pull" => match label {
            "Pull (fast-forward only)" => Some(UiAction::SetPullOperation(
                crate::git::models::PullOperation::FastForwardOnly,
            )),
            "Pull (rebase)" => Some(UiAction::SetPullOperation(
                crate::git::models::PullOperation::Rebase,
            )),
            "Fetch all remotes" => Some(UiAction::SetPullOperation(
                crate::git::models::PullOperation::FetchAll,
            )),
            _ => Some(UiAction::SetPullOperation(
                crate::git::models::PullOperation::FastForward,
            )),
        },
        "diff" => diff_selection_action(state, label),
        "notification" => Some(UiAction::DismissOverlay),
        _ => None,
    }
}

fn diff_selection_action(state: &AppState, label: &str) -> Option<UiAction> {
    let request = state.selected_file.as_ref()?;
    let diff = state.diff.as_ref()?;
    let mut indices = state.diff_selected_rows.iter().copied().collect::<Vec<_>>();
    indices.sort_unstable();
    let (lines, copied): (Vec<_>, Vec<_>) = indices
        .into_iter()
        .filter_map(|index| diff.rows.get(index))
        .filter(|row| !matches!(row.kind, DiffRowKind::Context | DiffRowKind::Hunk))
        .map(|row| {
            (
                crate::git::models::DiffLineSelection {
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
        .unzip();
    match label.to_ascii_lowercase().as_str() {
        "discard selection" => Some(UiAction::DiscardDiffLines {
            path: request.path.clone(),
            lines,
        }),
        "stage selection" | "stage selected lines" => Some(UiAction::StageDiffLines {
            path: request.path.clone(),
            lines,
        }),
        "unstage selection" | "unstage selected lines" => Some(UiAction::UnstageDiffLines {
            path: request.path.clone(),
            lines,
        }),
        "copy" => Some(UiAction::CopyDiffLines(copied)),
        _ => None,
    }
}

fn overlay_focus_action(state: &AppState, slot: u8) -> Option<UiAction> {
    match (&state.overlay, slot) {
        (Overlay::CreateBranch, 1) => Some(UiAction::FocusCreateBranch),
        (Overlay::RenameBranch(_), 1) => Some(UiAction::FocusRenameBranch),
        (Overlay::CreateTag(_), 1) => Some(UiAction::FocusCreateTagName),
        (Overlay::CreateTag(_), 2) => Some(UiAction::FocusCreateTagMessage),
        (Overlay::EditCommitMessage(_), 1) => Some(UiAction::FocusEditMessageSummary),
        (Overlay::EditCommitMessage(_), 2) => Some(UiAction::FocusEditMessageBody),
        _ => None,
    }
}

fn overlay_submit_action(state: &AppState) -> Option<UiAction> {
    match &state.overlay {
        Overlay::CreateBranch => Some(UiAction::CreateBranch),
        Overlay::RenameBranch(_) => Some(UiAction::RenameBranch),
        Overlay::CreateTag(_) => Some(UiAction::CreateTag),
        Overlay::EditCommitMessage(_) => Some(UiAction::ConfirmEditMessage),
        _ => None,
    }
}

/// Maps a graph row's stable domain key to the existing semantic action model.
pub(crate) fn graph_row_action(state: &AppState, item: &str) -> Option<UiAction> {
    if item == "wip" {
        return Some(UiAction::SelectWip);
    }
    let snapshot = state.snapshot.as_ref()?;
    if item.starts_with("worktree-wip:") {
        return snapshot
            .worktrees
            .iter()
            .find(|worktree| worktree_wip_key(worktree) == item)
            .and_then(|worktree| worktree.target.clone())
            .map(UiAction::SelectCommit);
    }
    snapshot
        .commits
        .iter()
        .find(|commit| commit.id == item)
        .map(|commit| UiAction::SelectCommit(commit.id.clone()))
}

/// Maps a graph row context signal without losing its keyed commit identity.
pub(crate) fn graph_row_context_action(state: &AppState, item: &str) -> Option<UiAction> {
    match graph_row_action(state, item)? {
        UiAction::SelectCommit(id) => Some(UiAction::OpenCommitContext(id)),
        _ => None,
    }
}

/// Returns the branch/tag identity encoded by an interactive graph chip key.
pub(crate) fn graph_ref_identity(item: &str) -> Option<(&str, bool)> {
    if let Some(name) = item.strip_prefix("local:") {
        Some((name, false))
    } else if let Some(name) = item.strip_prefix("remote:") {
        Some((name, false))
    } else {
        item.strip_prefix("tag:").map(|name| (name, true))
    }
}

/// Maps graph-chip activation to the existing branch/tag/WIP actions.
pub(crate) fn graph_ref_action(item: &str) -> Option<UiAction> {
    if item == "wip" {
        return Some(UiAction::SelectWip);
    }
    graph_ref_identity(item).map(|(name, tag)| {
        if tag {
            UiAction::TagClick(name.to_owned())
        } else {
            UiAction::BranchClick(name.to_owned())
        }
    })
}

/// Maps graph-chip context signals to existing branch/tag context actions.
pub(crate) fn graph_ref_context_action(item: &str) -> Option<UiAction> {
    graph_ref_identity(item).map(|(name, tag)| {
        if tag {
            UiAction::OpenTagContext(name.to_owned())
        } else {
            UiAction::OpenBranchContext(name.to_owned())
        }
    })
}

const fn color_alpha(color: u32, alpha: u8) -> u32 {
    let [red, green, blue, _] = color.to_le_bytes();
    u32::from_le_bytes([red, green, blue, alpha])
}

fn change_colors(kind: ChangeKind) -> (u32, u32) {
    match kind {
        ChangeKind::Added => (GREEN, GREEN_SOFT),
        ChangeKind::Modified | ChangeKind::TypeChanged => (ORANGE, ORANGE_SOFT),
        ChangeKind::Deleted | ChangeKind::Conflicted => (RED, RED_SOFT),
        ChangeKind::Renamed => (rgba(222, 196, 80, 255), YELLOW_MARK),
    }
}

/// Formats one working file's display path — breadcrumb-style in tree mode —
/// including the old path for renames.
pub(crate) fn working_path_label(file: &WorkingFile, tree: bool) -> String {
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

fn working_rows(state: &AppState) -> (Vec<UnstagedFilesItem>, Vec<StagedFilesItem>) {
    let Some(snapshot) = &state.snapshot else {
        return (Vec::new(), Vec::new());
    };
    let unstaged = snapshot
        .working
        .files
        .iter()
        .filter_map(|file| {
            let kind = file.unstaged?;
            let (kind_fg, kind_bg) = change_colors(kind);
            Some(UnstagedFilesItem {
                key: Some(format!("unstaged:{}", file.path.display())),
                marker: kind.marker().to_owned(),
                path: working_path_label(file, state.path_tree),
                selected: state.selected_working_files.contains(&file.path),
                conflict: kind == ChangeKind::Conflicted,
                staged: false,
                kind_bg,
                kind_fg,
                open: state
                    .selected_file
                    .as_ref()
                    .is_some_and(|r| r.path == file.path),
            })
        })
        .collect();
    let staged = snapshot
        .working
        .files
        .iter()
        .filter_map(|file| {
            let kind = file.staged?;
            let (kind_fg, kind_bg) = change_colors(kind);
            Some(StagedFilesItem {
                key: Some(format!("staged:{}", file.path.display())),
                marker: kind.marker().to_owned(),
                path: working_path_label(file, state.path_tree),
                selected: state.selected_working_files.contains(&file.path),
                conflict: kind == ChangeKind::Conflicted,
                staged: true,
                kind_bg,
                kind_fg,
                open: state
                    .selected_file
                    .as_ref()
                    .is_some_and(|r| r.path == file.path),
            })
        })
        .collect();
    (unstaged, staged)
}

enum DetailTreeRow<'a> {
    Folder {
        path: PathBuf,
        name: String,
        depth: usize,
    },
    File {
        path: &'a Path,
        change: Option<&'a FileChange>,
        depth: usize,
    },
}

fn detail_rows(state: &AppState) -> Vec<DetailFilesItem> {
    let files = detail_source_rows(state);
    if state.path_tree {
        return detail_tree_rows(&files)
            .into_iter()
            .map(|row| match row {
                DetailTreeRow::Folder { path, name, depth } => DetailFilesItem {
                    key: Some(format!("detail:folder:{}", path.display())),
                    marker: String::new(),
                    prefix: String::new(),
                    name,
                    additions: String::new(),
                    deletions: String::new(),
                    indent: depth.to_f64().unwrap_or(0.0) * 14.0,
                    folder: true,
                    changed: false,
                    selected: false,
                    marker_tone: DIM,
                    marker_bg: TRANSPARENT,
                    tooltip: path.display().to_string(),
                },
                DetailTreeRow::File {
                    path,
                    change,
                    depth,
                } => detail_file(state, path, change, depth, false),
            })
            .collect();
    }
    files
        .into_iter()
        .map(|(path, change)| detail_file(state, path, change, 0, true))
        .collect()
}

fn detail_source_rows(state: &AppState) -> Vec<(&Path, Option<&FileChange>)> {
    if state.selected_commits.len() > 1 {
        return state.range_detail.as_ref().map_or_else(Vec::new, |range| {
            range
                .files
                .iter()
                .map(|file| (file.path.as_path(), Some(file)))
                .collect()
        });
    }
    let Some(detail) = &state.detail else {
        return Vec::new();
    };
    if state.view_all_files
        && let Some(all_files) = &detail.all_files
    {
        return all_files
            .iter()
            .map(|path| {
                let change = detail
                    .files
                    .binary_search_by(|file| file.path.as_path().cmp(path.as_path()))
                    .ok()
                    .map(|index| &detail.files[index]);
                (path.as_path(), change)
            })
            .collect();
    }
    detail
        .files
        .iter()
        .map(|file| (file.path.as_path(), Some(file)))
        .collect()
}

fn detail_tree_rows<'a>(files: &[(&'a Path, Option<&'a FileChange>)]) -> Vec<DetailTreeRow<'a>> {
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
                rows.push(DetailTreeRow::File {
                    path,
                    change,
                    depth,
                });
                continue;
            }
            directory.push(name);
            if directories.insert(directory.clone()) {
                rows.push(DetailTreeRow::Folder {
                    path: directory.clone(),
                    name: name.to_string_lossy().into_owned(),
                    depth,
                });
            }
            depth += 1;
        }
    }
    rows
}

fn detail_file(
    state: &AppState,
    path: &Path,
    change: Option<&FileChange>,
    depth: usize,
    show_prefix: bool,
) -> DetailFilesItem {
    let (prefix, name) = detail_path_parts(state, path, depth, show_prefix);
    let (marker, marker_tone, marker_bg, additions, deletions) = change.map_or_else(
        || {
            (
                String::new(),
                DIM,
                TRANSPARENT,
                String::new(),
                String::new(),
            )
        },
        |file| {
            let (tone, background) = detail_marker_colors(file.kind);
            (
                file.kind.marker().to_owned(),
                tone,
                background,
                (file.additions > 0)
                    .then(|| format!("+{}", file.additions))
                    .unwrap_or_default(),
                (file.deletions > 0)
                    .then(|| format!("-{}", file.deletions))
                    .unwrap_or_default(),
            )
        },
    );
    DetailFilesItem {
        key: Some(format!("detail:file:{}", path.display())),
        marker,
        prefix,
        name,
        additions,
        deletions,
        indent: depth.to_f64().unwrap_or(0.0) * 14.0,
        folder: false,
        changed: change.is_some(),
        selected: state
            .selected_file
            .as_ref()
            .is_some_and(|request| request.path == path),
        marker_tone,
        marker_bg,
        tooltip: path.display().to_string(),
    }
}

fn detail_path_parts(
    state: &AppState,
    path: &Path,
    depth: usize,
    show_prefix: bool,
) -> (String, String) {
    let detail_width = layout::Layout::for_state(state)
        .detail
        .map_or(710.0, |detail| detail.width);
    let budget = ((detail_width - 142.0 - depth.to_f32().unwrap_or(f32::MAX) * 14.0).max(0.0)
        / 7.2)
        .floor()
        .to_usize()
        .unwrap_or(0);
    let raw_name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let name_count = raw_name.chars().count();
    let prefix = if show_prefix && budget > name_count.saturating_add(2) {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| {
                let raw = format!("{}/", parent.display());
                elide_path_front(&raw, budget.saturating_sub(name_count))
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    (prefix, elide_path_front(&raw_name, budget))
}

fn elide_path_front(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_owned();
    }
    if limit == 0 {
        return String::new();
    }
    let tail = value
        .chars()
        .rev()
        .take(limit.saturating_sub(1))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}

fn detail_marker_colors(kind: ChangeKind) -> (u32, u32) {
    match kind {
        ChangeKind::Added => (GREEN, GREEN_SOFT),
        ChangeKind::Deleted | ChangeKind::Conflicted => (RED, RED_SOFT),
        ChangeKind::Renamed => (MUTED, rgba(24, 24, 24, 255)),
        ChangeKind::Modified | ChangeKind::TypeChanged => (ORANGE, ORANGE_SOFT),
    }
}

fn detail_parent_rows(state: &AppState) -> Vec<DetailParentsItem> {
    // HEAD's commit_detail draws nothing after "PARENT:" for a root commit,
    // so an empty parent list maps to an empty row set.
    state.detail.as_ref().map_or_else(Vec::new, |detail| {
        detail
            .parents
            .iter()
            .map(|parent| DetailParentsItem {
                key: Some(format!("detail:parent:{parent}")),
                label: parent.chars().take(7).collect(),
            })
            .collect()
    })
}

fn detail_conflict_rows(state: &AppState) -> Vec<DetailConflictsItem> {
    state.detail.as_ref().map_or_else(Vec::new, |detail| {
        detail
            .conflicts
            .iter()
            .take(5)
            .map(|path| DetailConflictsItem {
                key: Some(format!("detail:conflict:{}", path.display())),
                path: path.display().to_string(),
            })
            .collect()
    })
}

fn detail_commit_rows(state: &AppState) -> Vec<DetailCommitsItem> {
    state.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
        snapshot
            .commits
            .iter()
            .filter(|commit| state.selected_commits.contains(&commit.id))
            .take(8)
            .map(|commit| DetailCommitsItem {
                key: Some(format!("detail:commit:{}", commit.id)),
                sha: commit.short_id.clone(),
                subject: commit.subject.clone(),
            })
            .collect()
    })
}

fn detail_counts(files: &[FileChange]) -> (usize, usize) {
    let mut modified = 0;
    let mut added = 0;
    for file in files {
        match file.kind {
            ChangeKind::Added => added += 1,
            ChangeKind::Modified => modified += 1,
            _ => {}
        }
    }
    (modified, added)
}

fn diff_render_status(state: &AppState) -> (bool, &'static str) {
    let Some(diff) = &state.diff else {
        return (false, "Loading file diff…");
    };
    if diff.binary {
        return if state.diff_file_view {
            (false, "Binary file — textual view unavailable")
        } else {
            (false, "Binary file — textual diff unavailable")
        };
    }
    if state.diff_file_view {
        let Some(content) = &diff.content else {
            return (false, "File is absent in the selected revision");
        };
        if content.len() > MAX_INLINE_FILE_BYTES {
            return (false, "File is too large for inline rendering");
        }
        return if content.is_empty() {
            (false, "File is empty in the selected revision")
        } else {
            (true, "")
        };
    }
    if diff.rows.len() > MAX_INLINE_DIFF_ROWS {
        return (false, "Diff is too large for inline rendering");
    }
    if diff.rows.is_empty() {
        (false, "No changes in the selected scope")
    } else {
        (true, "")
    }
}

fn diff_content_width(state: &AppState, viewport_width: f32) -> f64 {
    let minimum = f64::from(viewport_width.max(320.0));
    let Some(diff) = &state.diff else {
        return minimum;
    };
    if state.diff_split && !state.diff_file_view {
        // Split mode divides the viewport: two 50% panes with the divider at
        // the center and code clipped per pane, matching the previous UI.
        return minimum;
    }
    let columns = if state.diff_file_view {
        diff.content.as_deref().map_or(0, |content| {
            content
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(0)
        })
    } else {
        diff.rows
            .iter()
            .map(|row| {
                row.old_text
                    .chars()
                    .count()
                    .max(row.new_text.chars().count())
            })
            .max()
            .unwrap_or(0)
    }
    .to_f64()
    .unwrap_or(0.0);
    minimum.max(columns * DIFF_CHAR_WIDTH + 100.0)
}

fn diff_rows(state: &AppState) -> Vec<DiffRowsItem> {
    let (renderable, _) = diff_render_status(state);
    if !renderable {
        return Vec::new();
    }
    let Some(diff) = &state.diff else {
        return Vec::new();
    };
    let total = if state.diff_file_view {
        diff.content
            .as_deref()
            .map_or(0, |content| content.lines().count())
    } else {
        diff.rows.len()
    };
    let (styled_start, styled_end) = diff_styled_window(state, total);
    if state.diff_file_view {
        return diff.content.as_deref().map_or_else(Vec::new, |content| {
            content
                .lines()
                .enumerate()
                .map(|(index, line)| DiffRowsItem {
                    key: Some(format!("diff:file:{}:{index}", diff.path.display())),
                    old_no: String::new(),
                    new_no: index.saturating_add(1).to_string(),
                    hunk_text: String::new(),
                    prefix: String::new(),
                    prefix_tone: MUTED,
                    old_tone: DIM,
                    new_tone: DIM,
                    old_bg: TRANSPARENT,
                    new_bg: TRANSPARENT,
                    unified_bg: if index % 2 == 0 {
                        TRANSPARENT
                    } else {
                        FILE_ROW_ALT
                    },
                    old_empty: false,
                    new_empty: false,
                    split: false,
                    hunk: false,
                    selected: false,
                    old_runs: Vec::new(),
                    new_runs: Vec::new(),
                    unified_runs: syntax_runs(
                        &diff.path,
                        line,
                        index >= styled_start && index < styled_end,
                    ),
                    old_marks: Vec::new(),
                    new_marks: Vec::new(),
                    unified_marks: Vec::new(),
                    interactive: false,
                })
                .collect()
        });
    }

    let search_results = state.diff_search_results();
    diff.rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            diff_row_item(
                state,
                diff.path.as_path(),
                index,
                row,
                index >= styled_start && index < styled_end,
                &search_results,
            )
        })
        .collect()
}

fn diff_map_rows(state: &AppState, viewport_height: f32) -> Vec<DiffMapItem> {
    if state.diff_file_view {
        return Vec::new();
    }
    let Some(diff) = &state.diff else {
        return Vec::new();
    };
    let total = diff.rows.len();
    if total == 0 {
        return Vec::new();
    }
    let search_height = if state.focus == FocusField::DiffSearch {
        38.0
    } else {
        0.0
    };
    let map_height = (viewport_height - 78.0 - 24.0 - search_height).max(1.0);
    let extent = f64::from(map_height) / total.to_f64().unwrap_or(1.0);
    let current_hunk = diff.hunks.get(state.current_hunk).copied();
    diff.rows
        .iter()
        .enumerate()
        .map(|(index, row)| DiffMapItem {
            key: Some(format!("diff-map:{index}")),
            tone: match row.kind {
                DiffRowKind::Context => rgba(48, 48, 48, 255),
                DiffRowKind::Changed => ORANGE_SOFT,
                DiffRowKind::Added => GREEN_SOFT,
                DiffRowKind::Deleted => RED_SOFT,
                DiffRowKind::Hunk => PURPLE_SOFT,
            },
            extent,
            current: state.diff_selected_rows.contains(&index) || current_hunk == Some(index),
        })
        .collect()
}

fn diff_styled_window(state: &AppState, total: usize) -> (usize, usize) {
    const OVERSCAN: usize = 12;
    let first = (state.diff_scroll.max(0.0) / 20.0)
        .to_usize()
        .unwrap_or(usize::MAX)
        .min(total);
    let visible = usize::try_from(state.height)
        .unwrap_or(usize::MAX)
        .checked_div(20)
        .unwrap_or(usize::MAX)
        .saturating_add(OVERSCAN * 2);
    let start = first.saturating_sub(OVERSCAN);
    (start, start.saturating_add(visible).min(total))
}

fn diff_row_item(
    state: &AppState,
    path: &Path,
    index: usize,
    row: &DiffRow,
    styled: bool,
    search_results: &[(usize, u8, usize, usize)],
) -> DiffRowsItem {
    if row.kind == DiffRowKind::Hunk {
        return DiffRowsItem {
            key: Some(format!("diff:row:{}:{index}", path.display())),
            old_no: String::new(),
            new_no: String::new(),
            hunk_text: row.new_text.to_uppercase(),
            prefix: String::new(),
            prefix_tone: PURPLE,
            old_tone: DIM,
            new_tone: DIM,
            old_bg: PURPLE_SOFT,
            new_bg: PURPLE_SOFT,
            unified_bg: PURPLE_SOFT,
            old_empty: false,
            new_empty: false,
            split: state.diff_split,
            hunk: true,
            selected: false,
            old_runs: Vec::new(),
            new_runs: Vec::new(),
            unified_runs: Vec::new(),
            old_marks: Vec::new(),
            new_marks: Vec::new(),
            unified_marks: Vec::new(),
            interactive: true,
        };
    }

    let (old_bg, new_bg, unified_bg, prefix, prefix_tone, old_tone, new_tone) = match row.kind {
        DiffRowKind::Context => (TRANSPARENT, TRANSPARENT, TRANSPARENT, " ", MUTED, DIM, DIM),
        DiffRowKind::Changed => (RED_SOFT, GREEN_SOFT, ORANGE_SOFT, "~", ORANGE, RED, GREEN),
        DiffRowKind::Added => (TRANSPARENT, GREEN_SOFT, GREEN_SOFT, "+", GREEN, DIM, GREEN),
        DiffRowKind::Deleted => (RED_SOFT, TRANSPARENT, RED_SOFT, "-", RED, RED, DIM),
        DiffRowKind::Hunk => unreachable!("hunk rows return above"),
    };
    let mut old_runs = Vec::new();
    let mut new_runs = Vec::new();
    let mut unified_runs = Vec::new();
    let mut old_marks = Vec::new();
    let mut new_marks = Vec::new();
    let mut unified_marks = Vec::new();
    if state.diff_split {
        old_runs = syntax_runs(path, &row.old_text, styled);
        new_runs = syntax_runs(path, &row.new_text, styled);
        old_marks = diff_marks(
            state,
            index,
            0,
            &row.old_text,
            row.old_mark,
            RED_INTRALINE,
            search_results,
        );
        new_marks = diff_marks(
            state,
            index,
            1,
            &row.new_text,
            row.new_mark,
            GREEN_INTRALINE,
            search_results,
        );
    } else {
        let (side, text, intraline, tone) = match row.kind {
            DiffRowKind::Deleted => (0, row.old_text.as_str(), row.old_mark, RED_INTRALINE),
            DiffRowKind::Changed => (1, row.new_text.as_str(), row.new_mark, ORANGE_INTRALINE),
            DiffRowKind::Added | DiffRowKind::Context => {
                (1, row.new_text.as_str(), row.new_mark, GREEN_INTRALINE)
            }
            DiffRowKind::Hunk => unreachable!("hunk rows return above"),
        };
        unified_runs = syntax_runs(path, text, styled);
        unified_marks = diff_marks(state, index, side, text, intraline, tone, search_results);
    }
    DiffRowsItem {
        key: Some(format!("diff:row:{}:{index}", path.display())),
        old_no: row
            .old_number
            .map(|number| number.to_string())
            .unwrap_or_default(),
        new_no: row
            .new_number
            .map(|number| number.to_string())
            .unwrap_or_default(),
        hunk_text: String::new(),
        prefix: prefix.to_owned(),
        prefix_tone,
        old_tone,
        new_tone,
        old_bg,
        new_bg,
        unified_bg,
        old_empty: row.kind == DiffRowKind::Added,
        new_empty: row.kind == DiffRowKind::Deleted,
        split: state.diff_split,
        hunk: false,
        selected: state.diff_selected_rows.contains(&index),
        old_runs,
        new_runs,
        unified_runs,
        old_marks,
        new_marks,
        unified_marks,
        interactive: true,
    }
}

fn syntax_runs(path: &Path, line: &str, highlighted: bool) -> Vec<DiffRowsOldRunsItem> {
    if !highlighted || line.is_empty() {
        return vec![DiffRowsOldRunsItem {
            key: Some("syntax:plain".to_owned()),
            content: line.to_owned(),
            tone: if line.is_empty() { TEXT } else { MUTED },
        }];
    }
    static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
    static THEMES: LazyLock<ThemeSet> = LazyLock::new(ThemeSet::load_defaults);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("txt");
    let syntax = SYNTAXES
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let Some(theme) = THEMES
        .themes
        .get("base16-ocean.dark")
        .or_else(|| THEMES.themes.values().next())
    else {
        return vec![DiffRowsOldRunsItem {
            key: Some("syntax:fallback".to_owned()),
            content: line.to_owned(),
            tone: MUTED,
        }];
    };
    let mut highlighter = HighlightLines::new(syntax, theme);
    highlighter.highlight_line(line, &SYNTAXES).map_or_else(
        |_| {
            vec![DiffRowsOldRunsItem {
                key: Some("syntax:error".to_owned()),
                content: line.to_owned(),
                tone: MUTED,
            }]
        },
        |ranges| {
            let mut offset = 0;
            ranges
                .into_iter()
                .map(|(style, token)| {
                    let item = DiffRowsOldRunsItem {
                        key: Some(format!("syntax:{offset}")),
                        content: token.to_owned(),
                        tone: rgba(
                            style.foreground.r,
                            style.foreground.g,
                            style.foreground.b,
                            style.foreground.a,
                        ),
                    };
                    offset += token.len();
                    item
                })
                .collect()
        },
    )
}

fn diff_marks(
    state: &AppState,
    row: usize,
    side: u8,
    text: &str,
    intraline: Option<(usize, usize)>,
    intraline_tone: u32,
    search_results: &[(usize, u8, usize, usize)],
) -> Vec<DiffRowsOldMarksItem> {
    let mut marks = Vec::new();
    if let Some((start_byte, end_byte)) = intraline {
        let start = text
            .get(..start_byte)
            .map_or(0, |prefix| prefix.chars().count());
        let end = text
            .get(..end_byte)
            .map_or(start.saturating_add(1), |prefix| prefix.chars().count());
        push_diff_mark(
            &mut marks,
            format!("intraline:{start}:{end}"),
            start,
            end.max(start.saturating_add(1)),
            intraline_tone,
            false,
        );
    }
    if let Some((start, end)) = diff_text_selection(state, row, side, text.chars().count()) {
        push_diff_mark(
            &mut marks,
            format!("selection:{start}:{end}"),
            start,
            end,
            TEXT_SELECTION,
            false,
        );
    }
    for (match_index, (match_row, match_side, start, end)) in search_results.iter().enumerate() {
        if *match_row != row || *match_side != side {
            continue;
        }
        let current = match_index == state.diff_search_cursor;
        push_diff_mark(
            &mut marks,
            format!("search:{match_index}:{start}:{end}"),
            *start,
            *end,
            if current {
                YELLOW_MARK_CURRENT
            } else {
                YELLOW_MARK
            },
            current,
        );
    }
    marks
}

fn diff_text_selection(
    state: &AppState,
    row: usize,
    side: u8,
    line_length: usize,
) -> Option<(usize, usize)> {
    let ((start_row, selected_side, start_column), (end_row, _, end_column)) =
        state.diff_text_selection?;
    if selected_side != side || row < start_row.min(end_row) || row > start_row.max(end_row) {
        return None;
    }
    let (first_row, first_column, last_row, last_column) =
        if (start_row, start_column) <= (end_row, end_column) {
            (start_row, start_column, end_row, end_column)
        } else {
            (end_row, end_column, start_row, start_column)
        };
    let start = if row == first_row { first_column } else { 0 }.min(line_length);
    let end = if row == last_row {
        last_column
    } else {
        line_length
    }
    .min(line_length);
    (end > start).then_some((start, end))
}

fn push_diff_mark(
    marks: &mut Vec<DiffRowsOldMarksItem>,
    key: String,
    start: usize,
    end: usize,
    tone: u32,
    current: bool,
) {
    if end <= start {
        return;
    }
    marks.push(DiffRowsOldMarksItem {
        key: Some(key),
        x: 6.0 + start.to_f64().unwrap_or(0.0) * DIFF_CHAR_WIDTH,
        width: end.saturating_sub(start).to_f64().unwrap_or(1.0) * DIFF_CHAR_WIDTH,
        tone,
        current,
    });
}

fn recent_rows(state: &AppState) -> Vec<RecentReposItem> {
    state
        .settings
        .recent_repos
        .iter()
        .map(|recent| RecentReposItem {
            key: Some(recent.path.display().to_string()),
            name: recent
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Repository")
                .to_owned(),
            path: recent.path.display().to_string(),
            badge: format_date(recent.last_opened),
        })
        .collect()
}
fn preference_nav(state: &AppState) -> Vec<PreferenceNavItem> {
    let mut rows = vec![PreferenceNavItem {
        key: Some("nav_header:preferences".to_owned()),
        label: "PREFERENCES".to_owned(),
        header: true,
        selected: false,
        disabled: false,
        sub: false,
        page: false,
    }];
    rows.extend(
        GLOBAL_PREFERENCE_PAGES
            .iter()
            .map(|page| PreferenceNavItem {
                key: Some(format!("pref_page:{}", page)),
                label: (*page).to_owned(),
                header: false,
                selected: state.preference_page == *page,
                disabled: false,
                sub: false,
                page: true,
            }),
    );
    let repo_disabled = state.snapshot.is_none();
    rows.push(PreferenceNavItem {
        key: Some("nav_header:repo_preferences".to_owned()),
        label: "REPO-SPECIFIC PREFERENCES".to_owned(),
        header: true,
        selected: false,
        disabled: repo_disabled,
        sub: false,
        page: false,
    });
    if let Some(snapshot) = &state.snapshot {
        rows.push(PreferenceNavItem {
            key: Some("nav_header:repo_name".to_owned()),
            label: format!("REPO: {}", snapshot.name.to_uppercase()),
            header: false,
            selected: false,
            disabled: false,
            sub: true,
            page: false,
        });
    } else {
        rows.push(PreferenceNavItem {
            key: Some("nav_header:repo_name_disabled".to_owned()),
            label: "REPO: NONE".to_owned(),
            header: false,
            selected: false,
            disabled: true,
            sub: true,
            page: false,
        });
    }
    rows.extend(REPO_PREFERENCE_PAGES.iter().map(|page| PreferenceNavItem {
        key: Some(format!("pref_page:{}", page)),
        label: (*page).to_owned(),
        header: false,
        selected: state.preference_page == *page,
        disabled: repo_disabled,
        sub: false,
        page: true,
    }));
    rows
}

fn pref_profiles(state: &AppState) -> Vec<PrefProfilesItem> {
    state
        .settings
        .profiles
        .iter()
        .map(|profile| PrefProfilesItem {
            key: Some(profile.name.clone()),
            name: profile.name.clone(),
            author: profile.author_name.clone(),
            email: profile.author_email.clone(),
            selected: state.settings.selected_profile.as_ref() == Some(&profile.name),
        })
        .collect()
}

fn preference_rows(page: &str, settings: &Settings) -> Vec<PreferenceRowsItem> {
    match page {
        "General" => vec![
            pref_number(
                "auto_fetch_minutes",
                "Auto-Fetch Interval",
                format!("{} min", settings.auto_fetch_minutes),
            ),
            pref_toggle("auto_prune", "Auto-Prune", settings.auto_prune),
            pref_number(
                "initial_commits",
                "Initial Commits in Graph",
                settings.initial_commits.to_string(),
            ),
            pref_toggle(
                "lazy_load_commits",
                "Lazy Load Commits in Graph",
                settings.lazy_load_commits,
            ),
        ],
        "Profiles" => {
            Vec::new() // Profile info is populated in sync_scalars via pref_profiles, but we can leave this empty.
        }
        "SSH" => vec![
            pref_toggle(
                "use_local_ssh_agent",
                "Use local SSH agent",
                settings.use_local_ssh_agent,
            ),
            pref_field_browse(
                "ssh_private_key",
                "Private key",
                &settings.ssh_private_key,
                "Browse",
            ),
            pref_field_browse(
                "ssh_public_key",
                "Public key",
                &settings.ssh_public_key,
                "Browse",
            ),
        ],
        "External Tools" => vec![
            pref_field(
                "external_editor",
                "External editor command",
                &settings.external_editor,
            ),
            pref_field(
                "external_terminal",
                "External terminal command",
                &settings.external_terminal,
            ),
            pref_toggle(
                "show_external_tool_arguments",
                "Show arguments when launching",
                settings.show_external_tool_arguments,
            ),
            pref_button(
                "open_external_editor",
                "External editor",
                "Open external editor",
            ),
            pref_button(
                "open_external_terminal",
                "External terminal",
                "Open external terminal",
            ),
        ],
        "Commit Signing" => vec![
            pref_field_browse(
                "gpg_program",
                "GPG program",
                &settings.gpg_program,
                "Browse",
            ),
            pref_field("gpg_key_id", "Signing key ID", &settings.gpg_key_id),
            pref_toggle(
                "sign_commits_by_default",
                "Sign commits by default",
                settings.sign_commits_by_default,
            ),
            pref_toggle(
                "sign_tags_by_default",
                "Sign tags by default",
                settings.sign_tags_by_default,
            ),
        ],
        "Notifications" => vec![
            pref_toggle(
                "notify_operation_success",
                "Operation successes",
                settings.notify_operation_success,
            ),
            pref_toggle(
                "notify_operation_failure",
                "Operation failures",
                settings.notify_operation_failure,
            ),
            pref_toggle(
                "notify_fetch_results",
                "Fetch results",
                settings.notify_fetch_results,
            ),
        ],
        "Experimental" => vec![
            pref_toggle(
                "use_git_executable",
                "Use Git executable",
                settings.use_git_executable,
            ),
            pref_field_browse(
                "git_executable",
                "Git executable path",
                &settings.git_executable,
                "Browse",
            ),
        ],
        "UI Customization" => vec![
            pref_toggle(
                "show_commit_author",
                "Show commit author avatar",
                settings.show_commit_author,
            ),
            pref_toggle(
                "show_commit_date",
                "Show commit date/time",
                settings.show_commit_date,
            ),
            pref_toggle(
                "show_commit_sha",
                "Show commit SHA",
                settings.show_commit_sha,
            ),
        ],
        "Editor" => vec![pref_number(
            "editor_font_size",
            "Font Size",
            settings.editor_font_size.to_string(),
        )],
        "In-App Terminal" => {
            vec![pref_number(
                "terminal_font_size",
                "Font Size",
                settings.terminal_font_size.to_string(),
            )]
        }
        "Encoding" => vec![pref_field(
            "default_encoding",
            "Default encoding (UTF-8 or Latin-1)",
            &settings.default_encoding,
        )],
        "Gitflow" => vec![
            pref_field(
                "gitflow_main_branch",
                "Main branch",
                &settings.gitflow_main_branch,
            ),
            pref_field(
                "gitflow_develop_branch",
                "Develop branch",
                &settings.gitflow_develop_branch,
            ),
            pref_field(
                "gitflow_feature_prefix",
                "Feature prefix",
                &settings.gitflow_feature_prefix,
            ),
            pref_field(
                "gitflow_release_prefix",
                "Release prefix",
                &settings.gitflow_release_prefix,
            ),
            pref_field(
                "gitflow_hotfix_prefix",
                "Hotfix prefix",
                &settings.gitflow_hotfix_prefix,
            ),
            pref_button("initialize_gitflow", "Gitflow", "Initialize Gitflow"),
        ],
        "LFS" => {
            let mut rows = vec![pref_field("lfs_pattern", "Tracking pattern", "")];
            rows.push(pref_button(
                "add_lfs_pattern",
                "Git LFS",
                "Add tracking pattern",
            ));
            rows.extend(settings.lfs_patterns.iter().map(|pattern| {
                pref_button(
                    &format!("remove_lfs_pattern:{}", pattern),
                    pattern,
                    "Remove",
                )
            }));
            rows
        }
        "Sparse Checkout" => vec![
            pref_field(
                "sparse_checkout_paths",
                "Paths (space-separated)",
                &settings.sparse_checkout_paths,
            ),
            pref_button("apply_sparse_checkout", "Sparse checkout", "Apply"),
            pref_button("disable_sparse_checkout", "Sparse checkout", "Disable"),
        ],
        _ => Vec::new(),
    }
}

fn pref_toggle(key: &str, label: &str, checked: bool) -> PreferenceRowsItem {
    PreferenceRowsItem {
        key: Some(key.to_owned()),
        label: label.to_owned(),
        description: String::new(),
        has_description: false,
        disabled: false,
        is_toggle: true,
        checked,
        is_number: false,
        number_value: String::new(),
        is_field: false,
        field_value: String::new(),
        field_placeholder: String::new(),
        field_browse_label: String::new(),
        has_browse: false,
        is_button: false,
        button_label: String::new(),
    }
}

fn pref_number(key: &str, label: &str, value: String) -> PreferenceRowsItem {
    PreferenceRowsItem {
        key: Some(key.to_owned()),
        label: label.to_uppercase(),
        description: String::new(),
        has_description: false,
        disabled: false,
        is_toggle: false,
        checked: false,
        is_number: true,
        number_value: value,
        is_field: false,
        field_value: String::new(),
        field_placeholder: String::new(),
        field_browse_label: String::new(),
        has_browse: false,
        is_button: false,
        button_label: String::new(),
    }
}

fn pref_field(key: &str, label: &str, value: &str) -> PreferenceRowsItem {
    PreferenceRowsItem {
        key: Some(key.to_owned()),
        label: label.to_uppercase(),
        description: String::new(),
        has_description: false,
        disabled: false,
        is_toggle: false,
        checked: false,
        is_number: false,
        number_value: String::new(),
        is_field: true,
        field_value: value.to_owned(),
        field_placeholder: String::new(),
        field_browse_label: String::new(),
        has_browse: false,
        is_button: false,
        button_label: String::new(),
    }
}

fn pref_field_browse(key: &str, label: &str, value: &str, browse: &str) -> PreferenceRowsItem {
    PreferenceRowsItem {
        key: Some(key.to_owned()),
        label: label.to_uppercase(),
        description: String::new(),
        has_description: false,
        disabled: false,
        is_toggle: false,
        checked: false,
        is_number: false,
        number_value: String::new(),
        is_field: true,
        field_value: value.to_owned(),
        field_placeholder: String::new(),
        field_browse_label: browse.to_owned(),
        has_browse: true,
        is_button: false,
        button_label: String::new(),
    }
}

fn pref_button(key: &str, label: &str, button_label: &str) -> PreferenceRowsItem {
    PreferenceRowsItem {
        key: Some(key.to_owned()),
        label: label.to_uppercase(),
        description: String::new(),
        has_description: false,
        disabled: false,
        is_toggle: false,
        checked: false,
        is_number: false,
        number_value: String::new(),
        is_field: false,
        field_value: String::new(),
        field_placeholder: String::new(),
        field_browse_label: String::new(),
        has_browse: false,
        is_button: true,
        button_label: button_label.to_owned(),
    }
}

fn overlay_rows(state: &AppState) -> Vec<OverlayRowsItem> {
    if let Some(menu) = state.context_menu() {
        return menu
            .entries
            .iter()
            .flat_map(|entry| match entry {
                MenuEntry::Item { label, enabled, .. } => vec![OverlayRowsItem {
                    key: Some(format!("menu:{}", label)),
                    icon: String::new(),
                    label: label.to_owned(),
                    detail: String::new(),
                    selected: false,
                    disabled: !enabled,
                    separator: false,
                    not_separator: true,
                    danger: is_danger(label),
                    checked: false,
                    has_children: false,
                    children: Vec::new(),
                }],
                MenuEntry::Submenu { label, entries } => {
                    let children: Vec<_> = entries
                        .iter()
                        .map(|(child_label, _)| OverlayRowsChildrenItem {
                            key: Some(format!("submenu:{}", child_label)),
                            label: child_label.clone(),
                            danger: is_danger(child_label),
                            disabled: false,
                        })
                        .collect();
                    vec![OverlayRowsItem {
                        key: Some(format!("menu:{}", label)),
                        icon: String::new(),
                        label: label.to_owned(),
                        detail: String::new(),
                        selected: false,
                        disabled: false,
                        separator: false,
                        not_separator: true,
                        danger: false,
                        checked: false,
                        has_children: !children.is_empty(),
                        children,
                    }]
                }
                MenuEntry::Separator => vec![OverlayRowsItem {
                    key: Some("separator".to_owned()),
                    icon: String::new(),
                    label: String::new(),
                    detail: String::new(),
                    selected: false,
                    disabled: false,
                    separator: true,
                    not_separator: false,
                    danger: false,
                    checked: false,
                    has_children: false,
                    children: Vec::new(),
                }],
            })
            .collect();
    }
    match &state.overlay {
        Overlay::Lfs => [
            "Checkout all LFS files",
            "Pull all LFS files",
            "Push all LFS files",
            "Prune local LFS",
        ]
        .into_iter()
        .map(|label| OverlayRowsItem {
            key: Some(format!("lfs:{}", label)),
            icon: String::new(),
            label: label.to_owned(),
            detail: String::new(),
            selected: false,
            disabled: false,
            separator: false,
            not_separator: true,
            danger: false,
            checked: false,
            has_children: false,
            children: Vec::new(),
        })
        .collect(),
        Overlay::Actions => ["Fetch all remotes", "Create branch", "Stash changes"]
            .into_iter()
            .map(|label| OverlayRowsItem {
                key: Some(format!("actions:{}", label)),
                icon: String::new(),
                label: label.to_owned(),
                detail: String::new(),
                selected: false,
                disabled: false,
                separator: false,
                not_separator: true,
                danger: false,
                checked: false,
                has_children: false,
                children: Vec::new(),
            })
            .collect(),
        Overlay::CommitOptions => vec![OverlayRowsItem {
            key: Some("commit:push".to_owned()),
            icon: String::new(),
            label: "Commit and Push".to_owned(),
            detail: String::new(),
            selected: false,
            disabled: false,
            separator: false,
            not_separator: true,
            danger: false,
            checked: false,
            has_children: false,
            children: Vec::new(),
        }],
        Overlay::PullOptions => [
            (
                "Pull (fast-forward only)",
                crate::git::models::PullOperation::FastForwardOnly,
            ),
            ("Pull (rebase)", crate::git::models::PullOperation::Rebase),
        ]
        .into_iter()
        .map(|(label, operation)| {
            let selected = state.settings.default_pull_operation == operation;
            OverlayRowsItem {
                key: Some(format!("pull:{}", label)),
                icon: "O".to_owned(),
                label: label.to_owned(),
                detail: String::new(),
                selected,
                disabled: false,
                separator: false,
                not_separator: true,
                danger: false,
                checked: selected,
                has_children: false,
                children: Vec::new(),
            }
        })
        .collect(),
        Overlay::DiffSelection => ["Discard selection", "Stage selection", "Copy"]
            .into_iter()
            .map(|label| OverlayRowsItem {
                key: Some(format!("diff:{}", label)),
                icon: String::new(),
                label: label.to_owned(),
                detail: String::new(),
                selected: false,
                disabled: false,
                separator: false,
                not_separator: true,
                danger: false,
                checked: false,
                has_children: false,
                children: Vec::new(),
            })
            .collect(),
        Overlay::Tabs => state
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| OverlayRowsItem {
                key: Some(format!("tab:{}", index)),
                icon: "▣".to_owned(),
                label: tab.title.clone(),
                detail: if index == state.active_tab {
                    "ACTIVE"
                } else {
                    ""
                }
                .to_owned(),
                selected: index == state.active_tab,
                disabled: false,
                separator: false,
                not_separator: true,
                danger: false,
                checked: false,
                has_children: false,
                children: Vec::new(),
            })
            .collect(),
        Overlay::Notifications => vec![OverlayRowsItem {
            key: Some("notification:caught_up".to_owned()),
            icon: "v".to_owned(),
            label: "You're all caught up".to_owned(),
            detail: "No new notifications".to_owned(),
            selected: true,
            disabled: false,
            separator: false,
            not_separator: true,
            danger: false,
            checked: false,
            has_children: false,
            children: Vec::new(),
        }],
        _ => Vec::new(),
    }
}

/// Rows for the dedicated branch dropdown, mirroring HEAD's
/// `build_branches()` (src/views/overlays.rs): filtered by the search
/// query, capped at 20, check/remote/branch codicon per row.
fn branch_menu_rows(state: &AppState) -> Vec<BranchRowsItem> {
    if state.overlay != Overlay::Branches {
        return Vec::new();
    }
    let Some(snapshot) = state.snapshot.as_ref() else {
        return Vec::new();
    };
    let query = state.branch_filter.trim().to_lowercase();
    snapshot
        .branches
        .iter()
        .filter(|branch| query.is_empty() || branch.name.to_lowercase().contains(&query))
        .take(20)
        .map(|branch| BranchRowsItem {
            key: Some(format!("branch:{}", branch.name)),
            icon: if branch.current {
                crate::ui::icons::CHECK
            } else if branch.remote {
                crate::ui::icons::REMOTE
            } else {
                crate::ui::icons::BRANCH
            }
            .to_owned(),
            branch_name: branch.name.clone(),
            current: branch.current,
        })
        .collect()
}

fn palette_rows(state: &AppState) -> Vec<PaletteRowsItem> {
    let Some(skin) = palette::skin(&state.overlay) else {
        return Vec::new();
    };
    let Some(model) = &state.palette else {
        return Vec::new();
    };
    let indices = palette::filtered_indices(skin, model.query.text());
    indices
        .iter()
        .enumerate()
        .filter_map(|(filtered_index, source_index)| {
            let (label, keybinding) = palette::command_presentation(skin, *source_index)?;
            Some(PaletteRowsItem {
                key: Some(format!("palette:{}:{}", filtered_index, label)),
                icon: "›".to_owned(),
                label: label.to_owned(),
                hint: keybinding,
                selected: filtered_index == model.cursor,
                disabled: false,
            })
        })
        .collect()
}

fn overlay_header(state: &AppState) -> (Cow<'_, str>, &'static str, f64, f64) {
    match &state.overlay {
        Overlay::Lfs => (
            Cow::Borrowed("LFS Commands"),
            "Git Large File Storage",
            240.0,
            168.0,
        ),
        Overlay::Actions => (
            Cow::Borrowed("Repository Actions"),
            "Common repository operations",
            224.0,
            164.0,
        ),
        Overlay::CommitOptions => (Cow::Borrowed("Commit Options"), "", 220.0, 132.0),
        Overlay::PullOptions => (
            Cow::Borrowed("Pull / Fetch"),
            "Choose the default operation",
            310.0,
            190.0,
        ),
        Overlay::DiffSelection => (
            Cow::Borrowed("Selected Lines"),
            "Apply an action to the selection",
            250.0,
            220.0,
        ),
        Overlay::Tabs => (
            Cow::Borrowed("Open Tabs"),
            "Switch repositories",
            420.0,
            360.0,
        ),
        Overlay::Notifications => (
            Cow::Borrowed("Notifications"),
            "Repository activity",
            340.0,
            210.0,
        ),
        Overlay::CreateBranch => (
            Cow::Borrowed("Create Branch"),
            "Create from the selected commit",
            480.0,
            250.0,
        ),
        Overlay::AddRemote => (
            Cow::Borrowed("Add Remote"),
            "Connect this repository to a remote",
            480.0,
            392.0,
        ),
        Overlay::RenameBranch(_) => (
            Cow::Borrowed("Rename Branch"),
            "Enter a new branch name",
            480.0,
            250.0,
        ),
        Overlay::CreateTag(_) => (
            Cow::Borrowed("Create Tag"),
            "Name and describe the tag",
            480.0,
            310.0,
        ),
        Overlay::EditCommitMessage(_) => (
            Cow::Borrowed("Edit Commit Message"),
            "Amend the selected commit",
            520.0,
            350.0,
        ),
        _ => state
            .context_menu()
            .map_or((Cow::Borrowed("Menu"), "", 300.0, 320.0), |menu| {
                (Cow::Owned(menu.title), "", 300.0, 320.0)
            }),
    }
}

fn overlay_fields(
    state: &AppState,
) -> (
    Option<&str>,
    Option<&str>,
    Option<&str>,
    Option<&str>,
    Option<&str>,
) {
    match &state.overlay {
        Overlay::CreateBranch => (
            Some(state.new_branch.text()),
            None,
            None,
            Some("Create Branch"),
            Some("Cancel"),
        ),
        Overlay::AddRemote => (
            Some(state.add_remote_name.text()),
            Some(state.add_remote_url.text()),
            Some(state.add_remote_push_url.text()),
            Some("Add Remote"),
            Some("Cancel"),
        ),
        Overlay::RenameBranch(_) => (
            Some(state.renamed_branch.text()),
            None,
            None,
            Some("Rename"),
            Some("Cancel"),
        ),
        Overlay::CreateTag(_) => (
            Some(state.tag_name.text()),
            Some(state.tag_message.text()),
            None,
            Some("Create Tag"),
            Some("Cancel"),
        ),
        Overlay::EditCommitMessage(_) => (
            Some(state.edit_summary.text()),
            Some(state.edit_body.text()),
            None,
            Some("Save"),
            Some("Cancel"),
        ),
        _ => (None, None, None, None, None),
    }
}

fn is_danger(label: &str) -> bool {
    ["delete", "discard", "remove", "reset"]
        .iter()
        .any(|word| label.to_lowercase().contains(word))
}

fn format_date(seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|date| date.with_timezone(&Local).format("%m/%d/%Y").to_string())
        .unwrap_or_default()
}

fn format_timestamp(seconds: i64) -> String {
    DateTime::<Utc>::from_timestamp(seconds, 0)
        .map(|date| {
            date.with_timezone(&Local)
                .format("%m/%d/%Y @ %-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    u32::from_le_bytes([red, green, blue, alpha])
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use slab_kernel::dispatch::{E_POINTER_DOWN, E_POINTER_MOVE, E_POINTER_UP, E_TEXT};

    use crate::git::models::{
        BranchInfo, CommitSummary, DiffDocument, DiffRequest, RepoSnapshot, WorkingFile,
        WorkingTree,
    };

    use super::*;

    fn workspace_state() -> AppState {
        let mut state = AppState::new(None, 1_600, 1_000, None);
        let path = PathBuf::from("/tmp/kraken-slab-test");
        state.settings.sidebar_collapsed = false;
        state.tabs[0].title = "kraken-slab-test".to_owned();
        state.tabs[0].path = Some(path.clone());
        state.snapshot = Some(RepoSnapshot {
            path,
            name: "kraken-slab-test".to_owned(),
            head: "main".to_owned(),
            head_id: Some("deadbeef".to_owned()),
            branches: vec![BranchInfo {
                name: "main".to_owned(),
                target: "deadbeef".to_owned(),
                current: true,
                remote: false,
                upstream: None,
            }],
            tags: Vec::new(),
            stashes: Vec::new(),
            worktrees: Vec::new(),
            commits: vec![CommitSummary {
                id: "deadbeef".to_owned(),
                short_id: "deadbee".to_owned(),
                subject: "Initial commit".to_owned(),
                description: String::new(),
                author: "Kraken".to_owned(),
                email: "kraken@example.com".to_owned(),
                authored_seconds: 0,
                parents: Vec::new(),
                is_local: false,
                refs: Vec::new(),
                branch_refs: Vec::new(),
            }],
            working: WorkingTree {
                files: vec![WorkingFile {
                    path: PathBuf::from("src/main.rs"),
                    old_path: None,
                    staged: None,
                    unstaged: Some(ChangeKind::Modified),
                }],
            },
            loaded_limit: 200,
            has_more: false,
            refs_sig: 0,
        });
        state
    }

    fn pointer(etype: u32, x: f64, y: f64, button: u32) -> Event {
        Event {
            etype,
            x,
            y,
            dx: 0.0,
            dy: 0.0,
            button,
            clicks: 1,
            key: String::new(),
            text: String::new(),
            mods: 0,
        }
    }

    fn labeled_rect(
        state: &AppState,
        document: &mut SlabDocument,
        expected: &str,
    ) -> (f64, f64, f64, f64) {
        let frame = document.frame(state);
        frame
            .scene
            .iter()
            .rev()
            .find_map(|node| {
                let label = document
                    .doc
                    .inst
                    .st
                    .scene_strs
                    .get(usize::try_from(node.label).ok()?)?;
                (label == expected && node.w > 0.0 && node.h > 0.0)
                    .then_some((node.x, node.y, node.w, node.h))
            })
            .unwrap_or_else(|| panic!("Slab frame has no visible control labeled {expected:?}"))
    }

    fn labeled_center(state: &AppState, document: &mut SlabDocument, expected: &str) -> (f64, f64) {
        let (x, y, width, height) = labeled_rect(state, document, expected);
        (x + width / 2.0, y + height / 2.0)
    }

    /// Center of the first visible scene node whose authored key ends in `suffix`.
    fn keyed_center(state: &AppState, document: &mut SlabDocument, suffix: &str) -> (f64, f64) {
        let frame = document.frame(state);
        let inst = &document.doc.inst;
        frame
            .scene
            .iter()
            .find_map(|node| {
                let key = slab_kernel::scene::key_of(&inst.doc, &inst.st.lists, node.node);
                (key.ends_with(suffix) && node.w > 0.0 && node.h > 0.0)
                    .then_some((node.x + node.w / 2.0, node.y + node.h / 2.0))
            })
            .unwrap_or_else(|| panic!("Slab frame has no visible node keyed {suffix:?}"))
    }

    fn click_label(
        state: &mut AppState,
        document: &mut SlabDocument,
        label: &str,
    ) -> Vec<SlabHostCommand> {
        let (x, y) = labeled_center(state, document, label);
        let mut commands = document
            .dispatch(state, &pointer(E_POINTER_DOWN, x, y, 0))
            .host_commands;
        commands.extend(
            document
                .dispatch(state, &pointer(E_POINTER_UP, x, y, 0))
                .host_commands,
        );
        commands
    }

    fn context_label(state: &mut AppState, document: &mut SlabDocument, label: &str) {
        let (x, y) = labeled_center(state, document, label);
        document.dispatch(state, &pointer(E_POINTER_DOWN, x, y, 2));
    }

    #[test]
    fn authored_shell_controls_dispatch_application_and_host_actions() {
        let mut state = workspace_state();
        let mut document = SlabDocument::new(generated::Doc::new());

        for (label, overlay) in [
            ("Actions", Overlay::Actions),
            ("Select pull operation", Overlay::PullOptions),
            ("Open tabs", Overlay::Tabs),
            ("Notifications", Overlay::Notifications),
            ("LFS", Overlay::Lfs),
        ] {
            let commands = click_label(&mut state, &mut document, label);
            assert!(commands.is_empty(), "{label}: {commands:?}");
            assert_eq!(state.overlay, overlay);
            state.escape();
        }

        assert!(click_label(&mut state, &mut document, "Add remote").is_empty());
        assert_eq!(state.overlay, Overlay::AddRemote);
        assert!(click_label(&mut state, &mut document, "Fetch URL").is_empty());
        assert_eq!(state.focus, FocusField::AddRemoteUrl);
        let mut text = pointer(E_TEXT, -1.0, -1.0, 0);
        text.text = "https://example.com/repo.git".to_owned();
        document.dispatch(&mut state, &text);
        assert_eq!(state.add_remote_url.text(), "https://example.com/repo.git");
        state.escape();

        context_label(&mut state, &mut document, "main");
        assert_eq!(state.overlay, Overlay::BranchContext("main".to_owned()));
        state.escape();

        let (x, y) = labeled_center(&state, &mut document, "Window drag region");
        assert_eq!(
            document
                .dispatch(&mut state, &pointer(E_POINTER_DOWN, x, y, 0))
                .host_commands,
            vec![SlabHostCommand::DragWindow]
        );
        document.dispatch(&mut state, &pointer(E_POINTER_UP, x, y, 0));

        assert_eq!(
            click_label(&mut state, &mut document, "Close"),
            vec![SlabHostCommand::Close]
        );
    }

    #[test]
    fn authored_diff_controls_dispatch_layout_selection_and_context_actions() {
        let mut state = workspace_state();
        let path = PathBuf::from("src/main.rs");
        state.main_view = MainView::Diff;
        state.diff_split = true;
        state.diff_file_view = false;
        state.selected_file = Some(DiffRequest {
            path: path.clone(),
            scope: DiffScope::Unstaged,
        });
        state.diff = Some(DiffDocument {
            path,
            scope: DiffScope::Unstaged,
            old_label: "a/src/main.rs".to_owned(),
            new_label: "b/src/main.rs".to_owned(),
            rows: vec![
                DiffRow {
                    old_number: None,
                    new_number: None,
                    old_text: "@@ -1,2 +1,3 @@".to_owned(),
                    new_text: "@@ -1,2 +1,3 @@".to_owned(),
                    kind: DiffRowKind::Hunk,
                    old_mark: None,
                    new_mark: None,
                },
                DiffRow {
                    old_number: Some(1),
                    new_number: Some(1),
                    old_text: "fn old() {}".to_owned(),
                    new_text: "fn new() {}".to_owned(),
                    kind: DiffRowKind::Changed,
                    old_mark: Some((3, 6)),
                    new_mark: Some((3, 6)),
                },
                DiffRow {
                    old_number: None,
                    new_number: Some(2),
                    old_text: String::new(),
                    new_text: "fn added() {}".to_owned(),
                    kind: DiffRowKind::Added,
                    old_mark: None,
                    new_mark: None,
                },
            ],
            content: Some("fn new() {}\nfn added() {}\n".to_owned()),
            hunks: vec![0],
            binary: false,
        });
        let mut document = SlabDocument::new(generated::Doc::new());

        click_label(&mut state, &mut document, "Unified diff layout");
        assert!(!state.diff_split);
        click_label(&mut state, &mut document, "Split diff layout");
        assert!(state.diff_split);
        click_label(&mut state, &mut document, "File View");
        assert!(state.diff_file_view);
        click_label(&mut state, &mut document, "Diff View");
        assert!(!state.diff_file_view);

        let (x, y, _, height) = labeled_rect(&state, &mut document, "Diff row");
        document.dispatch(
            &mut state,
            &pointer(E_POINTER_DOWN, x + 8.0, y + height / 2.0, 0),
        );
        document.dispatch(
            &mut state,
            &pointer(E_POINTER_UP, x + 8.0, y + height / 2.0, 0),
        );
        assert_eq!(state.diff_selected_rows.len(), 1);

        context_label(&mut state, &mut document, "Diff row");
        assert_eq!(state.overlay, Overlay::DiffSelection);
    }

    #[test]
    fn authored_preferences_navigation_reaches_every_page() {
        let mut state = workspace_state();
        let mut document = SlabDocument::new(generated::Doc::new());

        click_label(&mut state, &mut document, "Preferences");
        assert!(state.preferences_open);

        for page in [
            "General",
            "Profiles",
            "SSH",
            "External Tools",
            "Commit Signing",
            "Notifications",
            "Experimental",
            "UI Customization",
            "Editor",
            "In-App Terminal",
            "Encoding",
            "Gitflow",
            "LFS",
            "Sparse Checkout",
        ] {
            click_label(&mut state, &mut document, page);
            assert_eq!(state.preference_page, page);
        }

        click_label(&mut state, &mut document, "‹ Exit Preferences");
        assert!(!state.preferences_open);
    }

    #[test]
    fn authored_graph_rows_select_and_open_commit_context() {
        let mut state = workspace_state();
        let mut document = SlabDocument::new(generated::Doc::new());

        click_label(&mut state, &mut document, "Initial commit");
        assert_eq!(state.selected_commit.as_deref(), Some("deadbeef"));

        context_label(&mut state, &mut document, "Initial commit");
        assert_eq!(state.overlay, Overlay::CommitContext("deadbeef".to_owned()));
    }

    #[test]
    fn authored_working_copy_rows_select_toggle_and_open_context() {
        let mut state = workspace_state();
        state.main_view = MainView::Wip;
        let mut document = SlabDocument::new(generated::Doc::new());
        let path = PathBuf::from("src/main.rs");

        click_label(&mut state, &mut document, "Select file");
        assert!(state.selected_working_files.contains(&path));
        assert_eq!(state.main_view, MainView::Wip);

        context_label(&mut state, &mut document, "src/main.rs");
        assert_eq!(
            state.overlay,
            Overlay::FileContext {
                path: path.clone(),
                scope: FileContextScope::Unstaged,
            }
        );
        state.escape();

        click_label(&mut state, &mut document, "src/main.rs");
        assert_eq!(
            state.selected_file,
            Some(DiffRequest {
                path,
                scope: DiffScope::Unstaged,
            })
        );
        assert_eq!(state.main_view, MainView::Diff);
    }

    #[test]
    fn overlay_outside_click_dismisses_without_activating_underneath() {
        let mut state = workspace_state();
        let mut document = SlabDocument::new(generated::Doc::new());

        assert!(click_label(&mut state, &mut document, "Actions").is_empty());
        assert_eq!(state.overlay, Overlay::Actions);

        let selected = state.selected_commits.clone();
        let (x, y) = labeled_center(&state, &mut document, "Initial commit");
        document.dispatch(&mut state, &pointer(E_POINTER_DOWN, x, y, 0));
        document.dispatch(&mut state, &pointer(E_POINTER_UP, x, y, 0));
        assert_eq!(state.overlay, Overlay::None);
        assert_eq!(
            state.selected_commits, selected,
            "outside click only dismisses; it never reaches the row beneath"
        );
    }

    #[test]
    fn divider_drag_resizes_graph_columns_before_release() {
        let mut state = workspace_state();
        let mut document = SlabDocument::new(generated::Doc::new());

        // Growth is clamped by HEAD's resize_preference budget (the test
        // layout already sits at the ref cap), so drag toward shrink: the
        // 100px floor always leaves room below the effective width.
        let (x, y) = keyed_center(&state, &mut document, "#graph-ref-divider");
        let layout = layout::Layout::for_state(&state);
        let before = layout::column_layout(&state, layout.center).refs.width;
        document.dispatch(&mut state, &pointer(E_POINTER_DOWN, x, y, 0));
        document.dispatch(&mut state, &pointer(E_POINTER_MOVE, x - 30.0, y, 0));
        let live_width = state.ref_column_width;
        let after = layout::column_layout(&state, layout.center).refs.width;
        assert!(
            f64::from(before - after) > 20.0,
            "column width follows the pointer before release (before {before}, after {after})"
        );
        document.dispatch(&mut state, &pointer(E_POINTER_UP, x - 30.0, y, 0));
        assert!(
            (state.ref_column_width - live_width).abs() < 1.0,
            "release keeps the live extent"
        );
    }
}
