//! Per-repository read acceleration for the Git worker.
//!
//! A [`RepoStore`] lives on the worker thread for the lifetime of an open
//! repository and keeps everything worth remembering between jobs: immutable
//! commit metadata keyed by object id, plus the last assembled history slice.
//! Reads go through gitoxide. The reference pass resolves peeled targets from
//! packed-refs without touching the object database, and the commit walk
//! streams `--date-order` output through commit-graph generation numbers
//! instead of pre-sorting the entire history the way libgit2 does. A repeated
//! snapshot whose references did not move (staging churn, watcher refreshes)
//! reuses the previous walk outright and only rescans worktree status.

use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::Entry},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};
use gix::{
    ObjectId,
    bstr::{BStr, BString, ByteSlice},
    refs::TargetRef,
    traverse::commit::topo,
};

use crate::git::cache::SnapshotCache;
use crate::git::models::{
    BranchInfo, ChangeKind, CommitBranchRef, CommitSummary, RefLabel, RepoSnapshot, WorkingFile,
    WorkingTree,
};

/// Longest commit description carried into graph rows.
const DESCRIPTION_LIMIT: usize = 160;

/// Commits kept in the cross-session cache: enough to fill the first paint,
/// bounded so a deeply paged session cannot make the next launch slow.
const PERSIST_COMMITS: usize = 500;

/// Worker-thread cache for one repository; see the module docs.
pub(crate) struct RepoStore {
    path: PathBuf,
    /// Immutable commit metadata keyed by object id. Content-addressed, so
    /// entries never invalidate; unreachable ids merely linger (bounded by
    /// the deepest history ever walked).
    commits: HashMap<ObjectId, CachedCommit>,
    /// The last walk, reused verbatim while no reference moves.
    memo: Option<GraphMemo>,
    /// Change marker (refs signature + working tree) of the snapshot last
    /// written to disk, so unchanged snapshots skip the write entirely.
    persisted: Option<(u64, WorkingTree)>,
    maintenance_spawned: bool,
}

/// Parsed commit fields that never change for a given object id.
struct CachedCommit {
    subject: String,
    description: String,
    author: String,
    email: String,
    seconds: i64,
    parents: Vec<String>,
}

/// One assembled history slice keyed by the reference signature it saw.
///
/// Worktree labels baked into `commits` can go stale while references are
/// unmoved; the filesystem watcher already accepts that staleness because
/// linked-worktree HEADs never participate in the signature.
struct GraphMemo {
    refs_sig: u64,
    limit: usize,
    commits: Vec<CommitSummary>,
    has_more: bool,
}

/// Everything one pass over the references yields for a snapshot.
pub(crate) struct RefsSnapshot {
    /// Order-independent digest of every reference plus HEAD.
    pub(crate) sig: u64,
    /// Checked-out branch shorthand, or "HEAD" when detached or unborn.
    pub(crate) head: String,
    /// Commit id HEAD resolves to, absent on unborn branches.
    pub(crate) head_id: Option<String>,
    /// Local and remote branches, sorted for the sidebar.
    pub(crate) branches: Vec<BranchInfo>,
    /// Tags with the commit each one labels after peeling.
    pub(crate) tags: Vec<TagTip>,
    /// Unique commit ids seeding the history walk.
    tips: Vec<ObjectId>,
    /// Tips reachable from remote branches, seeding local-only detection.
    remote_tips: Vec<ObjectId>,
}

/// A tag name and the hex id of the commit it (transitively) points at.
pub(crate) struct TagTip {
    pub(crate) name: String,
    pub(crate) target: String,
}

