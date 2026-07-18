use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use winit::event_loop::EventLoopProxy;

use crate::{
    app::UserEvent,
    git::{
        backend::{Backend, GitBackend, LfsOperation},
        models::{
            CommitDetail, CommitInput, DiffDocument, DiffLineSelection, DiffRequest, PullOperation,
            RepoSnapshot, WorkingTree,
        },
    },
};

/// Blocking Git work accepted by the dedicated repository thread.
#[derive(Clone, Debug)]
pub(crate) enum GitJobKind {
    LoadSnapshot {
        limit: usize,
    },
    LoadDetail {
        id: String,
    },
    LoadDiff {
        request: DiffRequest,
    },
    Stage {
        paths: Vec<PathBuf>,
        limit: usize,
    },
    Unstage {
        paths: Vec<PathBuf>,
        limit: usize,
    },
    StageLines {
        path: PathBuf,
        lines: Vec<DiffLineSelection>,
        limit: usize,
    },
    UnstageLines {
        path: PathBuf,
        lines: Vec<DiffLineSelection>,
        limit: usize,
    },
    DiscardLines {
        path: PathBuf,
        lines: Vec<DiffLineSelection>,
        limit: usize,
    },
    DiscardFile {
        path: PathBuf,
        limit: usize,
    },
    StageAll {
        limit: usize,
    },
    UnstageAll {
        limit: usize,
    },
    Commit {
        input: CommitInput,
        limit: usize,
    },
    Checkout {
        branch: String,
        limit: usize,
    },
    CreateBranch {
        branch: String,
        limit: usize,
        target: Option<String>,
    },
    AddRemote {
        name: String,
        url: String,
        push_url: Option<String>,
        limit: usize,
    },
    Fetch {
        prune: bool,
        limit: usize,
    },
    Pull {
        operation: PullOperation,
        limit: usize,
    },
    Push {
        limit: usize,
    },
    Reword {
        summary: String,
        body: String,
        limit: usize,
    },
    SavePatch {
        id: String,
        destination: PathBuf,
        limit: usize,
    },
    Stash {
        limit: usize,
    },
    StashFile {
        path: PathBuf,
        limit: usize,
    },
    PopStash {
        limit: usize,
    },
    RebaseOnto {
        source: String,
        target: String,
        limit: usize,
    },
    FastForwardTo {
        source: String,
        target: String,
        limit: usize,
    },
    FastForward {
        branch: String,
        limit: usize,
    },
    Merge {
        branch: String,
        limit: usize,
    },
    Rebase {
        branch: String,
        limit: usize,
    },
    CreateTag {
        name: String,
        target: String,
        message: Option<String>,
        limit: usize,
    },
    CherryPick {
        id: String,
        limit: usize,
    },
    Revert {
        id: String,
        limit: usize,
    },
    Reset {
        target: String,
        mode: String,
        limit: usize,
    },
    DeleteBranch {
        branch: String,
        limit: usize,
    },
    RestoreBranch {
        branch: String,
        target: String,
        limit: usize,
    },
    UndoBranchCreate {
        branch: String,
        target: String,
        previous: String,
        limit: usize,
    },
    RenameBranch {
        branch: String,
        new_name: String,
        limit: usize,
    },
    ApplyStash {
        index: usize,
        limit: usize,
    },
    DropStash {
        index: usize,
        limit: usize,
    },
    DeleteTag {
        tag: String,
        limit: usize,
    },
    Lfs {
        operation: LfsOperation,
        limit: usize,
    },
    InitGitflow {
        limit: usize,
    },
    SparseCheckout {
        paths: Option<String>,
        limit: usize,
    },
    TrackLfsPattern {
        pattern: String,
        limit: usize,
    },
    IgnorePattern {
        pattern: String,
        limit: usize,
    },
    Clone {
        url: String,
        destination: PathBuf,
    },
}