impl RepoStore {
    /// Creates an empty store for the repository at `path` (worktree root, or
    /// the gitdir for bare repositories).
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            commits: HashMap::new(),
            memo: None,
            persisted: None,
            maintenance_spawned: false,
        }
    }

    /// The bounded value to cache for `snapshot`, or `None` when the
    /// repository state has not moved since the last write.
    ///
    /// Only the first [`PERSIST_COMMITS`] rows are kept. Paging history
    /// raises the live snapshot depth to as much as 100k commits, and a warm
    /// open would then have to decode and lay out every one of them before
    /// painting — the opposite of the point. Deeper history is re-paged from
    /// the repository once the real snapshot lands.
    pub(crate) fn prepare_persist(&self, snapshot: &RepoSnapshot) -> Option<RepoSnapshot> {
        let unchanged = self.persisted.as_ref().is_some_and(|(refs_sig, working)| {
            *refs_sig == snapshot.refs_sig && *working == snapshot.working
        });
        if unchanged {
            return None;
        }
        // Built field by field so the bounded copy never clones the full
        // commit vector on the way to being cut down.
        Some(RepoSnapshot {
            path: snapshot.path.clone(),
            name: snapshot.name.clone(),
            head: snapshot.head.clone(),
            head_id: snapshot.head_id.clone(),
            branches: snapshot.branches.clone(),
            tags: snapshot.tags.clone(),
            stashes: snapshot.stashes.clone(),
            worktrees: snapshot.worktrees.clone(),
            commits: snapshot
                .commits
                .get(..PERSIST_COMMITS)
                .unwrap_or(&snapshot.commits)
                .to_vec(),
            working: snapshot.working.clone(),
            loaded_limit: snapshot.loaded_limit.min(PERSIST_COMMITS),
            has_more: snapshot.has_more || snapshot.commits.len() > PERSIST_COMMITS,
            refs_sig: snapshot.refs_sig,
            remote_url: snapshot.remote_url.clone(),
        })
    }

    /// Writes a value from [`RepoStore::prepare_persist`] and records it, so
    /// unchanged repository state skips later writes.
    ///
    /// Callers run this *after* handing the fresh snapshot to the UI:
    /// serialization and the LMDB commit must never sit in front of a paint.
    pub(crate) fn commit_persist(&mut self, cache: &SnapshotCache, snapshot: &RepoSnapshot) {
        if cache.store(&self.path, snapshot).is_ok() {
            self.persisted = Some((snapshot.refs_sig, snapshot.working.clone()));
        }
    }

    /// Prepares and writes in one step, for callers not on a paint path.
    pub(crate) fn persist_snapshot(&mut self, cache: &SnapshotCache, snapshot: &RepoSnapshot) {
        if let Some(bounded) = self.prepare_persist(snapshot) {
            self.commit_persist(cache, &bounded);
        }
    }

    /// Opens a fresh gitoxide handle. Opening is cheap and guarantees each
    /// snapshot sees the current packed-refs and object-store state.
    pub(crate) fn open(&self) -> Result<gix::Repository> {
        open_repository(&self.path)
    }

    /// Runs one background `git commit-graph write` per store lifetime so
    /// subsequent topological walks stream through generation numbers.
    /// Failures are ignored: the walk works without a graph, just slower.
    pub(crate) fn spawn_commit_graph_maintenance(&mut self, program: &str) {
        if self.maintenance_spawned || program.is_empty() {
            return;
        }
        self.maintenance_spawned = true;
        let program = program.to_owned();
        let path = self.path.clone();
        let _ = std::thread::Builder::new()
            .name("kraken-commit-graph".to_owned())
            .spawn(move || {
                let _ = Command::new(program)
                    .args([
                        "commit-graph",
                        "write",
                        "--reachable",
                        "--split",
                        "--no-progress",
                    ])
                    .current_dir(&path)
                    .output();
            });
    }

    /// Collects branches, tags, HEAD, walk tips, and the reference signature
    /// in a single pass over the reference database.
    ///
    /// Ref shorthands flow back into checkout/delete/rename jobs, so names
    /// with invalid UTF-8 are skipped outright (matching libgit2's behavior)
    /// instead of lossily collapsed into strings that no longer name the ref.
    pub(crate) fn read_refs(&self, repo: &gix::Repository) -> Result<RefsSnapshot> {
        let (head, head_ref, head_id) = head_state(repo)?;
        let mut sig = 0u64;
        let mut branches = Vec::new();
        let mut locals = Vec::new();
        let mut remote_names = HashSet::<BString>::new();
        let mut tags = Vec::new();
        let mut tips = Vec::new();
        let mut seen = HashSet::new();
        let mut remote_tips = Vec::new();
        let platform = repo.references().context("enumerate references")?;
        for mut reference in platform.all().context("iterate references")?.flatten() {
            let name = reference.name().as_bstr().to_owned();
            if !name.starts_with_str("refs/") {
                continue;
            }
            sig ^= reference_digest(name.as_bstr(), reference.target());
            if let Some(short) = name.strip_prefix(b"refs/heads/") {
                let Some(short) = utf8_name(short) else {
                    continue;
                };
                let target = self.commit_target(repo, &mut reference);
                if let Some(id) = target
                    && seen.insert(id)
                {
                    tips.push(id);
                }
                locals.push((branches.len(), reference.name().to_owned()));
                branches.push(BranchInfo {
                    name: short,
                    target: target.map(|id| id.to_string()).unwrap_or_default(),
                    current: head_ref.as_ref().is_some_and(|head| *head == name),
                    remote: false,
                    upstream: None,
                });
            } else if let Some(short) = name.strip_prefix(b"refs/remotes/") {
                let Some(short) = utf8_name(short) else {
                    continue;
                };
                remote_names.insert(name.clone());
                let target = self.commit_target(repo, &mut reference);
                if let Some(id) = target {
                    if seen.insert(id) {
                        tips.push(id);
                    }
                    remote_tips.push(id);
                }
                branches.push(BranchInfo {
                    name: short,
                    target: target.map(|id| id.to_string()).unwrap_or_default(),
                    current: false,
                    remote: true,
                    upstream: None,
                });
            } else if let Some(short) = name.strip_prefix(b"refs/tags/") {
                let Some(short) = utf8_name(short) else {
                    continue;
                };
                let Some(id) = self.commit_target(repo, &mut reference) else {
                    continue;
                };
                if seen.insert(id) {
                    tips.push(id);
                }
                tags.push(TagTip {
                    name: short,
                    target: id.to_string(),
                });
            }
        }
        for (index, full) in locals {
            let Some(upstream) = repo
                .branch_remote_tracking_ref_name(full.as_ref(), gix::remote::Direction::Fetch)
                .and_then(Result::ok)
            else {
                continue;
            };
            if remote_names.contains(upstream.as_bstr()) {
                let short = upstream
                    .as_bstr()
                    .strip_prefix(b"refs/remotes/")
                    .unwrap_or(upstream.as_bstr());
                if let Some(short) = utf8_name(short) {
                    branches[index].upstream = Some(short);
                }
            }
        }
        sort_branches(&mut branches);
        let head_id = head_id.map(|id| id.to_string());
        sig ^= head_digest(&head, head_id.as_deref());
        Ok(RefsSnapshot {
            sig,
            head,
            head_id,
            branches,
            tags,
            tips,
            remote_tips,
        })
    }

    /// Walks history from the snapshot's tips in `--date-order`, reusing the
    /// previous walk when no reference moved and `limit` is already covered.
    pub(crate) fn history(
        &mut self,
        repo: &gix::Repository,
        refs: &RefsSnapshot,
        labels: &HashMap<String, Vec<RefLabel>>,
        branch_refs: &HashMap<String, Vec<CommitBranchRef>>,
        limit: usize,
    ) -> Result<(Vec<CommitSummary>, bool)> {
        if let Some(memo) = &self.memo
            && memo.refs_sig == refs.sig
            && memo.limit >= limit
        {
            let has_more = memo.has_more || memo.commits.len() > limit;
            return Ok((memo.commits.iter().take(limit).cloned().collect(), has_more));
        }
        if refs.tips.is_empty() {
            return Ok((Vec::new(), false));
        }
        let commit_graph = repo.commit_graph_if_enabled().ok().flatten();
        let walk = topo::Builder::from_iters(
            &repo.objects,
            refs.tips.iter().copied(),
            None::<Vec<ObjectId>>,
        )
        .sorting(topo::Sorting::DateOrder)
        .with_commit_graph(commit_graph)
        .build()
        .context("start commit walk")?;
        // Remote reachability propagates child-to-parent along the walk
        // (children always precede parents in date order), replacing per-commit
        // merge-base queries that would be O(remotes × history).
        let mut remote_reachable: HashSet<ObjectId> = refs.remote_tips.iter().copied().collect();
        let mut commits = Vec::with_capacity(limit.min(10_000));
        let mut has_more = false;
        for info in walk {
            let info = info.context("walk commit")?;
            if commits.len() == limit {
                has_more = true;
                break;
            }
            let is_local = !remote_reachable.contains(&info.id);
            if !is_local {
                for parent in &info.parent_ids {
                    remote_reachable.insert(*parent);
                }
            }
            let cached = match self.commits.entry(info.id) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => entry.insert(parse_commit(repo, &info)?),
            };
            let id = info.id.to_string();
            commits.push(CommitSummary {
                short_id: id.chars().take(7).collect(),
                subject: cached.subject.clone(),
                description: cached.description.clone(),
                author: cached.author.clone(),
                email: cached.email.clone(),
                authored_seconds: cached.seconds,
                parents: cached.parents.clone(),
                is_local,
                refs: labels.get(&id).cloned().unwrap_or_default(),
                branch_refs: branch_refs.get(&id).cloned().unwrap_or_default(),
                id,
            });
        }
        self.memo = Some(GraphMemo {
            refs_sig: refs.sig,
            limit,
            commits: commits.clone(),
            has_more,
        });
        Ok((commits, has_more))
    }

    /// Resolves a reference to the commit it labels: packed peeled entries
    /// first, then the direct target, peeling symbolic references and tag
    /// objects only when required. Non-commit targets yield `None`.
    fn commit_target(
        &self,
        repo: &gix::Repository,
        reference: &mut gix::Reference<'_>,
    ) -> Option<ObjectId> {
        if let Some(peeled) = reference.inner.peeled {
            return self.verified_commit(repo, peeled);
        }
        let id = match reference.target() {
            TargetRef::Object(id) => id.to_owned(),
            TargetRef::Symbolic(_) => reference.peel_to_id().ok()?.detach(),
        };
        self.verified_commit(repo, id)
    }

    /// Confirms `id` names a commit, following tag chains; the header lookup
    /// is skipped for ids the commit cache already proved.
    fn verified_commit(&self, repo: &gix::Repository, id: ObjectId) -> Option<ObjectId> {
        if self.commits.contains_key(&id) {
            return Some(id);
        }
        match repo.find_header(id).ok()?.kind() {
            gix::object::Kind::Commit => Some(id),
            gix::object::Kind::Tag => {
                let object = repo.find_object(id).ok()?.peel_tags_to_end().ok()?;
                (object.kind == gix::object::Kind::Commit).then_some(object.id)
            }
            _ => None,
        }
    }
}