impl GitJobKind {
    /// Returns the user-visible operation name for successful mutations.
    pub(crate) fn success_message(&self) -> Option<String> {
        match self {
            Self::LoadSnapshot { .. }
            | Self::LoadDetail { .. }
            | Self::LoadDiff { .. }
            | Self::Commit { .. } => None,
            Self::Stage { .. } => Some("Staged selected files".to_owned()),
            Self::Unstage { .. } => Some("Unstaged selected files".to_owned()),
            Self::StageLines { .. } => Some("Staged selected lines".to_owned()),
            Self::UnstageLines { .. } => Some("Unstaged selected lines".to_owned()),
            Self::DiscardLines { .. } => Some("Discarded selected lines".to_owned()),
            Self::DiscardFile { path, .. } => {
                Some(format!("Discarded changes in {}", path.display()))
            }
            Self::StageAll { .. } => Some("Staged all changes".to_owned()),
            Self::UnstageAll { .. } => Some("Unstaged all changes".to_owned()),
            Self::Checkout { branch, .. } => Some(format!("Checked out {branch}")),
            Self::CreateBranch { branch, .. } => Some(format!("Created branch {branch}")),
            Self::AddRemote { name, .. } => Some(format!("Added remote {name}")),
            Self::Fetch { .. } => Some("Fetched all remotes".to_owned()),
            Self::Pull { operation, .. } => Some(
                match operation {
                    PullOperation::FetchAll => "Fetched all remotes",
                    PullOperation::FastForward => "Pulled current upstream",
                    PullOperation::FastForwardOnly => "Pulled current upstream (fast-forward only)",
                    PullOperation::Rebase => "Pulled current upstream (rebase)",
                }
                .to_owned(),
            ),
            Self::Push { .. } => Some("Pushed current branch".to_owned()),
            Self::Stash { .. } => Some("Stashed working changes".to_owned()),
            Self::StashFile { path, .. } => Some(format!("Stashed {}", path.display())),
            Self::PopStash { .. } => Some("Restored newest stash".to_owned()),
            Self::Reword { .. } => Some("Amended commit message".to_owned()),
            Self::SavePatch { destination, .. } => {
                Some(format!("Saved patch to {}", destination.display()))
            }
            Self::FastForward { branch, .. } => Some(format!("Fast-forwarded to {branch}")),
            Self::RebaseOnto { source, target, .. } => {
                Some(format!("Rebased {source} onto {target}"))
            }
            Self::FastForwardTo { source, target, .. } => {
                Some(format!("Fast-forwarded {target} to {source}"))
            }
            Self::Merge { branch, .. } => Some(format!("Merged {branch}")),
            Self::Rebase { branch, .. } => Some(format!("Rebased onto {branch}")),
            Self::CreateTag { name, .. } => Some(format!("Created tag {name}")),
            Self::CherryPick { .. } => Some("Cherry-picked commit".to_owned()),
            Self::Revert { .. } => Some("Reverted commit".to_owned()),
            Self::Reset { mode, .. } => Some(format!("Reset ({mode})")),
            Self::DeleteBranch { branch, .. } => Some(format!("Deleted branch {branch}")),
            Self::RestoreBranch { branch, .. } => Some(format!("Restored branch {branch}")),
            Self::UndoBranchCreate { branch, .. } => Some(format!("Removed branch {branch}")),
            Self::RenameBranch { new_name, .. } => Some(format!("Renamed branch to {new_name}")),
            Self::ApplyStash { .. } => Some("Applied stash".to_owned()),
            Self::DropStash { .. } => Some("Dropped stash".to_owned()),
            Self::DeleteTag { tag, .. } => Some(format!("Deleted tag {tag}")),
            Self::Lfs { operation, .. } => Some(format!("Completed Git LFS {operation:?}")),
            Self::InitGitflow { .. } => Some("Initialized Gitflow".to_owned()),
            Self::SparseCheckout { paths, .. } => Some(if paths.is_some() {
                "Applied sparse checkout".to_owned()
            } else {
                "Disabled sparse checkout".to_owned()
            }),
            Self::TrackLfsPattern { pattern, .. } => {
                Some(format!("Tracking {pattern} with Git LFS"))
            }
            Self::IgnorePattern { pattern, .. } => Some(format!("Ignoring {pattern}")),
            Self::Clone { .. } => Some("Repository cloned".to_owned()),
        }
    }
}

/// A versioned worker request; stale results are ignored by the application.
#[derive(Clone, Debug)]
pub(crate) struct GitJob {
    pub(crate) generation: u64,
    pub(crate) path: PathBuf,
    pub(crate) kind: GitJobKind,
    pub(crate) settings: crate::settings::Settings,
}

/// Successful immutable payload returned to the UI thread.
#[derive(Debug)]
pub(crate) enum GitPayload {
    Snapshot(RepoSnapshot),
    Detail(CommitDetail),
    Diff(DiffDocument),
    Mutated {
        snapshot: RepoSnapshot,
        commit_id: Option<String>,
        message: Option<String>,
    },
    Cloned(PathBuf),
}