/// Opens a gitoxide handle with a modest object cache for peeling and parses.
pub(crate) fn open_repository(path: &Path) -> Result<gix::Repository> {
    let mut repo =
        gix::open(path).with_context(|| format!("open repository {}", path.display()))?;
    repo.object_cache_size_if_unset(16 * 1024 * 1024);
    Ok(repo)
}

/// Order-independent digest of every reference plus HEAD.
///
/// Equal signatures mean no branch, tag, or HEAD motion happened, so the
/// filesystem watcher can downgrade a refresh to a status-only scan.
pub(crate) fn refs_signature(path: &Path) -> Result<u64> {
    let repo = open_repository(path)?;
    let mut sig = 0u64;
    let platform = repo.references().context("enumerate references")?;
    for reference in platform.all().context("iterate references")?.flatten() {
        let name = reference.name().as_bstr();
        if !name.starts_with_str("refs/") {
            continue;
        }
        sig ^= reference_digest(name, reference.target());
    }
    let (head, _, head_id) = head_state(&repo)?;
    let head_id = head_id.map(|id| id.to_string());
    Ok(sig ^ head_digest(&head, head_id.as_deref()))
}

/// Combined index and worktree status through gitoxide's parallel status
/// machinery: the HEAD→index tree diff and the index→worktree scan (untracked
/// files included, renames tracked on both sides) run on worker threads.
pub(crate) fn read_status(path: &Path) -> Result<WorkingTree> {
    let repo = open_repository(path)?;
    let items = repo
        .status(gix::progress::Discard)
        .context("prepare worktree status")?
        .untracked_files(gix::status::UntrackedFiles::Files)
        .index_worktree_rewrites(gix::diff::Rewrites::default())
        .into_iter(Vec::new())
        .context("run worktree status")?;
    let mut files = BTreeMap::<PathBuf, WorkingFile>::new();
    for item in items {
        match item.context("read status item")? {
            gix::status::Item::TreeIndex(change) => staged_side(&mut files, &change),
            gix::status::Item::IndexWorktree(item) => worktree_side(&mut files, &item),
        }
    }
    Ok(WorkingTree {
        files: files.into_values().collect(),
    })
}