/// One worker completion with a displayable error on failure.
#[derive(Debug)]
pub(crate) struct GitEvent {
    pub(crate) generation: u64,
    /// The completed request, absent for filesystem-watcher refreshes.
    pub(crate) kind: Option<GitJobKind>,
    pub(crate) result: Result<GitPayload, String>,
}

/// Owns channels for a single long-lived blocking Git worker.
pub(crate) struct GitRunner {
    jobs: Sender<GitJob>,
    events: Receiver<GitEvent>,
}

impl GitRunner {
    /// Starts the worker thread and returns its nonblocking UI endpoint.
    pub(crate) fn new(event_loop_proxy: Option<EventLoopProxy<UserEvent>>) -> Self {
        let (job_sender, job_receiver) = unbounded::<GitJob>();
        let (event_sender, event_receiver) = unbounded::<GitEvent>();
        thread::Builder::new()
            .name("kraken-git".to_owned())
            .spawn(move || worker_loop(&job_receiver, &event_sender, &event_loop_proxy))
            .expect("spawn Git worker thread");
        Self {
            jobs: job_sender,
            events: event_receiver,
        }
    }

    /// Queues work without blocking the UI thread.
    pub(crate) fn submit(&self, job: GitJob) {
        let _ = self.jobs.send(job);
    }

    /// Drains all completions currently available to the UI loop.
    pub(crate) fn drain(&self) -> impl Iterator<Item = GitEvent> + '_ {
        self.events.try_iter()
    }
}