/// Records one HEAD→index change on the staged column.
fn staged_side(files: &mut BTreeMap<PathBuf, WorkingFile>, change: &gix::diff::index::Change) {
    use gix::diff::index::ChangeRef;
    let (location, old_path, kind) = match change {
        ChangeRef::Addition { location, .. } => (location, None, ChangeKind::Added),
        ChangeRef::Deletion { location, .. } => (location, None, ChangeKind::Deleted),
        ChangeRef::Modification {
            location,
            previous_entry_mode,
            entry_mode,
            ..
        } => (
            location,
            None,
            if mode_class(*previous_entry_mode) == mode_class(*entry_mode) {
                ChangeKind::Modified
            } else {
                ChangeKind::TypeChanged
            },
        ),
        ChangeRef::Rewrite {
            source_location,
            location,
            copy,
            ..
        } => {
            if *copy {
                (location, None, ChangeKind::Added)
            } else {
                (location, Some(source_location), ChangeKind::Renamed)
            }
        }
    };
    let file = file_entry(files, location.as_ref());
    file.staged = Some(kind);
    if let Some(old) = old_path {
        file.old_path = Some(gix::path::from_bstr(old.as_ref()).into_owned());
    }
}

/// Records one index→worktree change on the unstaged column.
fn worktree_side(
    files: &mut BTreeMap<PathBuf, WorkingFile>,
    item: &gix::status::index_worktree::Item,
) {
    use gix::status::index_worktree::{Item, iter::Summary};
    let Some(summary) = item.summary() else {
        return;
    };
    let file = file_entry(files, item.rela_path());
    match summary {
        Summary::Conflict => {
            // Mirror libgit2, which flagged conflicted paths in both columns.
            file.staged = Some(ChangeKind::Conflicted);
            file.unstaged = Some(ChangeKind::Conflicted);
        }
        Summary::Added | Summary::Copied => file.unstaged = Some(ChangeKind::Added),
        Summary::Removed => file.unstaged = Some(ChangeKind::Deleted),
        // Intent-to-add entries surface their pending content here while the
        // HEAD→index diff already reports the staged addition.
        Summary::Modified | Summary::IntentToAdd => file.unstaged = Some(ChangeKind::Modified),
        Summary::TypeChange => file.unstaged = Some(ChangeKind::TypeChanged),
        Summary::Renamed => {
            file.unstaged = Some(ChangeKind::Renamed);
            if let Item::Rewrite { source, .. } = item {
                file.old_path = Some(gix::path::from_bstr(source.rela_path()).into_owned());
            }
        }
    }
}