fn worker_loop(
    jobs: &Receiver<GitJob>,
    events: &Sender<GitEvent>,
    event_loop_proxy: &Option<EventLoopProxy<UserEvent>>,
) {
    let (filesystem_sender, filesystem_events) = unbounded::<notify::Result<Event>>();
    let mut _watcher: Option<RecommendedWatcher> = None;
    let mut watched = None;
    let mut pending_refresh = None;
    loop {
        while let Ok(event) = filesystem_events.try_recv() {
            if event.as_ref().is_ok_and(event_is_relevant) {
                pending_refresh = Some(Instant::now());
            }
        }
        if pending_refresh.is_some_and(|started| started.elapsed() >= Duration::from_millis(400)) {
            pending_refresh = None;
            if !refresh_watched(&mut watched, events, event_loop_proxy) {
                break;
            }
        }
        match jobs.recv_timeout(Duration::from_millis(100)) {
            Ok(job) => {
                let generation = job.generation;
                let path = job.path.clone();
                let limit = snapshot_limit(&job.kind);
                let kind = job.kind.clone();
                let result = execute(job).map_err(|error| format!("{error:#}"));
                if let (Some(limit), Ok(payload)) = (limit, &result)
                    && let Some(working) = payload_working(payload)
                {
                    let repository = WatchedRepository {
                        generation,
                        path,
                        limit,
                        working: working.clone(),
                    };
                    _watcher = arm_watcher(&repository.path, &filesystem_sender).ok();
                    watched = Some(repository);
                }
                if events
                    .send(GitEvent {
                        generation,
                        kind: Some(kind),
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                wake_event_loop(event_loop_proxy, UserEvent::Git);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn arm_watcher(
    path: &std::path::Path,
    sender: &Sender<notify::Result<Event>>,
) -> notify::Result<RecommendedWatcher> {
    let event_sender = sender.clone();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = event_sender.send(event);
    })?;
    watcher.watch(path, RecursiveMode::Recursive)?;
    let repository = git2::Repository::discover(path)
        .map_err(|error| notify::Error::generic(&error.to_string()))?;
    watcher.watch(repository.path(), RecursiveMode::Recursive)?;
    Ok(watcher)
}

fn event_is_relevant(event: &Event) -> bool {
    event.paths.iter().any(|path| {
        !path
            .components()
            .any(|component| component.as_os_str() == "objects")
            && path
                .file_name()
                .is_none_or(|name| !name.to_string_lossy().ends_with(".lock"))
    })
}
fn refresh_watched(
    watched: &mut Option<WatchedRepository>,
    events: &Sender<GitEvent>,
    event_loop_proxy: &Option<EventLoopProxy<UserEvent>>,
) -> bool {
    let Some(repository) = watched.as_mut() else {
        return true;
    };
    let Ok(snapshot) = GitBackend::discover(&repository.path)
        .and_then(|backend| backend.snapshot(repository.limit))
    else {
        return true;
    };
    repository.working.clone_from(&snapshot.working);
    if events
        .send(GitEvent {
            generation: repository.generation,
            kind: None,
            result: Ok(GitPayload::Snapshot(snapshot)),
        })
        .is_err()
    {
        return false;
    }
    wake_event_loop(event_loop_proxy, UserEvent::Filesystem);
    true
}

fn wake_event_loop(event_loop_proxy: &Option<EventLoopProxy<UserEvent>>, event: UserEvent) {
    if let Some(proxy) = event_loop_proxy {
        let _ = proxy.send_event(event);
    }
}

struct WatchedRepository {
    generation: u64,
    path: PathBuf,
    limit: usize,
    working: WorkingTree,
}

fn snapshot_limit(kind: &GitJobKind) -> Option<usize> {
    match kind {
        GitJobKind::LoadSnapshot { limit }
        | GitJobKind::Stage { limit, .. }
        | GitJobKind::Unstage { limit, .. }
        | GitJobKind::RestoreBranch { limit, .. }
        | GitJobKind::UndoBranchCreate { limit, .. }
        | GitJobKind::StageLines { limit, .. }
        | GitJobKind::UnstageLines { limit, .. }
        | GitJobKind::DiscardLines { limit, .. }
        | GitJobKind::DiscardFile { limit, .. }
        | GitJobKind::StageAll { limit }
        | GitJobKind::UnstageAll { limit }
        | GitJobKind::Commit { limit, .. }
        | GitJobKind::Checkout { limit, .. }
        | GitJobKind::CreateTag { limit, .. }
        | GitJobKind::CherryPick { limit, .. }
        | GitJobKind::Revert { limit, .. }
        | GitJobKind::Reset { limit, .. }
        | GitJobKind::DeleteBranch { limit, .. }
        | GitJobKind::RenameBranch { limit, .. }
        | GitJobKind::ApplyStash { limit, .. }
        | GitJobKind::DropStash { limit, .. }
        | GitJobKind::Fetch { limit, .. }
        | GitJobKind::DeleteTag { limit, .. }
        | GitJobKind::CreateBranch { limit, .. }
        | GitJobKind::Pull { limit, .. }
        | GitJobKind::SavePatch { limit, .. }
        | GitJobKind::Reword { limit, .. }
        | GitJobKind::Push { limit }
        | GitJobKind::Stash { limit }
        | GitJobKind::StashFile { limit, .. }
        | GitJobKind::PopStash { limit }
        | GitJobKind::FastForward { limit, .. }
        | GitJobKind::RebaseOnto { limit, .. }
        | GitJobKind::FastForwardTo { limit, .. }
        | GitJobKind::Merge { limit, .. }
        | GitJobKind::Rebase { limit, .. }
        | GitJobKind::Lfs { limit, .. }
        | GitJobKind::InitGitflow { limit }
        | GitJobKind::SparseCheckout { limit, .. }
        | GitJobKind::TrackLfsPattern { limit, .. }
        | GitJobKind::IgnorePattern { limit, .. }
        | GitJobKind::AddRemote { limit, .. } => Some(*limit),
        GitJobKind::LoadDetail { .. } | GitJobKind::LoadDiff { .. } => None,
        GitJobKind::Clone { .. } => None,
    }
}

fn payload_working(payload: &GitPayload) -> Option<&WorkingTree> {
    match payload {
        GitPayload::Snapshot(snapshot) | GitPayload::Mutated { snapshot, .. } => {
            Some(&snapshot.working)
        }
        GitPayload::Detail(_) | GitPayload::Diff(_) | GitPayload::Cloned(_) => None,
    }
}

fn execute(job: GitJob) -> anyhow::Result<GitPayload> {
    if let GitJobKind::Clone { url, destination } = &job.kind {
        let name = url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("repository")
            .trim_end_matches(".git");
        let target = destination.join(name);
        git2::Repository::clone(url, &target)?;
        return Ok(GitPayload::Cloned(target));
    }
    let message = job.kind.success_message();
    let backend = GitBackend::discover_with_settings(job.path, job.settings)?;
    match job.kind {
        GitJobKind::LoadSnapshot { limit } => backend.snapshot(limit).map(GitPayload::Snapshot),
        GitJobKind::LoadDetail { id } => backend.commit_detail(&id).map(GitPayload::Detail),
        GitJobKind::LoadDiff { request } => backend.diff(&request).map(GitPayload::Diff),
        GitJobKind::StageLines { path, lines, limit } => {
            backend.stage_lines(&path, &lines)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::UnstageLines { path, lines, limit } => {
            backend.unstage_lines(&path, &lines)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::DiscardLines { path, lines, limit } => {
            backend.discard_lines(&path, &lines)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::DiscardFile { path, limit } => {
            backend.discard_file(&path)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Stage { paths, limit } => {
            backend.stage(&paths)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Unstage { paths, limit } => {
            backend.unstage(&paths)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::StageAll { limit } => {
            backend.stage_all()?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::UnstageAll { limit } => {
            backend.unstage_all()?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Commit { input, limit } => {
            let id = backend.commit(&input)?;
            refreshed(&backend, limit, Some(id), message)
        }
        GitJobKind::Reword {
            summary,
            body,
            limit,
        } => {
            let id = backend.reword(&summary, &body)?;
            refreshed(&backend, limit, Some(id), message)
        }
        GitJobKind::SavePatch {
            id,
            destination,
            limit,
        } => {
            backend.save_patch(&id, &destination)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Checkout { branch, limit } => {
            backend.checkout(&branch)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::CreateBranch {
            branch,
            target,
            limit,
        } => {
            backend.create_branch(&branch, target.as_deref())?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Fetch { prune, limit } => {
            backend.fetch(prune)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::AddRemote {
            name,
            url,
            push_url,
            limit,
        } => {
            backend.add_remote(&name, &url, push_url.as_deref())?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Pull { operation, limit } => {
            backend.pull(operation)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Push { limit } => {
            backend.push()?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Stash { limit } => {
            backend.stash()?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::StashFile { path, limit } => {
            backend.stash_file(&path)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::PopStash { limit } => {
            backend.pop_stash()?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::FastForward { branch, limit } => {
            backend.fast_forward(&branch)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::RebaseOnto {
            source,
            target,
            limit,
        } => {
            backend.rebase_onto(&source, &target)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::FastForwardTo {
            source,
            target,
            limit,
        } => {
            backend.fast_forward_to(&source, &target)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Merge { branch, limit } => {
            backend.merge(&branch)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Rebase { branch, limit } => {
            backend.rebase(&branch)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::CreateTag {
            name,
            target,
            message: tag_message,
            limit,
        } => {
            backend.create_tag(&name, &target, tag_message.as_deref())?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::CherryPick { id, limit } => {
            backend.cherry_pick(&id)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Revert { id, limit } => {
            backend.revert(&id)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Reset {
            target,
            mode,
            limit,
        } => {
            backend.reset(&target, &mode)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::DeleteBranch { branch, limit } => {
            backend.delete_branch(&branch)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::RestoreBranch {
            branch,
            target,
            limit,
        } => {
            backend.restore_branch(&branch, &target)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::UndoBranchCreate {
            branch,
            target,
            previous,
            limit,
        } => {
            backend.checkout(&previous)?;
            backend.delete_branch_at(&branch, &target)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::RenameBranch {
            branch,
            new_name,
            limit,
        } => {
            backend.rename_branch(&branch, &new_name)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::ApplyStash { index, limit } => {
            backend.apply_stash(index)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::DropStash { index, limit } => {
            backend.drop_stash(index)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::DeleteTag { tag, limit } => {
            backend.delete_tag(&tag)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::InitGitflow { limit } => {
            backend.init_gitflow()?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::SparseCheckout { paths, limit } => {
            backend.sparse_checkout(paths.as_deref())?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::TrackLfsPattern { pattern, limit } => {
            backend.track_lfs_pattern(&pattern)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::IgnorePattern { pattern, limit } => {
            backend.append_ignore(&pattern)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Lfs { operation, limit } => {
            backend.lfs(operation)?;
            refreshed(&backend, limit, None, message)
        }
        GitJobKind::Clone { .. } => unreachable!("clone returns before repository discovery"),
    }
}

fn refreshed(
    backend: &GitBackend,
    limit: usize,
    commit_id: Option<String>,
    message: Option<String>,
) -> anyhow::Result<GitPayload> {
    Ok(GitPayload::Mutated {
        snapshot: backend.snapshot(limit)?,
        commit_id,
        message,
    })
}