/// The mutable status row for a repository-relative path.
fn file_entry<'a>(
    files: &'a mut BTreeMap<PathBuf, WorkingFile>,
    rela_path: &BStr,
) -> &'a mut WorkingFile {
    let path = gix::path::from_bstr(rela_path).into_owned();
    files.entry(path.clone()).or_insert(WorkingFile {
        path,
        old_path: None,
        staged: None,
        unstaged: None,
    })
}

/// File, symlink, or submodule class used to mirror libgit2's typechange
/// classification; executable-bit flips stay plain modifications.
fn mode_class(mode: gix::index::entry::Mode) -> u8 {
    if mode.contains(gix::index::entry::Mode::SYMLINK) {
        1
    } else if mode.contains(gix::index::entry::Mode::COMMIT) {
        2
    } else {
        0
    }
}

/// Sorts branches case-insensitively for stable sidebar and menu order.
pub(crate) fn sort_branches(branches: &mut [BranchInfo]) {
    branches.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
}

/// HEAD display name (branch shorthand or "HEAD"), the full reference name of
/// a born branch, and the peeled commit id when one exists.
fn head_state(repo: &gix::Repository) -> Result<(String, Option<BString>, Option<ObjectId>)> {
    let head = repo.head().context("read repository HEAD")?;
    let name = match &head.kind {
        gix::head::Kind::Symbolic(reference) => Some(reference.name.as_bstr().to_owned()),
        gix::head::Kind::Unborn(_) | gix::head::Kind::Detached { .. } => None,
    };
    let display = name
        .as_ref()
        .and_then(|name| name.strip_prefix(b"refs/heads/").and_then(utf8_name))
        .unwrap_or_else(|| "HEAD".to_owned());
    let id = head.id().map(gix::Id::detach);
    Ok((display, name, id))
}

/// Owned shorthand for operational ref names; `None` rejects invalid UTF-8
/// so a mangled name can never be sent back to checkout/delete/rename.
fn utf8_name(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

/// Hashes one reference name and target; callers combine digests with XOR so
/// enumeration order cannot influence the result.
fn reference_digest(name: &BStr, target: TargetRef<'_>) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    name.as_bytes().hash(&mut hasher);
    match target {
        TargetRef::Object(id) => {
            1u8.hash(&mut hasher);
            id.as_bytes().hash(&mut hasher);
        }
        TargetRef::Symbolic(symbolic) => {
            2u8.hash(&mut hasher);
            symbolic.as_bstr().as_bytes().hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// HEAD's contribution to [`refs_signature`].
fn head_digest(head: &str, head_id: Option<&str>) -> u64 {
    let mut hasher = std::hash::DefaultHasher::new();
    head.hash(&mut hasher);
    head_id.hash(&mut hasher);
    hasher.finish()
}

/// Parses the commit fields the graph needs once; the result is cached.
fn parse_commit(
    repo: &gix::Repository,
    info: &gix::traverse::commit::Info,
) -> Result<CachedCommit> {
    let commit = repo
        .find_commit(info.id)
        .with_context(|| format!("load walked commit {}", info.id))?;
    let decoded = commit.decode().context("decode commit")?;
    let message = decoded.message();
    let summary = message.summary();
    let subject = if summary.is_empty() {
        "(no commit message)".to_owned()
    } else {
        lossy(summary.as_ref())
    };
    let description = message
        .body
        .and_then(|body| body.lines().find(|line| !line.trim().is_empty()))
        .map(|line| lossy(line.trim()).chars().take(DESCRIPTION_LIMIT).collect())
        .unwrap_or_default();
    let author = decoded.author().context("parse commit author")?;
    let committer = decoded.committer().context("parse commit committer")?;
    Ok(CachedCommit {
        subject,
        description,
        author: lossy(author.name),
        email: lossy(author.email),
        seconds: committer.seconds(),
        parents: info.parent_ids.iter().map(ToString::to_string).collect(),
    })
}

/// UTF-8 text for display, replacing invalid bytes.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::sort_branches;
    use crate::git::models::{BranchInfo, CommitSummary, RepoSnapshot, WorkingTree};
    use std::path::PathBuf;

    fn branch(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_owned(),
            target: String::new(),
            current: false,
            remote: false,
            upstream: None,
        }
    }

    /// Git filenames are bytes; the status rows must carry them byte-exact
    /// or follow-up stage/diff actions target a nonexistent path. Decoding
    /// is testable everywhere; APFS/NTFS reject such names on disk, so the
    /// filesystem round-trip below is Linux-only.
    #[test]
    #[cfg(unix)]
    fn file_entry_preserves_non_utf8_bytes() {
        use std::os::unix::ffi::OsStrExt;
        let mut files = std::collections::BTreeMap::new();
        let file = super::file_entry(&mut files, b"caf\xe9.txt".as_slice().into());
        assert_eq!(file.path.as_os_str().as_bytes(), b"caf\xe9.txt");
    }

    /// Ref names with invalid UTF-8 must vanish from the snapshot (libgit2
    /// parity) rather than surface as `�` strings that checkout/delete would
    /// send back at a nonexistent ref. Injected through packed-refs content
    /// because filesystems like APFS refuse such bytes in loose ref files.
    #[test]
    fn refs_with_invalid_utf8_names_are_skipped_not_mangled() {
        let directory = tempfile::tempdir().expect("temp dir");
        let repository = git2::Repository::init(directory.path()).expect("init repository");
        let signature = git2::Signature::now("t", "t@t").expect("signature");
        let tree_id = {
            let mut index = repository.index().expect("open index");
            index.write_tree().expect("write tree")
        };
        let tree = repository.find_tree(tree_id).expect("find tree");
        let oid = repository
            .commit(Some("HEAD"), &signature, &signature, "base", &tree, &[])
            .expect("create commit");

        let mut packed = Vec::new();
        packed.extend_from_slice(b"# pack-refs with: peeled fully-peeled sorted \n");
        packed.extend_from_slice(format!("{oid} refs/heads/café-ok\n").as_bytes());
        packed.extend_from_slice(format!("{oid} refs/heads/caf").as_bytes());
        packed.extend_from_slice(b"\xe9-bad\n");
        std::fs::write(directory.path().join(".git/packed-refs"), packed)
            .expect("write packed refs");

        let store = super::RepoStore::new(directory.path().to_path_buf());
        let repo = store.open().expect("open with gitoxide");
        let refs = store.read_refs(&repo).expect("read refs");
        let names = refs
            .branches
            .iter()
            .map(|branch| branch.name.as_str())
            .collect::<Vec<_>>();
        assert!(
            names.contains(&"café-ok"),
            "valid UTF-8 branch survives: {names:?}"
        );
        assert!(
            !names.iter().any(|name| name.contains('\u{FFFD}')),
            "no lossy replacement names may appear: {names:?}"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn status_preserves_non_utf8_paths_byte_exact() {
        use super::read_status;
        use crate::git::models::ChangeKind;
        use std::os::unix::ffi::OsStrExt;
        let directory = tempfile::tempdir().expect("temp dir");
        git2::Repository::init(directory.path()).expect("init repository");
        let name = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        std::fs::write(directory.path().join(name), "content\n").expect("write file");

        let working = read_status(directory.path()).expect("read status");
        assert_eq!(working.files.len(), 1);
        assert_eq!(working.files[0].path.as_os_str(), name);
        assert_eq!(working.files[0].unstaged, Some(ChangeKind::Added));

        // The returned path must be usable for the follow-up index mutation.
        let backend = crate::git::backend::GitBackend::discover(directory.path())
            .expect("discover repository");
        crate::git::backend::Backend::stage(&backend, std::slice::from_ref(&working.files[0].path))
            .expect("stage non-UTF-8 path");
        let staged = read_status(directory.path()).expect("status after stage");
        assert_eq!(staged.files.len(), 1);
        assert_eq!(staged.files[0].path.as_os_str(), name);
        assert_eq!(staged.files[0].staged, Some(ChangeKind::Added));
        assert_eq!(staged.files[0].unstaged, None);
    }

    /// Paging history deepens the live snapshot without deepening what the
    /// next launch must decode and lay out before it can paint.
    #[test]
    fn persisted_snapshots_are_capped_at_the_first_paint_depth() {
        let directory = tempfile::tempdir().expect("temp dir");
        let cache = crate::git::cache::SnapshotCache::at(directory.path()).expect("open cache");
        let repo = PathBuf::from("/tmp/deep");
        let mut store = super::RepoStore::new(repo.clone());

        let deep = super::PERSIST_COMMITS * 4;
        let mut snapshot = paged_snapshot(&repo, deep);
        snapshot.has_more = false;
        store.persist_snapshot(&cache, &snapshot);

        let loaded = cache.load(&repo).expect("cached after persist");
        assert_eq!(loaded.commits.len(), super::PERSIST_COMMITS);
        assert_eq!(loaded.loaded_limit, super::PERSIST_COMMITS);
        assert!(loaded.has_more, "truncation is reported as more history");
        assert_eq!(
            loaded.commits[0].id, snapshot.commits[0].id,
            "the newest rows are the ones kept"
        );

        // A snapshot within the bound is stored whole.
        let mut store = super::RepoStore::new(repo.clone());
        let shallow = paged_snapshot(&repo, 3);
        store.persist_snapshot(&cache, &shallow);
        let loaded = cache.load(&repo).expect("cached");
        assert_eq!(loaded.commits.len(), 3);
        assert!(!loaded.has_more);
    }

    fn paged_snapshot(repo: &std::path::Path, commits: usize) -> RepoSnapshot {
        RepoSnapshot {
            path: repo.to_path_buf(),
            name: "deep".to_owned(),
            head: "main".to_owned(),
            head_id: None,
            branches: Vec::new(),
            tags: Vec::new(),
            stashes: Vec::new(),
            worktrees: Vec::new(),
            commits: (0..commits)
                .map(|index| CommitSummary {
                    id: format!("{index:040}"),
                    short_id: format!("{index:07}"),
                    subject: "row".to_owned(),
                    description: String::new(),
                    author: "t".to_owned(),
                    email: "t@t".to_owned(),
                    authored_seconds: 0,
                    parents: Vec::new(),
                    is_local: true,
                    refs: Vec::new(),
                    branch_refs: Vec::new(),
                })
                .collect(),
            working: WorkingTree::default(),
            loaded_limit: commits,
            has_more: false,
            refs_sig: 1,
            remote_url: None,
        }
    }

    #[test]
    fn branch_order_is_case_insensitive_and_independent_of_head() {
        let mut branches = ["main", "feature/lane-3", "Feature/Detail", "feature/lane-1"]
            .into_iter()
            .map(branch)
            .collect::<Vec<_>>();
        branches[0].current = true;

        sort_branches(&mut branches);
        assert_eq!(
            branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            ["Feature/Detail", "feature/lane-1", "feature/lane-3", "main"]
        );

        for branch in &mut branches {
            branch.current = branch.name == "feature/lane-3";
        }
        sort_branches(&mut branches);
        assert_eq!(
            branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            ["Feature/Detail", "feature/lane-1", "feature/lane-3", "main"]
        );
    }

    #[test]
    fn sort_branches_breaks_case_ties_stably() {
        let mut branches = vec![branch("Zeta"), branch("alpha"), branch("Alpha")];
        sort_branches(&mut branches);
        assert_eq!(
            branches
                .iter()
                .map(|branch| branch.name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "alpha", "Zeta"]
        );
    }
}
