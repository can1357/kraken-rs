use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};
use git2::{
    BranchType, Cred, Delta, Diff, DiffFindOptions, DiffOptions, IndexAddOption, MergeAnalysis,
    ObjectType, Oid, PushOptions, RemoteCallbacks, Repository, RepositoryState, Signature, Sort,
    StashFlags, Status, StatusOptions, TreeWalkMode, TreeWalkResult, build::CheckoutBuilder,
};
use similar::{DiffTag, TextDiff};

use crate::git::models::{
    BranchInfo, ChangeKind, CommitBranchRef, CommitDetail, CommitInput, CommitSummary,
    DiffDocument, DiffLineSelection, DiffRequest, DiffRow, DiffRowKind, DiffScope, FileChange,
    PullOperation, RefKind, RefLabel, RepoSnapshot, StashInfo, WorkingFile, WorkingTree,
    WorktreeInfo,
};
use crate::settings::Settings;

/// Blocking repository contract executed exclusively by the Git worker or CLI verifier.
pub(crate) trait Backend {
    /// Captures branches, refs, status, and a bounded topological history.
    fn snapshot(&self, limit: usize) -> Result<RepoSnapshot>;
    /// Loads metadata and changed paths for one commit.
    fn commit_detail(&self, id: &str) -> Result<CommitDetail>;
    /// Builds an aligned split diff for one committed or working file.
    fn diff(&self, request: &DiffRequest) -> Result<DiffDocument>;
    /// Adds selected worktree paths to the index.
    fn stage(&self, paths: &[PathBuf]) -> Result<()>;
    /// Resets selected index paths to HEAD.
    fn unstage(&self, paths: &[PathBuf]) -> Result<()>;
    /// Applies only the selected worktree diff rows to the index.
    fn stage_lines(&self, path: &Path, lines: &[DiffLineSelection]) -> Result<()>;
    /// Removes only the selected staged diff rows from the index.
    fn unstage_lines(&self, path: &Path, lines: &[DiffLineSelection]) -> Result<()>;
    /// Reverts only the selected worktree diff rows.
    fn discard_lines(&self, path: &Path, lines: &[DiffLineSelection]) -> Result<()>;
    /// Reverts one worktree file to its indexed content; untracked files are removed.
    fn discard_file(&self, path: &Path) -> Result<()>;
    /// Adds every worktree change to the index.
    fn stage_all(&self) -> Result<()>;
    /// Resets the entire index to HEAD.
    fn unstage_all(&self) -> Result<()>;
    /// Creates or amends a commit from the current index.
    fn commit(&self, input: &CommitInput) -> Result<String>;
    /// Rewrites only the HEAD commit's message, keeping its tree untouched.
    fn reword(&self, summary: &str, body: &str) -> Result<String>;
    /// Checks out a local branch or creates a tracking branch for a remote name.
    fn checkout(&self, branch: &str) -> Result<()>;
    /// Creates and checks out a branch at HEAD.
    fn create_branch(&self, branch: &str, target: Option<&str>) -> Result<()>;
    /// Fetches every configured remote, optionally pruning stale tracking refs.
    fn fetch(&self, prune: bool) -> Result<()>;
    /// Registers a new remote (optionally with a distinct push URL) and fetches it.
    fn add_remote(&self, name: &str, url: &str, push_url: Option<&str>) -> Result<()>;
    /// Fetches and integrates the current upstream using GitKraken's selected operation.
    fn pull(&self, operation: PullOperation) -> Result<()>;
    /// Pushes the current branch to its upstream or origin.
    fn push(&self) -> Result<()>;
    /// Writes `git format-patch` output for one commit to `destination`.
    fn save_patch(&self, id: &str, destination: &Path) -> Result<()>;
    /// Saves tracked and untracked WIP in a real stash.
    fn stash(&self) -> Result<()>;
    /// Stashes only the given path, like `git stash push -- <path>`.
    fn stash_file(&self, path: &Path) -> Result<()>;
    /// Applies and drops the newest stash.
    fn pop_stash(&self) -> Result<()>;
    /// Fast-forwards the current branch to the selected branch when possible.
    fn fast_forward(&self, branch: &str) -> Result<()>;
    /// Merges the selected branch into the current branch.
    fn merge(&self, branch: &str) -> Result<()>;
    /// Rebases the current branch onto the selected branch.
    fn rebase(&self, branch: &str) -> Result<()>;
    /// Rebases `source` onto `target` (`git rebase <target> <source>`); git
    /// leaves `source` checked out afterwards.
    fn rebase_onto(&self, source: &str, target: &str) -> Result<()>;
    /// Fast-forwards local branch `target` to `source`; updates the ref in
    /// place when `target` is not checked out.
    fn fast_forward_to(&self, source: &str, target: &str) -> Result<()>;
    /// Creates a tag at a commit, annotating it when a message is supplied.
    fn create_tag(&self, name: &str, target: &str, message: Option<&str>) -> Result<()>;
    /// Applies a commit and records it immediately, like `git cherry-pick`.
    fn cherry_pick(&self, id: &str) -> Result<()>;
    /// Reverts a commit and records the revert immediately.
    fn revert(&self, id: &str) -> Result<()>;
    /// Moves HEAD using the requested reset mode (`soft`, `mixed`, or `hard`).
    fn reset(&self, target: &str, mode: &str) -> Result<()>;
    /// Deletes a local branch, refusing the currently checked out branch.
    fn delete_branch(&self, branch: &str) -> Result<()>;
    /// Deletes a local branch only when it still points at the recorded commit.
    fn delete_branch_at(&self, branch: &str, target: &str) -> Result<()>;
    /// Recreates a local branch at a commit without changing HEAD.
    fn restore_branch(&self, branch: &str, target: &str) -> Result<()>;
    /// Renames a local branch.
    fn rename_branch(&self, branch: &str, new_name: &str) -> Result<()>;
    /// Applies or drops a stash at its snapshot index.
    fn apply_stash(&self, index: usize) -> Result<()>;
    fn drop_stash(&self, index: usize) -> Result<()>;
    /// Deletes a tag reference.
    fn delete_tag(&self, tag: &str) -> Result<()>;
    /// Runs a supported Git LFS repository command.
    fn lfs(&self, operation: LfsOperation) -> Result<()>;
    /// Initializes Gitflow configuration and the develop branch in this repository.
    fn init_gitflow(&self) -> Result<()>;
    /// Configures sparse checkout with the supplied newline-separated paths, or disables it.
    fn sparse_checkout(&self, paths: Option<&str>) -> Result<()>;
    /// Appends one LFS tracking rule to .gitattributes.
    fn track_lfs_pattern(&self, pattern: &str) -> Result<()>;
    /// Appends one ignore pattern to the repository root .gitignore, skipping duplicates.
    fn append_ignore(&self, pattern: &str) -> Result<()>;
}

/// Supported Git LFS operations exposed by the toolbar menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LfsOperation {
    Checkout,
    Pull,
    Push,
    Prune,
}

/// A path-only libgit2 backend; each operation opens a fresh repository handle.
#[derive(Clone, Debug)]
pub(crate) struct GitBackend {
    path: PathBuf,
    settings: Settings,
}
impl GitBackend {
    /// Discovers a repository from a worktree path or descendant.
    pub(crate) fn discover(path: impl AsRef<Path>) -> Result<Self> {
        Self::discover_with_settings(path, Settings::default())
    }

    /// Opens a repository using the active application settings for credential and CLI behavior.
    pub(crate) fn discover_with_settings(
        path: impl AsRef<Path>,
        settings: Settings,
    ) -> Result<Self> {
        let repository = Repository::discover(path.as_ref())
            .with_context(|| format!("discover repository from {}", path.as_ref().display()))?;
        let path = repository
            .workdir()
            .map_or_else(|| repository.path().to_path_buf(), Path::to_path_buf);
        Ok(Self { path, settings })
    }

    /// Returns the canonical worktree path used by worker jobs.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    fn run_git(&self, arguments: &[&str]) -> Result<()> {
        let program = if self.settings.use_git_executable {
            self.settings.git_executable.trim()
        } else {
            "git"
        };
        run_git_program(program, &self.path, arguments)
    }

    fn run_git_capture(&self, arguments: &[&str]) -> Result<Vec<u8>> {
        let program = if self.settings.use_git_executable {
            self.settings.git_executable.trim()
        } else {
            "git"
        };
        run_git_program_output(program, &self.path, arguments)
    }

    fn open(&self) -> Result<Repository> {
        Repository::open(&self.path)
            .with_context(|| format!("open repository {}", self.path.display()))
    }

    fn signed_commit(&self, input: &CommitInput) -> Result<String> {
        let program = self.settings.gpg_program.trim();
        if program.is_empty() {
            bail!("GPG program path is empty");
        }
        Command::new(program)
            .arg("--version")
            .output()
            .with_context(|| format!("run GPG program {program}"))?;
        let mut command = Command::new(if self.settings.use_git_executable {
            self.settings.git_executable.trim()
        } else {
            "git"
        });
        command
            .current_dir(&self.path)
            .arg("-c")
            .arg(format!("gpg.program={program}"));
        if !self.settings.gpg_key_id.trim().is_empty() {
            command
                .arg("-c")
                .arg(format!("user.signingkey={}", self.settings.gpg_key_id));
        }
        if let Some(profile) = self
            .settings
            .selected_profile
            .as_ref()
            .and_then(|selected| {
                self.settings
                    .profiles
                    .iter()
                    .find(|profile| &profile.name == selected)
            })
        {
            command
                .arg("-c")
                .arg(format!("user.name={}", profile.author_name));
            command
                .arg("-c")
                .arg(format!("user.email={}", profile.author_email));
        }
        command.arg("commit").arg("-S");
        if input.amend {
            command.arg("--amend");
        }
        command.arg("-m").arg(input.summary.trim());
        if !input.body.trim().is_empty() {
            command.arg("-m").arg(input.body.trim());
        }
        let output = command.output().context("run signed git commit")?;
        if !output.status.success() {
            bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
        }
        let repository = self.open()?;
        Ok(repository
            .head()?
            .target()
            .context("resolve signed commit")?
            .to_string())
    }
}

impl Backend for GitBackend {
    fn snapshot(&self, limit: usize) -> Result<RepoSnapshot> {
        let mut repository = self.open()?;
        let head = head_name(&repository);
        let head_id = repository
            .head()
            .ok()
            .and_then(|reference| reference.peel_to_commit().ok())
            .map(|commit| commit.id().to_string());
        let branches = read_branches(&repository)?;
        let worktrees = read_worktrees(&repository)?;
        let ref_index = read_labels(&repository, &branches, &worktrees, head_id.as_deref())?;
        let stashes = read_stashes(&mut repository)?;
        let working = read_status(&repository)?;
        let (commits, has_more) = read_commits(
            &repository,
            limit.max(1),
            &ref_index.by_commit,
            &ref_index.branch_refs_by_commit,
        )?;
        let name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("repository")
            .to_owned();
        Ok(RepoSnapshot {
            path: self.path.clone(),
            name,
            head,
            head_id,
            branches,
            tags: ref_index.tags,
            stashes,
            worktrees,
            commits,
            working,
            loaded_limit: limit,
            has_more,
        })
    }

    fn commit_detail(&self, id: &str) -> Result<CommitDetail> {
        let repository = self.open()?;
        let oid = Oid::from_str(id).with_context(|| format!("parse commit id {id}"))?;
        let commit = repository
            .find_commit(oid)
            .with_context(|| format!("find commit {id}"))?;
        let tree = commit.tree().context("load selected commit tree")?;
        let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());
        let mut options = DiffOptions::new();
        options.include_typechange(true);
        let mut diff = repository
            .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))
            .context("diff selected commit")?;
        diff.find_similar(Some(DiffFindOptions::new().renames(true)))
            .context("detect commit renames")?;
        let mut files = file_changes(&diff)?;
        files.sort_by(|left, right| left.path.cmp(&right.path));
        let all_files = tree_paths(&tree)?;
        let message = commit.message().unwrap_or_default();
        let body = commit.body().unwrap_or_default().trim().to_owned();
        let author = commit.author();
        let parents = commit
            .parent_ids()
            .map(|parent| parent.to_string())
            .collect();
        Ok(CommitDetail {
            id: commit.id().to_string(),
            short_id: short_id(commit.id()),
            subject: commit.summary().unwrap_or("(no commit message)").to_owned(),
            body: body.clone(),
            author: author.name().unwrap_or("Unknown author").to_owned(),
            email: author.email().unwrap_or_default().to_owned(),
            authored_seconds: commit.time().seconds(),
            parents,
            files,
            all_files,
            conflicts: parse_conflicts(message),
            is_local: !is_reachable_from_remote(&repository, commit.id()),
        })
    }

    fn diff(&self, request: &DiffRequest) -> Result<DiffDocument> {
        let repository = self.open()?;
        let mut options = DiffOptions::new();
        options
            .pathspec(&request.path)
            .include_typechange(true)
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .show_untracked_content(true);
        let diff = match &request.scope {
            DiffScope::Commit(id) => {
                let oid = Oid::from_str(id).with_context(|| format!("parse commit id {id}"))?;
                let commit = repository.find_commit(oid).context("find diff commit")?;
                let tree = commit.tree().context("load diff commit tree")?;
                let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());
                repository
                    .diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut options))
                    .context("create committed file diff")?
            }
            DiffScope::Staged => {
                let head_tree = repository
                    .head()
                    .ok()
                    .and_then(|head| head.peel_to_tree().ok());
                let index = repository.index().context("open repository index")?;
                repository
                    .diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut options))
                    .context("create staged file diff")?
            }
            DiffScope::Unstaged => repository
                .diff_index_to_workdir(None, Some(&mut options))
                .context("create unstaged file diff")?,
        };
        let mut document = parse_diff(&diff, request)?;
        document.content = file_content(
            &repository,
            &self.path,
            request,
            &self.settings.default_encoding,
        )?;
        if !document.binary {
            let (old, new) = diff_side_content(
                &repository,
                &self.path,
                request,
                &self.settings.default_encoding,
            )?;
            let (rows, hunks) = full_diff_rows(
                old.as_deref().unwrap_or_default(),
                new.as_deref().unwrap_or_default(),
            );
            document.rows = rows;
            document.hunks = hunks;
        }
        Ok(document)
    }

    fn stage(&self, paths: &[PathBuf]) -> Result<()> {
        let repository = self.open()?;
        let expanded = expanded_paths(&repository, paths, false)?;
        let mut index = repository.index().context("open repository index")?;
        for path in &expanded {
            if self.path.join(path).exists() {
                index
                    .add_path(path)
                    .with_context(|| format!("stage {}", path.display()))?;
            } else {
                index
                    .remove_path(path)
                    .with_context(|| format!("stage deletion {}", path.display()))?;
            }
        }
        index.write().context("write staged index")
    }
    fn unstage(&self, paths: &[PathBuf]) -> Result<()> {
        let repository = self.open()?;
        let expanded = expanded_paths(&repository, paths, true)?;
        if let Ok(head) = repository
            .head()
            .and_then(|head| head.peel(ObjectType::Commit))
        {
            repository
                .reset_default(Some(&head), expanded.iter())
                .context("reset selected index paths")
        } else {
            let mut index = repository.index().context("open repository index")?;

            for path in &expanded {
                let _ = index.remove_path(path);
            }
            index.write().context("write reset index")
        }
    }

    fn stage_lines(&self, path: &Path, lines: &[DiffLineSelection]) -> Result<()> {
        let repository = self.open()?;
        let baseline = index_content(&repository, path)?;
        let target = selected_diff_content(
            &repository,
            path,
            LineDiff::Unstaged,
            baseline.as_deref().unwrap_or_default(),
            lines,
            true,
        )?;
        let remove = target.is_empty() && !self.path.join(path).exists();
        write_index_content(&repository, path, &target, remove)
    }

    fn unstage_lines(&self, path: &Path, lines: &[DiffLineSelection]) -> Result<()> {
        let repository = self.open()?;
        let baseline = head_content(&repository, path)?;
        let target = selected_diff_content(
            &repository,
            path,
            LineDiff::Staged,
            baseline.as_deref().unwrap_or_default(),
            lines,
            false,
        )?;
        let remove = target.is_empty() && baseline.is_none();
        write_index_content(&repository, path, &target, remove)
    }

    fn discard_lines(&self, path: &Path, lines: &[DiffLineSelection]) -> Result<()> {
        let repository = self.open()?;
        let baseline = index_content(&repository, path)?;
        let target = selected_diff_content(
            &repository,
            path,
            LineDiff::Unstaged,
            baseline.as_deref().unwrap_or_default(),
            lines,
            false,
        )?;
        let worktree_path = self.path.join(path);
        if target.is_empty() && baseline.is_none() {
            if worktree_path.exists() {
                std::fs::remove_file(&worktree_path)
                    .with_context(|| format!("discard selected lines in {}", path.display()))?;
            }
        } else {
            std::fs::write(&worktree_path, target)
                .with_context(|| format!("discard selected lines in {}", path.display()))?;
        }
        Ok(())
    }

    fn discard_file(&self, path: &Path) -> Result<()> {
        let repository = self.open()?;
        let baseline = index_content(&repository, path)?;
        let worktree_path = self.path.join(path);
        match baseline {
            Some(content) => std::fs::write(&worktree_path, content)
                .with_context(|| format!("restore {} from index", path.display()))?,
            None => {
                if worktree_path.exists() {
                    std::fs::remove_file(&worktree_path)
                        .with_context(|| format!("remove untracked {}", path.display()))?;
                }
            }
        }
        Ok(())
    }

    fn stage_all(&self) -> Result<()> {
        let repository = self.open()?;
        let mut index = repository.index().context("open repository index")?;
        index
            .add_all(["*"], IndexAddOption::DEFAULT, None)
            .context("stage all additions and modifications")?;
        index
            .update_all(["*"], None)
            .context("stage all tracked deletions")?;
        index.write().context("write fully staged index")
    }

    fn unstage_all(&self) -> Result<()> {
        let repository = self.open()?;
        if let Ok(head) = repository
            .head()
            .and_then(|head| head.peel(ObjectType::Commit))
        {
            repository
                .reset_default(Some(&head), [Path::new("*")])
                .context("reset complete index")?;
        } else {
            let mut index = repository.index().context("open repository index")?;
            index.clear().context("clear unborn index")?;
            index.write().context("write cleared index")?;
        }
        Ok(())
    }

    fn commit(&self, input: &CommitInput) -> Result<String> {
        let summary = input.summary.trim();
        if summary.is_empty() {
            bail!("commit summary cannot be empty");
        }
        let repository = self.open()?;
        if self.settings.sign_commits_by_default {
            return self.signed_commit(input);
        }
        if repository.state() != RepositoryState::Clean {
            bail!("repository is in {:?} state", repository.state());
        }
        let mut index = repository.index().context("open repository index")?;
        if index.has_conflicts() {
            bail!("resolve index conflicts before committing");
        }
        if !input.amend {
            let head_tree = repository
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok())
                .and_then(|head| head.tree().ok());
            let staged = repository
                .diff_tree_to_index(head_tree.as_ref(), Some(&index), None)
                .context("inspect staged changes")?;
            if staged.deltas().len() == 0 {
                bail!("no staged changes to commit");
            }
        }
        let tree_id = index.write_tree().context("write commit tree")?;
        let tree = repository.find_tree(tree_id).context("load commit tree")?;
        let signature = self
            .settings
            .selected_profile
            .as_ref()
            .and_then(|selected| {
                self.settings
                    .profiles
                    .iter()
                    .find(|profile| &profile.name == selected)
            })
            .and_then(|profile| {
                (!profile.author_name.trim().is_empty() && !profile.author_email.trim().is_empty())
                    .then(|| Signature::now(&profile.author_name, &profile.author_email).ok())
                    .flatten()
            })
            .or_else(|| repository.signature().ok())
            .unwrap_or_else(|| {
                Signature::now("Kraken Native", "kraken@localhost")
                    .expect("fallback signature has valid static fields")
            });
        let message = if input.body.trim().is_empty() {
            summary.to_owned()
        } else {
            format!("{summary}\n\n{}", input.body.trim())
        };
        let id = if input.amend {
            let head = repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .context("load commit to amend")?;
            head.amend(
                Some("HEAD"),
                None,
                Some(&signature),
                None,
                Some(&message),
                Some(&tree),
            )
            .context("amend commit")?
        } else {
            let parents = repository
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok())
                .into_iter()
                .collect::<Vec<_>>();
            let parent_refs = parents.iter().collect::<Vec<_>>();
            repository
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &message,
                    &tree,
                    &parent_refs,
                )
                .context("create commit")?
        };
        Ok(id.to_string())
    }

    fn reword(&self, summary: &str, body: &str) -> Result<String> {
        let summary = summary.trim();
        if summary.is_empty() {
            bail!("commit summary cannot be empty");
        }
        let repository = self.open()?;
        let message = if body.trim().is_empty() {
            summary.to_owned()
        } else {
            format!("{summary}\n\n{}", body.trim())
        };
        let head = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .context("load commit to reword")?;
        let id = head
            .amend(Some("HEAD"), None, None, None, Some(&message), None)
            .context("reword commit")?;
        Ok(id.to_string())
    }

    fn checkout(&self, branch: &str) -> Result<()> {
        let repository = self.open()?;
        if let Ok(local) = repository.find_branch(branch, BranchType::Local) {
            checkout_local(&repository, &local, branch)
        } else if let Ok(remote) = repository.find_branch(branch, BranchType::Remote) {
            let commit = remote
                .get()
                .peel_to_commit()
                .context("peel remote branch")?;
            let local_name = branch.split_once('/').map_or(branch, |(_, name)| name);
            let mut local = repository
                .branch(local_name, &commit, false)
                .with_context(|| format!("create tracking branch {local_name}"))?;
            local
                .set_upstream(Some(branch))
                .with_context(|| format!("track {branch}"))?;
            checkout_local(&repository, &local, local_name)
        } else {
            let target = repository
                .revparse_single(branch)
                .with_context(|| format!("resolve checkout target {branch}"))?;
            checkout_tree_reporting(&repository, &target, "checkout")?;
            repository
                .set_head_detached(target.id())
                .with_context(|| format!("detach HEAD at {branch}"))
        }
    }

    fn create_branch(&self, branch: &str, target: Option<&str>) -> Result<()> {
        let branch = branch.trim();
        if branch.is_empty() {
            bail!("branch name cannot be empty");
        }
        let repository = self.open()?;
        let commit = match target {
            Some(target) => repository
                .revparse_single(target)
                .with_context(|| format!("resolve branch target {target}"))?
                .peel_to_commit()
                .context("peel branch target to commit")?,
            None => repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .context("load HEAD for new branch")?,
        };
        let local = repository
            .branch(branch, &commit, false)
            .with_context(|| format!("create branch {branch}"))?;
        checkout_local(&repository, &local, branch)
    }

    fn fetch(&self, prune: bool) -> Result<()> {
        if prune {
            self.run_git(&["fetch", "--all", "--prune"])
        } else {
            self.run_git(&["fetch", "--all"])
        }
    }

    /// Registers a new remote, optionally with a distinct push URL, then
    /// fetches it so its branches appear immediately. The remote persists
    /// even when that first fetch fails, so a bad URL surfaces as a fetch
    /// error instead of silently losing the configuration.
    fn add_remote(&self, name: &str, url: &str, push_url: Option<&str>) -> Result<()> {
        let repository = self.open()?;
        repository
            .remote(name, url)
            .with_context(|| format!("add remote {name}"))?;
        if let Some(push_url) = push_url {
            repository
                .remote_set_pushurl(name, Some(push_url))
                .with_context(|| format!("set push URL for remote {name}"))?;
        }
        self.run_git(&["fetch", name])
            .with_context(|| format!("fetch new remote {name}"))
    }

    fn save_patch(&self, id: &str, destination: &Path) -> Result<()> {
        let patch = self.run_git_capture(&["format-patch", "-1", id, "--stdout"])?;
        std::fs::write(destination, patch)
            .with_context(|| format!("write patch {}", destination.display()))
    }

    fn pull(&self, operation: PullOperation) -> Result<()> {
        match operation {
            PullOperation::FetchAll => self.fetch(false),
            PullOperation::FastForward => self.run_git(&["pull", "--ff"]),
            PullOperation::FastForwardOnly => self.run_git(&["pull", "--ff-only"]),
            PullOperation::Rebase => self.run_git(&["pull", "--rebase=true"]),
        }
    }

    fn push(&self) -> Result<()> {
        let repository = self.open()?;
        let branch_name = current_branch(&repository)?;
        let mut branch = repository
            .find_branch(&branch_name, BranchType::Local)
            .context("find current branch")?;
        let configured_upstream = branch
            .upstream()
            .ok()
            .and_then(|upstream| upstream.name().ok().flatten().map(str::to_owned));
        let (remote_name, remote_branch) = configured_upstream
            .as_deref()
            .and_then(|name| name.split_once('/'))
            .map_or_else(
                || ("origin".to_owned(), branch_name.clone()),
                |(remote, branch)| (remote.to_owned(), branch.to_owned()),
            );
        let mut remote = repository
            .find_remote(&remote_name)
            .with_context(|| format!("find remote {remote_name}"))?;
        let callbacks = remote_callbacks(&repository, &self.settings)?;
        let mut options = PushOptions::new();
        options.remote_callbacks(callbacks);
        let refspec = format!("refs/heads/{branch_name}:refs/heads/{remote_branch}");
        remote
            .push(&[&refspec], Some(&mut options))
            .with_context(|| format!("push {branch_name} to {remote_name}"))?;
        drop(remote);
        if configured_upstream.is_none() {
            branch
                .set_upstream(Some(&format!("{remote_name}/{remote_branch}")))
                .context("record pushed branch upstream")?;
        }
        Ok(())
    }

    fn stash(&self) -> Result<()> {
        let mut repository = self.open()?;
        let signature = repository.signature().unwrap_or_else(|_| {
            Signature::now("Kraken Native", "kraken@localhost")
                .expect("fallback signature has valid static fields")
        });
        repository
            .stash_save(
                &signature,
                "Kraken Native WIP",
                Some(StashFlags::INCLUDE_UNTRACKED),
            )
            .context("create stash")?;
        Ok(())
    }

    fn stash_file(&self, path: &Path) -> Result<()> {
        // libgit2's stash pathspec still resets unrelated untracked files, so
        // this op goes through the CLI for exact `git stash push` semantics.
        let spec = path.to_string_lossy();
        self.run_git(&["stash", "push", "--include-untracked", "--", spec.as_ref()])
            .with_context(|| format!("stash {}", path.display()))
    }

    fn pop_stash(&self) -> Result<()> {
        let mut repository = self.open()?;
        repository.stash_pop(0, None).context("pop newest stash")
    }

    fn fast_forward(&self, branch: &str) -> Result<()> {
        let repository = self.open()?;
        let target = resolve_branch_commit(&repository, branch)?;
        let annotated = repository
            .find_annotated_commit(target.id())
            .context("annotate fast-forward target")?;
        let (analysis, _) = repository
            .merge_analysis(&[&annotated])
            .context("analyze fast-forward")?;
        if !analysis.contains(MergeAnalysis::ANALYSIS_FASTFORWARD) {
            bail!("current branch cannot fast-forward to {branch}");
        }
        fast_forward_to(
            &repository,
            target.id(),
            &format!("Fast-forward to {branch}"),
        )
    }

    fn rebase_onto(&self, source: &str, target: &str) -> Result<()> {
        self.run_git(&["rebase", target, source])
    }

    fn fast_forward_to(&self, source: &str, target: &str) -> Result<()> {
        let repository = self.open()?;
        let head = repository
            .head()
            .ok()
            .and_then(|head| head.shorthand().map(str::to_owned));
        if head.as_deref() == Some(target) {
            return self.fast_forward(source);
        }
        self.run_git(&["fetch", ".", &format!("{source}:{target}")])
    }

    fn merge(&self, branch: &str) -> Result<()> {
        let repository = self.open()?;
        let target = resolve_branch_commit(&repository, branch)?;
        integrate_commit(&repository, &target, &format!("Merge branch '{branch}'"))
    }

    fn rebase(&self, branch: &str) -> Result<()> {
        let repository = self.open()?;
        let target = resolve_branch_commit(&repository, branch)?;
        let upstream = repository
            .find_annotated_commit(target.id())
            .context("annotate rebase target")?;
        let signature = repository.signature().unwrap_or_else(|_| {
            Signature::now("Kraken Native", "kraken@localhost")
                .expect("fallback signature has valid static fields")
        });
        let mut rebase = repository
            .rebase(None, Some(&upstream), None, None)
            .with_context(|| format!("start rebase onto {branch}"))?;
        while let Some(operation) = rebase.next() {
            operation.context("advance rebase")?;
            let index = repository.index().context("read rebase index")?;
            if index.has_conflicts() {
                bail!("rebase onto {branch} produced conflicts");
            }
            rebase
                .commit(None, &signature, None)
                .context("commit rebased operation")?;
        }
        rebase.finish(Some(&signature)).context("finish rebase")
    }
    fn create_tag(&self, name: &str, target: &str, message: Option<&str>) -> Result<()> {
        if self.settings.sign_tags_by_default {
            let message = message
                .filter(|message| !message.trim().is_empty())
                .unwrap_or(name);
            let program = self.settings.gpg_program.trim();
            if program.is_empty() {
                bail!("GPG program path is empty");
            }
            Command::new(program)
                .arg("--version")
                .output()
                .with_context(|| format!("run GPG program {program}"))?;
            let mut command = Command::new(if self.settings.use_git_executable {
                self.settings.git_executable.trim()
            } else {
                "git"
            });
            command
                .current_dir(&self.path)
                .arg("-c")
                .arg(format!("gpg.program={program}"));
            if !self.settings.gpg_key_id.trim().is_empty() {
                command
                    .arg("-c")
                    .arg(format!("user.signingkey={}", self.settings.gpg_key_id));
            }
            let output = command
                .arg("tag")
                .arg("-s")
                .arg(name.trim())
                .arg(target)
                .arg("-m")
                .arg(message)
                .output()
                .context("run signed git tag")?;
            if !output.status.success() {
                bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
            }
            return Ok(());
        }
        let repository = self.open()?;
        let object = repository
            .revparse_single(target)
            .with_context(|| format!("resolve tag target {target}"))?;
        if let Some(message) = message.filter(|message| !message.trim().is_empty()) {
            let signature = repository.signature().unwrap_or_else(|_| {
                Signature::now("Kraken Native", "kraken@localhost").expect("static signature")
            });
            repository
                .tag(name.trim(), &object, &signature, message, false)
                .with_context(|| format!("create annotated tag {name}"))?;
        } else {
            repository
                .tag_lightweight(name.trim(), &object, false)
                .with_context(|| format!("create lightweight tag {name}"))?;
        }
        Ok(())
    }

    fn cherry_pick(&self, id: &str) -> Result<()> {
        self.run_git(&["cherry-pick", id])
    }

    fn revert(&self, id: &str) -> Result<()> {
        self.run_git(&["revert", "--no-edit", id])
    }

    fn reset(&self, target: &str, mode: &str) -> Result<()> {
        let flag = match mode {
            "soft" => "--soft",
            "mixed" => "--mixed",
            "hard" => "--hard",
            _ => bail!("unknown reset mode {mode}"),
        };
        self.run_git(&["reset", flag, target])
    }

    fn delete_branch(&self, branch: &str) -> Result<()> {
        let repository = self.open()?;
        if current_branch(&repository)? == branch {
            bail!("Cannot delete the branch '{branch}' which you are currently on.");
        }
        let mut branch_ref = repository
            .find_branch(branch, BranchType::Local)
            .with_context(|| format!("find branch {branch}"))?;
        branch_ref
            .delete()
            .with_context(|| format!("delete branch {branch}"))
    }

    fn delete_branch_at(&self, branch: &str, target: &str) -> Result<()> {
        let repository = self.open()?;
        if current_branch(&repository)? == branch {
            bail!("Cannot delete the branch '{branch}' which you are currently on.");
        }
        let mut branch_ref = repository
            .find_branch(branch, BranchType::Local)
            .with_context(|| format!("find branch {branch}"))?;
        let actual = branch_ref
            .get()
            .target()
            .map(|oid| oid.to_string())
            .context("read branch target")?;
        if actual != target {
            bail!("branch {branch} changed since the operation");
        }
        branch_ref
            .delete()
            .with_context(|| format!("delete branch {branch}"))
    }

    fn restore_branch(&self, branch: &str, target: &str) -> Result<()> {
        let repository = self.open()?;
        let commit = repository
            .revparse_single(target)
            .with_context(|| format!("resolve branch target {target}"))?
            .peel_to_commit()
            .context("peel branch target to commit")?;
        repository
            .branch(branch, &commit, false)
            .with_context(|| format!("restore branch {branch}"))?;
        Ok(())
    }

    fn rename_branch(&self, branch: &str, new_name: &str) -> Result<()> {
        let repository = self.open()?;
        let mut branch_ref = repository
            .find_branch(branch, BranchType::Local)
            .with_context(|| format!("find branch {branch}"))?;
        branch_ref
            .rename(new_name.trim(), false)
            .with_context(|| format!("rename branch {branch} to {new_name}"))?;
        Ok(())
    }

    fn apply_stash(&self, index: usize) -> Result<()> {
        let mut repository = self.open()?;
        repository
            .stash_apply(index, None)
            .with_context(|| format!("apply stash@{{{index}}}"))
    }

    fn drop_stash(&self, index: usize) -> Result<()> {
        let mut repository = self.open()?;
        repository
            .stash_drop(index)
            .with_context(|| format!("drop stash@{{{index}}}"))
    }

    fn delete_tag(&self, tag: &str) -> Result<()> {
        let repository = self.open()?;
        repository
            .find_reference(&format!("refs/tags/{tag}"))
            .with_context(|| format!("find tag {tag}"))?
            .delete()
            .with_context(|| format!("delete tag {tag}"))
    }

    fn lfs(&self, operation: LfsOperation) -> Result<()> {
        let arguments: &[&str] = match operation {
            LfsOperation::Checkout => &["lfs", "checkout"],
            LfsOperation::Pull => &["lfs", "pull"],
            LfsOperation::Push => &["lfs", "push", "--all", "origin"],
            LfsOperation::Prune => &["lfs", "prune"],
        };
        let program = if self.settings.use_git_executable {
            self.settings.git_executable.trim()
        } else {
            "git"
        };
        let output = Command::new(program)
            .args(arguments)
            .current_dir(&self.path)
            .output()
            .with_context(|| format!("run {program} {}", arguments.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            bail!(
                "git {} failed{}",
                arguments.join(" "),
                if stderr.is_empty() {
                    String::new()
                } else {
                    format!(": {stderr}")
                }
            );
        }
        Ok(())
    }

    fn init_gitflow(&self) -> Result<()> {
        for (key, value) in [
            (
                "gitflow.branch.master",
                self.settings.gitflow_main_branch.as_str(),
            ),
            (
                "gitflow.branch.develop",
                self.settings.gitflow_develop_branch.as_str(),
            ),
            (
                "gitflow.prefix.feature",
                self.settings.gitflow_feature_prefix.as_str(),
            ),
            (
                "gitflow.prefix.release",
                self.settings.gitflow_release_prefix.as_str(),
            ),
            (
                "gitflow.prefix.hotfix",
                self.settings.gitflow_hotfix_prefix.as_str(),
            ),
        ] {
            self.run_git(&["config", key, value])?;
        }
        let repository = self.open()?;
        if repository
            .find_branch(&self.settings.gitflow_develop_branch, BranchType::Local)
            .is_err()
        {
            let head = repository
                .head()?
                .peel_to_commit()
                .context("find HEAD for develop branch")?;
            repository
                .branch(&self.settings.gitflow_develop_branch, &head, false)
                .context("create Gitflow develop branch")?;
        }
        Ok(())
    }

    fn sparse_checkout(&self, paths: Option<&str>) -> Result<()> {
        let Some(paths) = paths else {
            return self.run_git(&["sparse-checkout", "disable"]);
        };
        let entries = paths
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return self.run_git(&["sparse-checkout", "disable"]);
        }
        let mut arguments = vec!["sparse-checkout", "set"];
        arguments.extend(entries);
        self.run_git(&arguments)
    }

    fn track_lfs_pattern(&self, pattern: &str) -> Result<()> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            bail!("LFS tracking pattern cannot be empty");
        }
        self.run_git(&["lfs", "track", pattern])
    }

    fn append_ignore(&self, pattern: &str) -> Result<()> {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            bail!("ignore pattern cannot be empty");
        }
        let path = self.path.join(".gitignore");
        let mut content = std::fs::read_to_string(&path).unwrap_or_default();
        if content.lines().any(|line| line.trim() == pattern) {
            return Ok(());
        }
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(pattern);
        content.push('\n');
        std::fs::write(&path, content).with_context(|| format!("append {pattern} to .gitignore"))
    }
}

#[derive(Clone, Copy)]
enum LineDiff {
    Staged,
    Unstaged,
}

fn run_git_program(program: &str, path: &Path, arguments: &[&str]) -> Result<()> {
    run_git_program_output(program, path, arguments).map(|_| ())
}

/// Runs git and returns captured stdout, surfacing stderr as the error text.
fn run_git_program_output(program: &str, path: &Path, arguments: &[&str]) -> Result<Vec<u8>> {
    if program.is_empty() {
        bail!("Git executable path is empty");
    }
    let output = Command::new(program)
        .args(arguments)
        .current_dir(path)
        .output()
        .with_context(|| format!("run {program} {}", arguments.join(" ")))?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        bail!("{program} {} failed", arguments.join(" "));
    }
    bail!("{stderr}")
}

fn index_content(repository: &Repository, path: &Path) -> Result<Option<Vec<u8>>> {
    let index = repository.index().context("open repository index")?;
    index
        .get_path(path, 0)
        .map(|entry| {
            repository
                .find_blob(entry.id)
                .map(|blob| blob.content().to_vec())
                .context("read index blob")
        })
        .transpose()
}

fn head_content(repository: &Repository, path: &Path) -> Result<Option<Vec<u8>>> {
    let Ok(head) = repository.head().and_then(|head| head.peel_to_commit()) else {
        return Ok(None);
    };
    let tree = head.tree().context("load HEAD tree")?;
    match tree.get_path(path) {
        Ok(entry) => repository
            .find_blob(entry.id())
            .map(|blob| Some(blob.content().to_vec()))
            .context("read HEAD blob"),
        Err(error) if error.code() == git2::ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn selected_diff_content(
    repository: &Repository,
    path: &Path,
    scope: LineDiff,
    baseline: &[u8],
    selections: &[DiffLineSelection],
    apply_selected: bool,
) -> Result<Vec<u8>> {
    let mut options = DiffOptions::new();
    options
        .pathspec(path)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    let diff = match scope {
        LineDiff::Staged => {
            let head_tree = repository
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok())
                .and_then(|commit| commit.tree().ok());
            let index = repository.index().context("open repository index")?;
            repository
                .diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut options))
                .context("create staged line diff")?
        }
        LineDiff::Unstaged => repository
            .diff_index_to_workdir(None, Some(&mut options))
            .context("create unstaged line diff")?,
    };
    let mut output = Vec::with_capacity(baseline.len());
    let mut old_cursor = 0usize;
    for diff_index in 0..diff.deltas().len() {
        let delta = diff.get_delta(diff_index).context("read line diff delta")?;
        if delta.new_file().path() != Some(path) && delta.old_file().path() != Some(path) {
            continue;
        }
        let file_patch =
            git2::Patch::from_diff(&diff, diff_index).context("open line diff patch")?;
        let Some(file_patch) = file_patch else {
            continue;
        };
        for hunk_index in 0..file_patch.num_hunks() {
            let (_, line_count) = file_patch.hunk(hunk_index).context("read line diff hunk")?;
            for line_index in 0..line_count {
                let line = file_patch
                    .line_in_hunk(hunk_index, line_index)
                    .context("read line diff row")?;
                let old_line = line.old_lineno();
                let new_line = line.new_lineno();
                let include_change = match line.origin() {
                    '-' => selections
                        .iter()
                        .any(|selection| selection.old_line == old_line),
                    '+' => selections
                        .iter()
                        .any(|selection| selection.new_line == new_line),
                    _ => false,
                };
                match line.origin() {
                    ' ' => append_through(&mut output, baseline, &mut old_cursor, old_line)?,
                    '-' => {
                        append_before(&mut output, baseline, &mut old_cursor, old_line)?;
                        let take = if apply_selected {
                            include_change
                        } else {
                            !include_change
                        };
                        if take {
                            old_cursor = old_cursor.saturating_add(1);
                        } else {
                            append_line_at(&mut output, baseline, &mut old_cursor, old_line)?;
                        }
                    }
                    '+' => {
                        let take = if apply_selected {
                            include_change
                        } else {
                            !include_change
                        };
                        if take {
                            output.extend_from_slice(line.content());
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    append_rest(&mut output, baseline, &mut old_cursor);
    Ok(output)
}

fn line_ranges(content: &[u8]) -> Vec<&[u8]> {
    content.split_inclusive(|byte| *byte == b'\n').collect()
}

fn append_before(
    output: &mut Vec<u8>,
    baseline: &[u8],
    cursor: &mut usize,
    line: Option<u32>,
) -> Result<()> {
    let Some(line) = line else {
        return Ok(());
    };
    let target = usize::try_from(line.saturating_sub(1)).context("convert old line number")?;
    let lines = line_ranges(baseline);
    while *cursor < target {
        if let Some(content) = lines.get(*cursor) {
            output.extend_from_slice(content);
        }
        *cursor += 1;
    }
    Ok(())
}

fn append_line_at(
    output: &mut Vec<u8>,
    baseline: &[u8],
    cursor: &mut usize,
    line: Option<u32>,
) -> Result<()> {
    append_before(output, baseline, cursor, line)?;
    if let Some(content) = line_ranges(baseline).get(*cursor) {
        output.extend_from_slice(content);
    }
    *cursor += 1;
    Ok(())
}

fn append_through(
    output: &mut Vec<u8>,
    baseline: &[u8],
    cursor: &mut usize,
    line: Option<u32>,
) -> Result<()> {
    append_line_at(output, baseline, cursor, line)
}

fn append_rest(output: &mut Vec<u8>, baseline: &[u8], cursor: &mut usize) {
    for content in line_ranges(baseline).into_iter().skip(*cursor) {
        output.extend_from_slice(content);
    }
    *cursor = line_ranges(baseline).len();
}

fn write_index_content(
    repository: &Repository,
    path: &Path,
    content: &[u8],
    remove: bool,
) -> Result<()> {
    let mut index = repository.index().context("open repository index")?;
    if remove {
        let _ = index.remove_path(path);
    } else {
        if index.get_path(path, 0).is_none() {
            index
                .add_path(path)
                .with_context(|| format!("prepare index entry for {}", path.display()))?;
        }
        let entry = index
            .get_path(path, 0)
            .ok_or_else(|| anyhow!("missing index entry for {}", path.display()))?;
        index
            .add_frombuffer(&entry, content)
            .with_context(|| format!("write selected lines to index for {}", path.display()))?;
    }
    index.write().context("write selected-line index")
}

fn head_name(repository: &Repository) -> String {
    repository
        .head()
        .ok()
        .and_then(|head| head.shorthand().map(str::to_owned))
        .unwrap_or_else(|| "HEAD".to_owned())
}

fn current_branch(repository: &Repository) -> Result<String> {
    let head = repository.head().context("read HEAD")?;
    if !head.is_branch() {
        bail!("HEAD is detached");
    }
    head.shorthand()
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("current branch name is not UTF-8"))
}

fn read_branches(repository: &Repository) -> Result<Vec<BranchInfo>> {
    let mut branches = Vec::new();
    for branch_type in [BranchType::Local, BranchType::Remote] {
        for result in repository
            .branches(Some(branch_type))
            .context("enumerate branches")?
        {
            let (branch, kind) = result.context("read branch")?;
            let Some(name) = branch.name().context("read branch name")? else {
                continue;
            };
            let target = branch
                .get()
                .peel_to_commit()
                .map(|commit| commit.id().to_string())
                .unwrap_or_default();
            let upstream = if kind == BranchType::Local {
                branch
                    .upstream()
                    .ok()
                    .and_then(|upstream| upstream.name().ok().flatten().map(str::to_owned))
            } else {
                None
            };
            branches.push(BranchInfo {
                name: name.to_owned(),
                target,
                current: branch.is_head(),
                remote: kind == BranchType::Remote,
                upstream,
            });
        }
    }
    sort_branches(&mut branches);
    Ok(branches)
}

fn sort_branches(branches: &mut [BranchInfo]) {
    branches.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
}

struct RefIndex {
    by_commit: HashMap<String, Vec<RefLabel>>,
    branch_refs_by_commit: HashMap<String, Vec<CommitBranchRef>>,
    tags: Vec<RefLabel>,
}

fn read_labels(
    repository: &Repository,
    branches: &[BranchInfo],
    worktrees: &[WorktreeInfo],
    head_id: Option<&str>,
) -> Result<RefIndex> {
    let mut by_commit: HashMap<String, Vec<RefLabel>> = HashMap::new();
    let mut branch_refs: HashMap<String, BTreeMap<String, CommitBranchRef>> = HashMap::new();
    for branch in branches {
        if branch.target.is_empty() {
            continue;
        }
        by_commit
            .entry(branch.target.clone())
            .or_default()
            .push(RefLabel {
                name: branch.name.clone(),
                kind: if branch.remote {
                    RefKind::RemoteBranch
                } else {
                    RefKind::LocalBranch
                },
            });
        let (remote, short_name) = branch
            .remote
            .then(|| branch.name.split_once('/'))
            .flatten()
            .map_or((None, branch.name.as_str()), |(remote, short_name)| {
                (Some(remote), short_name)
            });
        // Remote symbolic HEAD aliases duplicate a real branch and are not a
        // branch presence of their own.
        if branch.remote && short_name == "HEAD" {
            continue;
        }
        let presence = branch_refs
            .entry(branch.target.clone())
            .or_default()
            .entry(short_name.to_owned())
            .or_insert_with(|| CommitBranchRef {
                branch_short_name: short_name.to_owned(),
                is_local: false,
                remote_names: Vec::new(),
                is_head: false,
                is_tag: false,
            });
        if let Some(remote) = remote {
            presence.remote_names.push(remote.to_owned());
        } else {
            presence.is_local = true;
            presence.is_head |= branch.current;
        }
    }
    for worktree in worktrees {
        if let Some(target) = &worktree.target {
            by_commit.entry(target.clone()).or_default().push(RefLabel {
                name: worktree.name.clone(),
                kind: RefKind::Worktree,
            });
        }
    }
    if let Some(id) = head_id {
        by_commit.entry(id.to_owned()).or_default().insert(
            0,
            RefLabel {
                name: "HEAD".to_owned(),
                kind: RefKind::Head,
            },
        );
    }
    let mut tags = Vec::new();
    for name in repository
        .tag_names(None)
        .context("enumerate tags")?
        .iter()
        .flatten()
    {
        let Ok(object) = repository.revparse_single(&format!("refs/tags/{name}")) else {
            continue;
        };
        let Ok(commit) = object.peel_to_commit() else {
            continue;
        };
        let commit_id = commit.id().to_string();
        let label = RefLabel {
            name: name.to_owned(),
            kind: RefKind::Tag,
        };
        by_commit
            .entry(commit_id.clone())
            .or_default()
            .push(label.clone());
        branch_refs
            .entry(commit_id)
            .or_default()
            .entry(name.to_owned())
            .or_insert_with(|| CommitBranchRef {
                branch_short_name: name.to_owned(),
                is_local: false,
                remote_names: Vec::new(),
                is_head: false,
                is_tag: true,
            });
        tags.push(label);
    }
    let branch_refs_by_commit = branch_refs
        .into_iter()
        .map(|(commit, refs)| (commit, refs.into_values().collect()))
        .collect();
    tags.sort_by(|left, right| right.name.cmp(&left.name));
    Ok(RefIndex {
        by_commit,
        branch_refs_by_commit,
        tags,
    })
}

fn read_stashes(repository: &mut Repository) -> Result<Vec<StashInfo>> {
    let mut stashes = Vec::new();
    repository
        .stash_foreach(|index, name, oid| {
            stashes.push(StashInfo {
                index,
                name: name.to_owned(),
                target: oid.to_string(),
            });
            true
        })
        .context("enumerate stashes")?;
    Ok(stashes)
}

fn read_worktrees(repository: &Repository) -> Result<Vec<WorktreeInfo>> {
    let mut worktrees = Vec::new();
    for name in repository
        .worktrees()
        .context("enumerate worktrees")?
        .iter()
        .flatten()
    {
        let Ok(worktree) = repository.find_worktree(name) else {
            continue;
        };
        let path = worktree.path().to_path_buf();
        let (branch, target, changes) = Repository::open(&path).map_or((None, None, 0), |repo| {
            let branch = repo
                .head()
                .ok()
                .and_then(|head| head.shorthand().map(str::to_owned));
            let target = repo
                .head()
                .ok()
                .and_then(|head| head.peel_to_commit().ok())
                .map(|commit| commit.id().to_string());
            let changes = read_status(&repo).map_or(0, |working| working.files.len());
            (branch, target, changes)
        });
        worktrees.push(WorktreeInfo {
            name: name.to_owned(),
            path,
            branch,
            target,
            changes,
        });
    }
    Ok(worktrees)
}

fn read_status(repository: &Repository) -> Result<WorkingTree> {
    let mut options = worktree_status_options();
    let statuses = repository
        .statuses(Some(&mut options))
        .context("read worktree status")?;
    let mut files = BTreeMap::<PathBuf, WorkingFile>::new();
    for entry in statuses.iter() {
        let path = entry
            .index_to_workdir()
            .and_then(|delta| delta.new_file().path().map(Path::to_path_buf))
            .or_else(|| {
                entry
                    .head_to_index()
                    .and_then(|delta| delta.new_file().path().map(Path::to_path_buf))
            })
            .or_else(|| entry.path().map(PathBuf::from));
        let Some(path) = path else {
            continue;
        };
        let status = entry.status();
        let file = files.entry(path.clone()).or_insert(WorkingFile {
            path,
            old_path: None,
            staged: None,
            unstaged: None,
        });
        file.staged = index_change(status).or(file.staged);
        file.unstaged = worktree_change(status).or(file.unstaged);
        let old_path = entry
            .index_to_workdir()
            .or_else(|| entry.head_to_index())
            .and_then(|delta| {
                let old = delta.old_file().path()?;
                let new = delta.new_file().path()?;
                (old != new && new == file.path).then(|| old.to_path_buf())
            });
        file.old_path = old_path.or_else(|| file.old_path.clone());
    }
    apply_renames(&mut files, rename_pairs(repository, true)?, true);
    apply_renames(&mut files, rename_pairs(repository, false)?, false);
    Ok(WorkingTree {
        files: files.into_values().collect(),
    })
}

fn worktree_status_options() -> StatusOptions {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true)
        .include_unmodified(false);
    options
}

fn expanded_paths(
    repository: &Repository,
    paths: &[PathBuf],
    staged: bool,
) -> Result<Vec<PathBuf>> {
    let requested = paths.iter().map(PathBuf::as_path).collect::<HashSet<_>>();
    let mut expanded = paths.to_vec();
    for (old, new) in rename_pairs(repository, staged)? {
        if requested.contains(new.as_path()) {
            expanded.push(old);
        }
    }
    expanded.sort();
    expanded.dedup();
    Ok(expanded)
}

fn rename_pairs(repository: &Repository, staged: bool) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut options = DiffOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .include_typechange(true);
    let mut diff = if staged {
        let head_tree = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_tree().ok());
        let index = repository.index().context("open repository index")?;
        repository
            .diff_tree_to_index(head_tree.as_ref(), Some(&index), Some(&mut options))
            .context("inspect staged renames")?
    } else {
        repository
            .diff_index_to_workdir(None, Some(&mut options))
            .context("inspect worktree renames")?
    };
    let mut find = DiffFindOptions::new();
    find.renames(true)
        .renames_from_rewrites(true)
        .for_untracked(true);
    diff.find_similar(Some(&mut find))
        .context("detect status renames")?;
    Ok(diff
        .deltas()
        .filter(|delta| delta.status() == Delta::Renamed)
        .filter_map(|delta| {
            Some((
                delta.old_file().path()?.to_path_buf(),
                delta.new_file().path()?.to_path_buf(),
            ))
        })
        .collect())
}

fn apply_renames(
    files: &mut BTreeMap<PathBuf, WorkingFile>,
    renames: Vec<(PathBuf, PathBuf)>,
    staged: bool,
) {
    for (old, new) in renames {
        let old_file = files.remove(&old);
        let new_file = files.remove(&new);
        let staged_change = new_file
            .as_ref()
            .and_then(|file| file.staged)
            .or_else(|| old_file.as_ref().and_then(|file| file.staged));
        let unstaged_change = new_file
            .as_ref()
            .and_then(|file| file.unstaged)
            .or_else(|| old_file.as_ref().and_then(|file| file.unstaged));
        files.insert(
            new.clone(),
            WorkingFile {
                path: new,
                old_path: Some(old),
                staged: if staged {
                    Some(ChangeKind::Renamed)
                } else {
                    staged_change
                },
                unstaged: if staged {
                    unstaged_change
                } else {
                    Some(ChangeKind::Renamed)
                },
            },
        );
    }
}

fn index_change(status: Status) -> Option<ChangeKind> {
    if status.contains(Status::CONFLICTED) {
        Some(ChangeKind::Conflicted)
    } else if status.contains(Status::INDEX_NEW) {
        Some(ChangeKind::Added)
    } else if status.contains(Status::INDEX_MODIFIED) {
        Some(ChangeKind::Modified)
    } else if status.contains(Status::INDEX_DELETED) {
        Some(ChangeKind::Deleted)
    } else if status.contains(Status::INDEX_RENAMED) {
        Some(ChangeKind::Renamed)
    } else if status.contains(Status::INDEX_TYPECHANGE) {
        Some(ChangeKind::TypeChanged)
    } else {
        None
    }
}

fn worktree_change(status: Status) -> Option<ChangeKind> {
    if status.contains(Status::CONFLICTED) {
        Some(ChangeKind::Conflicted)
    } else if status.contains(Status::WT_NEW) {
        Some(ChangeKind::Added)
    } else if status.contains(Status::WT_MODIFIED) {
        Some(ChangeKind::Modified)
    } else if status.contains(Status::WT_DELETED) {
        Some(ChangeKind::Deleted)
    } else if status.contains(Status::WT_RENAMED) {
        Some(ChangeKind::Renamed)
    } else if status.contains(Status::WT_TYPECHANGE) {
        Some(ChangeKind::TypeChanged)
    } else {
        None
    }
}

fn read_commits(
    repository: &Repository,
    limit: usize,
    labels: &HashMap<String, Vec<RefLabel>>,
    branch_refs: &HashMap<String, Vec<CommitBranchRef>>,
) -> Result<(Vec<CommitSummary>, bool)> {
    let mut walk = repository.revwalk().context("create commit walk")?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
        .context("sort commit walk")?;
    let mut pushed = HashSet::new();
    for reference in repository.references().context("enumerate walk roots")? {
        let Ok(reference) = reference else {
            continue;
        };
        if !reference.name().is_some_and(|name| {
            name.starts_with("refs/heads/")
                || name.starts_with("refs/remotes/")
                || name.starts_with("refs/tags/")
        }) {
            continue;
        }
        let Ok(commit) = reference.peel_to_commit() else {
            continue;
        };
        if pushed.insert(commit.id()) {
            let _ = walk.push(commit.id());
        }
    }
    if pushed.is_empty() {
        return Ok((Vec::new(), false));
    }
    let mut commits = Vec::with_capacity(limit.min(10_000));
    for result in walk.take(limit.saturating_add(1)) {
        let oid = result.context("walk commit")?;
        if commits.len() == limit {
            return Ok((commits, true));
        }
        let commit = repository.find_commit(oid).context("load walked commit")?;
        commits.push(CommitSummary {
            id: oid.to_string(),
            short_id: short_id(oid),
            subject: commit.summary().unwrap_or("(no commit message)").to_owned(),
            description: commit
                .body()
                .and_then(|body| body.lines().find(|line| !line.trim().is_empty()))
                .map(|line| line.trim().chars().take(160).collect())
                .unwrap_or_default(),
            author: commit.author().name().unwrap_or("Unknown").to_owned(),
            email: commit.author().email().unwrap_or_default().to_owned(),
            authored_seconds: commit.time().seconds(),
            parents: commit
                .parent_ids()
                .map(|parent| parent.to_string())
                .collect(),
            refs: labels.get(&oid.to_string()).cloned().unwrap_or_default(),
            branch_refs: branch_refs
                .get(&oid.to_string())
                .cloned()
                .unwrap_or_default(),
        });
    }
    Ok((commits, false))
}

fn file_changes(diff: &Diff<'_>) -> Result<Vec<FileChange>> {
    let mut files = Vec::with_capacity(diff.deltas().len());
    for (index, delta) in diff.deltas().enumerate() {
        let path = delta
            .new_file()
            .path()
            .or_else(|| delta.old_file().path())
            .map_or_else(|| PathBuf::from("(unknown)"), Path::to_path_buf);
        let old_path = delta
            .old_file()
            .path()
            .map(Path::to_path_buf)
            .filter(|old| old != &path);
        let (additions, deletions) = git2::Patch::from_diff(diff, index)
            .context("open file patch")?
            .and_then(|patch| patch.line_stats().ok())
            .map_or((0, 0), |(_, additions, deletions)| (additions, deletions));
        files.push(FileChange {
            path,
            old_path,
            kind: delta_kind(delta.status()),
            additions,
            deletions,
        });
    }
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn delta_kind(delta: Delta) -> ChangeKind {
    match delta {
        Delta::Added | Delta::Untracked => ChangeKind::Added,
        Delta::Deleted => ChangeKind::Deleted,
        Delta::Renamed | Delta::Copied => ChangeKind::Renamed,
        Delta::Typechange => ChangeKind::TypeChanged,
        Delta::Conflicted => ChangeKind::Conflicted,
        Delta::Unmodified | Delta::Ignored | Delta::Unreadable | Delta::Modified => {
            ChangeKind::Modified
        }
    }
}

fn file_content(
    repository: &Repository,
    workdir: &Path,
    request: &DiffRequest,
    encoding: &str,
) -> Result<Option<String>> {
    match &request.scope {
        DiffScope::Unstaged => match std::fs::read(workdir.join(&request.path)) {
            Ok(content) => Ok(text_content(&content, encoding)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error).with_context(|| format!("read {}", request.path.display())),
        },
        DiffScope::Staged => {
            let index = repository.index().context("open repository index")?;
            let Some(entry) = index.get_path(&request.path, 0) else {
                return Ok(None);
            };
            let blob = repository.find_blob(entry.id).context("load staged file")?;
            Ok(text_content(blob.content(), encoding))
        }
        DiffScope::Commit(id) => {
            let oid = Oid::from_str(id).with_context(|| format!("parse commit id {id}"))?;
            let commit = repository.find_commit(oid).context("find content commit")?;
            let tree = commit.tree().context("load content tree")?;
            let Ok(entry) = tree.get_path(&request.path) else {
                return Ok(None);
            };
            let Ok(blob) = repository.find_blob(entry.id()) else {
                return Ok(None);
            };
            Ok(text_content(blob.content(), encoding))
        }
    }
}

fn diff_side_content(
    repository: &Repository,
    workdir: &Path,
    request: &DiffRequest,
    encoding: &str,
) -> Result<(Option<String>, Option<String>)> {
    let new = file_content(repository, workdir, request, encoding)?;
    let old = match &request.scope {
        DiffScope::Unstaged => index_content(repository, &request.path)?
            .as_deref()
            .and_then(|content| text_content(content, encoding)),
        DiffScope::Staged => head_content(repository, &request.path)?
            .as_deref()
            .and_then(|content| text_content(content, encoding)),
        DiffScope::Commit(id) => {
            let oid = Oid::from_str(id).with_context(|| format!("parse commit id {id}"))?;
            let commit = repository.find_commit(oid).context("find diff commit")?;
            commit
                .parent(0)
                .ok()
                .and_then(|parent| parent.tree().ok())
                .and_then(|tree| {
                    tree.get_path(&request.path)
                        .ok()
                        .and_then(|entry| repository.find_blob(entry.id()).ok())
                        .and_then(|blob| text_content(blob.content(), encoding))
                })
        }
    };
    Ok((old, new))
}

fn full_diff_rows(old: &str, new: &str) -> (Vec<DiffRow>, Vec<usize>) {
    let old_lines = old
        .lines()
        .enumerate()
        .map(|(index, text)| (u32::try_from(index + 1).ok(), text.to_owned()))
        .collect::<Vec<_>>();
    let new_lines = new
        .lines()
        .enumerate()
        .map(|(index, text)| (u32::try_from(index + 1).ok(), text.to_owned()))
        .collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(old_lines.len().max(new_lines.len()));
    align_changed_lines(&mut rows, &old_lines, &new_lines);
    let hunks = rows
        .iter()
        .enumerate()
        .filter(|(index, row)| {
            row.kind != DiffRowKind::Context
                && (*index == 0 || rows[*index - 1].kind == DiffRowKind::Context)
        })
        .map(|(index, _)| index)
        .collect();
    (rows, hunks)
}

fn text_content(content: &[u8], encoding: &str) -> Option<String> {
    (!content.contains(&0)).then(|| {
        let decoded = if encoding.eq_ignore_ascii_case("Latin-1")
            || encoding.eq_ignore_ascii_case("ISO-8859-1")
        {
            content.iter().map(|byte| char::from(*byte)).collect()
        } else {
            String::from_utf8_lossy(content).into_owned()
        };
        expand_tabs(&decoded)
    })
}

/// Expands tabs to 4-column stops for display.
///
/// The diff canvas renders and measures text by column, so tabs must become
/// deterministic spaces before rows, intraline marks, or file content reach
/// the renderer. Mutation paths read raw bytes and are unaffected.
fn expand_tabs(text: &str) -> String {
    if !text.contains('\t') {
        return text.to_owned();
    }
    let mut expanded = String::with_capacity(text.len() + 16);
    let mut column = 0usize;
    for ch in text.chars() {
        match ch {
            '\t' => {
                let pad = 4 - column % 4;
                expanded.extend(std::iter::repeat_n(' ', pad));
                column += pad;
            }
            '\n' => {
                expanded.push('\n');
                column = 0;
            }
            _ => {
                expanded.push(ch);
                column += 1;
            }
        }
    }
    expanded
}

fn parse_diff(diff: &Diff<'_>, request: &DiffRequest) -> Result<DiffDocument> {
    let mut patch = None;
    let mut matched = false;
    let mut labels = ("a/".to_owned(), "b/".to_owned());
    for (index, delta) in diff.deltas().enumerate() {
        let path_matches = delta.new_file().path() == Some(request.path.as_path())
            || delta.old_file().path() == Some(request.path.as_path());
        if !path_matches {
            continue;
        }
        matched = true;
        labels = (
            delta.old_file().path().map_or_else(
                || "/dev/null".to_owned(),
                |path| format!("a/{}", path.display()),
            ),
            delta.new_file().path().map_or_else(
                || "/dev/null".to_owned(),
                |path| format!("b/{}", path.display()),
            ),
        );
        patch = git2::Patch::from_diff(diff, index).context("open selected file patch")?;
        break;
    }
    let Some(patch) = patch else {
        return Ok(DiffDocument {
            path: request.path.clone(),
            scope: request.scope.clone(),
            old_label: labels.0,
            new_label: labels.1,
            rows: Vec::new(),
            hunks: Vec::new(),
            binary: matched,
            content: None,
        });
    };
    let mut rows = Vec::new();
    let mut hunks = Vec::new();
    for hunk_index in 0..patch.num_hunks() {
        let (hunk, line_count) = patch.hunk(hunk_index).context("read diff hunk")?;
        hunks.push(rows.len());
        rows.push(DiffRow {
            old_number: None,
            new_number: None,
            old_text: String::new(),
            new_text: String::from_utf8_lossy(hunk.header()).trim_end().to_owned(),
            kind: DiffRowKind::Hunk,
            old_mark: None,
            new_mark: None,
        });
        let mut line_index = 0;
        while line_index < line_count {
            let line = patch
                .line_in_hunk(hunk_index, line_index)
                .context("read diff line")?;
            match line.origin() {
                '-' => {
                    let mut deleted = Vec::new();
                    while line_index < line_count {
                        let candidate = patch
                            .line_in_hunk(hunk_index, line_index)
                            .context("read deleted diff line")?;
                        if candidate.origin() != '-' {
                            break;
                        }
                        deleted.push(parsed_line(&candidate));
                        line_index += 1;
                    }
                    let mut added = Vec::new();
                    while line_index < line_count {
                        let candidate = patch
                            .line_in_hunk(hunk_index, line_index)
                            .context("read added diff line")?;
                        if candidate.origin() != '+' {
                            break;
                        }
                        added.push(parsed_line(&candidate));
                        line_index += 1;
                    }
                    align_changed_lines(&mut rows, &deleted, &added);
                }
                '+' => {
                    rows.push(DiffRow {
                        old_number: None,
                        new_number: line.new_lineno(),
                        old_text: String::new(),
                        new_text: line_text(&line),
                        kind: DiffRowKind::Added,
                        old_mark: None,
                        new_mark: None,
                    });
                    line_index += 1;
                }
                ' ' | '=' => {
                    let text = line_text(&line);
                    rows.push(DiffRow {
                        old_number: line.old_lineno(),
                        new_number: line.new_lineno(),
                        old_text: text.clone(),
                        new_text: text,
                        kind: DiffRowKind::Context,
                        old_mark: None,
                        new_mark: None,
                    });
                    line_index += 1;
                }
                _ => {
                    line_index += 1;
                }
            }
        }
    }
    Ok(DiffDocument {
        path: request.path.clone(),
        scope: request.scope.clone(),
        old_label: labels.0,
        new_label: labels.1,
        rows,
        content: None,
        hunks,
        binary: false,
    })
}

fn parsed_line(line: &git2::DiffLine<'_>) -> (Option<u32>, String) {
    (line.old_lineno().or(line.new_lineno()), line_text(line))
}

fn line_text(line: &git2::DiffLine<'_>) -> String {
    expand_tabs(String::from_utf8_lossy(line.content()).trim_end_matches(['\r', '\n']))
}

fn align_changed_lines(
    rows: &mut Vec<DiffRow>,
    deleted: &[(Option<u32>, String)],
    added: &[(Option<u32>, String)],
) {
    let old_lines = deleted
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>();
    let new_lines = added
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>();
    let diff = TextDiff::from_slices(&old_lines, &new_lines);
    let operations = diff.ops();
    let mut index = 0;
    while index < operations.len() {
        let (tag, old, new) = operations[index].as_tag_tuple();
        if matches!(tag, DiffTag::Delete | DiffTag::Insert)
            && let Some(next) = operations.get(index + 1)
        {
            let (next_tag, next_old, next_new) = next.as_tag_tuple();
            if matches!(
                (tag, next_tag),
                (DiffTag::Delete, DiffTag::Insert) | (DiffTag::Insert, DiffTag::Delete)
            ) {
                let old = if tag == DiffTag::Delete {
                    old
                } else {
                    next_old
                };
                let new = if tag == DiffTag::Insert {
                    new
                } else {
                    next_new
                };
                push_aligned_range(rows, deleted, added, old, new, false);
                index += 2;
                continue;
            }
        }
        push_aligned_range(rows, deleted, added, old, new, tag == DiffTag::Equal);
        index += 1;
    }
}

fn push_aligned_range(
    rows: &mut Vec<DiffRow>,
    deleted: &[(Option<u32>, String)],
    added: &[(Option<u32>, String)],
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
    equal: bool,
) {
    for offset in 0..old.len().max(new.len()) {
        push_aligned_row(
            rows,
            deleted
                .get(old.start + offset)
                .filter(|_| offset < old.len()),
            added.get(new.start + offset).filter(|_| offset < new.len()),
            equal,
        );
    }
}

fn push_aligned_row(
    rows: &mut Vec<DiffRow>,
    old: Option<&(Option<u32>, String)>,
    new: Option<&(Option<u32>, String)>,
    equal: bool,
) {
    let old_text = old.map(|(_, text)| text.as_str()).unwrap_or_default();
    let new_text = new.map(|(_, text)| text.as_str()).unwrap_or_default();
    let (old_mark, new_mark) = intraline_marks(old_text, new_text);
    let kind = match (old, new) {
        (Some(_), Some(_)) if equal => DiffRowKind::Context,
        (Some(_), Some(_)) => DiffRowKind::Changed,
        (Some(_), None) => DiffRowKind::Deleted,
        (None, Some(_)) => DiffRowKind::Added,
        (None, None) => return,
    };
    rows.push(DiffRow {
        old_number: old.and_then(|(number, _)| *number),
        new_number: new.and_then(|(number, _)| *number),
        old_text: old_text.to_owned(),
        new_text: new_text.to_owned(),
        kind,
        old_mark,
        new_mark,
    });
}

type IntralineMarks = (Option<(usize, usize)>, Option<(usize, usize)>);

fn intraline_marks(old: &str, new: &str) -> IntralineMarks {
    if old.is_empty() || new.is_empty() || old == new {
        return (None, None);
    }
    let prefix = old
        .chars()
        .zip(new.chars())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let old_tail = &old[prefix.min(old.len())..];
    let new_tail = &new[prefix.min(new.len())..];
    let suffix = old_tail
        .chars()
        .rev()
        .zip(new_tail.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum::<usize>();
    let old_end = old.len().saturating_sub(suffix).max(prefix);
    let new_end = new.len().saturating_sub(suffix).max(prefix);
    (
        (old_end > prefix).then_some((prefix, old_end)),
        (new_end > prefix).then_some((prefix, new_end)),
    )
}

fn parse_conflicts(message: &str) -> Vec<PathBuf> {
    let mut conflicts = Vec::new();
    let mut in_conflicts = false;
    for line in message.lines() {
        let trimmed = line.trim();
        if trimmed.eq_ignore_ascii_case("# conflicts:")
            || trimmed.eq_ignore_ascii_case("conflicts:")
        {
            in_conflicts = true;
            continue;
        }
        if in_conflicts {
            if let Some(path) = trimmed
                .strip_prefix('#')
                .or_else(|| trimmed.strip_prefix('-'))
            {
                let path = path.trim();
                if !path.is_empty() {
                    conflicts.push(PathBuf::from(path));
                }
            } else if !trimmed.is_empty() {
                break;
            }
        }
    }
    conflicts
}

fn is_reachable_from_remote(repository: &Repository, commit: Oid) -> bool {
    let Ok(branches) = repository.branches(Some(BranchType::Remote)) else {
        return false;
    };
    branches.flatten().any(|(branch, _)| {
        branch.get().target().is_some_and(|target| {
            target == commit
                || repository
                    .graph_descendant_of(target, commit)
                    .unwrap_or(false)
        })
    })
}
/// Safe (dirty-tolerant) worktree checkout that reports conflicts with git's
/// own wording, listing the conflicting paths like `git checkout`/`git merge`.
fn checkout_tree_reporting(
    repository: &Repository,
    target: &git2::Object<'_>,
    verb: &str,
) -> Result<()> {
    let conflicts = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let sink = std::rc::Rc::clone(&conflicts);
    let mut options = CheckoutBuilder::new();
    options.safe();
    options.notify_on(git2::CheckoutNotificationType::CONFLICT);
    options.notify(move |_, path, _, _, _| {
        if let Some(path) = path {
            sink.borrow_mut().push(path.display().to_string());
        }
        true
    });
    match repository.checkout_tree(target, Some(&mut options)) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == git2::ErrorCode::Conflict => {
            let mut listed = conflicts.borrow_mut();
            listed.sort();
            listed.dedup();
            let files = listed.join("\n\t");
            bail!(
                "Your local changes to the following files would be overwritten by {verb}:\n\t{files}\nPlease commit your changes or stash them before you switch branches."
            )
        }
        Err(error) => Err(error).context("checkout working tree"),
    }
}

fn checkout_local(repository: &Repository, branch: &git2::Branch<'_>, name: &str) -> Result<()> {
    let reference_name = branch
        .get()
        .name()
        .ok_or_else(|| anyhow!("branch reference is not UTF-8"))?
        .to_owned();
    let target = branch
        .get()
        .peel(ObjectType::Commit)
        .context("peel branch")?;
    // `git checkout <branch>` order: update the working tree against the
    // current HEAD baseline first (safe checkout carries non-conflicting
    // local edits, errors on real conflicts), then move HEAD.
    checkout_tree_reporting(repository, &target, "checkout")?;
    repository
        .set_head(&reference_name)
        .with_context(|| format!("set HEAD to {name}"))
}

fn resolve_branch_commit<'repo>(
    repository: &'repo Repository,
    branch: &str,
) -> Result<git2::Commit<'repo>> {
    for kind in [BranchType::Local, BranchType::Remote] {
        if let Ok(found) = repository.find_branch(branch, kind) {
            return found
                .get()
                .peel_to_commit()
                .with_context(|| format!("peel branch {branch}"));
        }
    }
    bail!("branch {branch} does not exist")
}

fn integrate_commit(
    repository: &Repository,
    target: &git2::Commit<'_>,
    message: &str,
) -> Result<()> {
    let annotated = repository
        .find_annotated_commit(target.id())
        .context("annotate merge target")?;
    let (analysis, _) = repository
        .merge_analysis(&[&annotated])
        .context("analyze merge")?;
    if analysis.contains(MergeAnalysis::ANALYSIS_UP_TO_DATE) {
        return Ok(());
    }
    if analysis.contains(MergeAnalysis::ANALYSIS_FASTFORWARD) {
        return fast_forward_to(repository, target.id(), message);
    }
    if !analysis.contains(MergeAnalysis::ANALYSIS_NORMAL) {
        bail!("libgit2 refused the requested integration");
    }
    repository
        .merge(&[&annotated], None, Some(CheckoutBuilder::new().safe()))
        .context("merge target branch")?;
    let mut index = repository.index().context("read merge index")?;
    if index.has_conflicts() {
        bail!("merge produced conflicts; resolve them before committing");
    }
    let tree_id = index.write_tree().context("write merge tree")?;
    let tree = repository.find_tree(tree_id).context("load merge tree")?;
    let head = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .context("load merge HEAD")?;
    let signature = repository.signature().unwrap_or_else(|_| {
        Signature::now("Kraken Native", "kraken@localhost")
            .expect("fallback signature has valid static fields")
    });
    repository
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&head, target],
        )
        .context("create merge commit")?;
    repository.cleanup_state().context("clean merge state")?;
    let head_tree = repository
        .head()
        .and_then(|head| head.peel(ObjectType::Commit))
        .context("load merged HEAD")?;
    checkout_tree_reporting(repository, &head_tree, "merge").context("synchronize merged worktree")
}

fn fast_forward_to(repository: &Repository, target: Oid, message: &str) -> Result<()> {
    let mut head = repository.head().context("read fast-forward HEAD")?;
    if !head.is_branch() {
        bail!("cannot fast-forward detached HEAD");
    }
    // Match `git merge --ff-only`: update the worktree first with a safe,
    // dirty-tolerant checkout (fails only on genuinely conflicting local
    // changes, leaving the ref untouched), then move the branch reference.
    let commit = repository
        .find_commit(target)
        .context("find fast-forward target")?;
    let tree = commit.tree().context("read fast-forward tree")?;
    checkout_tree_reporting(repository, tree.as_object(), "merge")
        .context("checkout fast-forward target")?;
    head.set_target(target, message)
        .context("move branch reference")?;
    repository
        .set_head(head.name().unwrap_or("HEAD"))
        .context("re-point HEAD")?;
    Ok(())
}

fn remote_callbacks(
    repository: &Repository,
    settings: &Settings,
) -> Result<RemoteCallbacks<'static>> {
    let config = repository.config().context("open Git credential config")?;
    let use_agent = settings.use_local_ssh_agent;
    let private_key = (!settings.ssh_private_key.trim().is_empty())
        .then(|| PathBuf::from(&settings.ssh_private_key));
    let public_key = (!settings.ssh_public_key.trim().is_empty())
        .then(|| PathBuf::from(&settings.ssh_public_key));
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |url, username, allowed| {
        if allowed.contains(git2::CredentialType::SSH_KEY) {
            let username = username.unwrap_or("git");
            if let Some(private_key) = private_key.as_deref() {
                return Cred::ssh_key(username, public_key.as_deref(), private_key, None);
            }
            if use_agent && let Ok(credential) = Cred::ssh_key_from_agent(username) {
                return Ok(credential);
            }
        }
        if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
            && let Ok(credential) = Cred::credential_helper(&config, url, username)
        {
            return Ok(credential);
        }
        Cred::default()
    });
    Ok(callbacks)
}

fn short_id(oid: Oid) -> String {
    oid.to_string().chars().take(7).collect()
}

fn tree_paths(tree: &git2::Tree<'_>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    tree.walk(TreeWalkMode::PreOrder, |directory, entry| {
        if entry
            .kind()
            .is_some_and(|kind| matches!(kind, ObjectType::Blob | ObjectType::Commit))
            && let Some(name) = entry.name()
        {
            paths.push(Path::new(directory).join(name));
        }
        TreeWalkResult::Ok
    })
    .context("walk commit tree")?;
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repository_with_commit() -> (tempfile::TempDir, Repository) {
        let directory = tempfile::tempdir().expect("temporary repository");
        let repository = Repository::init(directory.path()).expect("initialize repository");
        let mut config = repository.config().expect("open test config");
        config
            .set_str("user.name", "Kraken Test")
            .expect("set test user name");
        config
            .set_str("user.email", "test@kraken.local")
            .expect("set test email");
        drop(config);
        std::fs::write(
            directory.path().join("tracked.txt"),
            "alpha\nvalue = old\nomega\n",
        )
        .expect("write tracked file");
        let mut index = repository.index().expect("open index");
        index
            .add_path(Path::new("tracked.txt"))
            .expect("add tracked path");
        index.write().expect("persist initial index");
        let tree_id = index.write_tree().expect("write initial tree");
        let tree = repository.find_tree(tree_id).expect("load initial tree");
        let signature =
            Signature::now("Kraken Test", "test@kraken.local").expect("create test signature");
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "feat: created real history",
                &tree,
                &[],
            )
            .expect("create initial commit");
        drop(tree);
        drop(index);
        (directory, repository)
    }

    #[test]
    fn branch_order_is_case_insensitive_and_independent_of_head() {
        let mut branches = ["main", "feature/lane-3", "Feature/Detail", "feature/lane-1"]
            .into_iter()
            .map(|name| BranchInfo {
                name: name.to_owned(),
                target: String::new(),
                current: name == "main",
                remote: false,
                upstream: None,
            })
            .collect::<Vec<_>>();

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
    fn snapshot_reads_real_history_refs_and_worktree_status() {
        let (directory, repository) = repository_with_commit();
        let commit = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .expect("load initial commit");
        let current_branch = repository
            .head()
            .expect("read HEAD")
            .shorthand()
            .expect("HEAD branch name")
            .to_owned();
        for remote in ["origin", "fork"] {
            repository
                .reference(
                    &format!("refs/remotes/{remote}/{current_branch}"),
                    commit.id(),
                    true,
                    "create tracking reference",
                )
                .expect("create tracking reference");
        }
        repository
            .branch("feature/real-ref", &commit, false)
            .expect("create branch");
        repository
            .tag_lightweight("v0.1.0", commit.as_object(), false)
            .expect("create tag");
        drop(commit);
        let linked_root = tempfile::tempdir().expect("temporary linked worktree root");
        let linked_path = linked_root.path().join("linked");
        repository
            .worktree("linked", &linked_path, None)
            .expect("create linked worktree");
        std::fs::write(linked_path.join("tracked.txt"), "linked working change\n")
            .expect("modify linked worktree");
        std::fs::write(directory.path().join("tracked.txt"), "working change\n")
            .expect("modify tracked file");

        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let snapshot = backend.snapshot(100).expect("capture snapshot");
        assert_eq!(snapshot.commits.len(), 1);
        assert_eq!(snapshot.tags.len(), 1);
        assert!(
            snapshot
                .branches
                .iter()
                .any(|branch| branch.name == "feature/real-ref")
        );
        let main_ref = snapshot.commits[0]
            .branch_refs
            .iter()
            .find(|reference| reference.branch_short_name == current_branch)
            .expect("consolidated current branch ref");
        assert!(main_ref.is_local);
        assert!(main_ref.is_head);
        assert_eq!(main_ref.remote_names, ["fork", "origin"]);
        assert!(!main_ref.is_tag);
        assert_eq!(snapshot.working.unstaged_count(), 1);
        let detail = backend
            .commit_detail(&snapshot.commits[0].id)
            .expect("load commit detail");
        assert_eq!(detail.all_files, [PathBuf::from("tracked.txt")]);
        assert_eq!(snapshot.worktrees.len(), 1);
        assert_eq!(snapshot.worktrees[0].changes, 1);
        assert_eq!(snapshot.wip_rows(), 2);
        assert!(!snapshot.has_more);
    }

    #[test]
    fn stage_diff_commit_and_amend_use_real_index_and_objects() {
        let (directory, _repository) = repository_with_commit();
        std::fs::write(
            directory.path().join("tracked.txt"),
            "alpha\nvalue = new\nomega\n",
        )
        .expect("modify tracked file");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let unstaged_request = DiffRequest {
            path: PathBuf::from("tracked.txt"),
            scope: DiffScope::Unstaged,
        };
        let unstaged = backend.diff(&unstaged_request).expect("load unstaged diff");
        assert!(
            unstaged.rows.iter().any(|row| {
                row.kind == DiffRowKind::Changed && row.old_mark.is_some() && row.new_mark.is_some()
            }),
            "{:#?}",
            unstaged.rows
        );
        assert!(
            unstaged
                .content
                .as_deref()
                .is_some_and(|text| text.contains("value = new"))
        );

        backend
            .stage(&[PathBuf::from("tracked.txt")])
            .expect("stage tracked file");
        let staged_snapshot = backend.snapshot(100).expect("snapshot staged file");
        assert_eq!(staged_snapshot.working.staged_count(), 1);
        assert_eq!(staged_snapshot.working.unstaged_count(), 0);
        let staged = backend
            .diff(&DiffRequest {
                path: PathBuf::from("tracked.txt"),
                scope: DiffScope::Staged,
            })
            .expect("load staged diff");
        assert!(
            staged
                .rows
                .iter()
                .any(|row| row.kind == DiffRowKind::Changed)
        );

        backend
            .unstage(&[PathBuf::from("tracked.txt")])
            .expect("unstage tracked file");
        assert_eq!(
            backend
                .snapshot(100)
                .expect("snapshot unstaged file")
                .working
                .staged_count(),
            0
        );
        backend
            .stage(&[PathBuf::from("tracked.txt")])
            .expect("restage tracked file");
        let id = backend
            .commit(&CommitInput {
                summary: "fix(core): committed through backend".to_owned(),
                body: "Verified staged index content.".to_owned(),
                amend: false,
            })
            .expect("create commit");
        let committed = backend.snapshot(100).expect("snapshot committed state");
        assert_eq!(committed.head_id.as_deref(), Some(id.as_str()));
        assert!(committed.working.files.is_empty());
        assert_eq!(committed.commits.len(), 2);
        assert_eq!(
            backend
                .commit_detail(&id)
                .expect("load created commit")
                .body,
            "Verified staged index content."
        );
        let empty = backend
            .commit(&CommitInput {
                summary: "fix(core): rejected empty index".to_owned(),
                body: String::new(),
                amend: false,
            })
            .expect_err("reject commit without staged changes");
        assert!(empty.to_string().contains("no staged changes"));

        let amended = backend
            .commit(&CommitInput {
                summary: "fix(core): amended through backend".to_owned(),
                body: String::new(),
                amend: true,
            })
            .expect("amend without staged changes");
        assert_ne!(amended, id);
        let after_amend = backend.snapshot(100).expect("snapshot amended state");
        assert_eq!(after_amend.commits.len(), 2);
        assert_eq!(
            backend
                .commit_detail(&amended)
                .expect("load amended commit")
                .subject,
            "fix(core): amended through backend"
        );
    }

    #[test]
    fn reword_changes_only_the_message_and_keeps_staged_changes() {
        let (directory, repository) = repository_with_commit();
        std::fs::write(
            directory.path().join("tracked.txt"),
            "alpha\nvalue = staged\nomega\n",
        )
        .expect("modify tracked file");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend
            .stage(&[PathBuf::from("tracked.txt")])
            .expect("stage change");
        let before = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .expect("head before reword");
        let tree_before = before.tree_id();

        let id = backend
            .reword("feat: reworded subject", "New body.")
            .expect("reword head commit");

        let after = repository
            .find_commit(git2::Oid::from_str(&id).expect("parse reworded id"))
            .expect("load reworded commit");
        assert_eq!(
            after.message().unwrap_or_default(),
            "feat: reworded subject\n\nNew body."
        );
        assert_eq!(
            after.tree_id(),
            tree_before,
            "reword must not commit the staged change"
        );
        let snapshot = backend.snapshot(10).expect("snapshot after reword");
        assert_eq!(snapshot.working.staged_count(), 1);
    }

    #[test]
    fn save_patch_writes_format_patch_output() {
        let (directory, repository) = repository_with_commit();
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let head = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .expect("head commit")
            .id()
            .to_string();
        let destination = directory.path().join("head.patch");
        backend
            .save_patch(&head, &destination)
            .expect("save patch for head");
        let patch = std::fs::read_to_string(&destination).expect("read written patch");
        assert!(patch.starts_with("From "), "{patch}");
        assert!(patch.contains("Subject: [PATCH] feat: created real history"));
        assert!(patch.contains("+value = old"));
    }

    #[test]
    fn stage_single_added_line_leaves_other_lines_unstaged() {
        let (directory, _repository) = repository_with_commit();
        let path = PathBuf::from("tracked.txt");
        std::fs::write(
            directory.path().join(&path),
            "alpha\nfirst addition\nvalue = old\nsecond addition\nomega\n",
        )
        .expect("write two added lines");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let diff = backend
            .diff(&DiffRequest {
                path: path.clone(),
                scope: DiffScope::Unstaged,
            })
            .expect("load unstaged diff");
        let first = diff
            .rows
            .iter()
            .find(|row| row.new_text == "first addition")
            .expect("first added row");
        backend
            .stage_lines(
                &path,
                &[DiffLineSelection {
                    old_line: first.old_number,
                    new_line: first.new_number,
                }],
            )
            .expect("stage one line");
        let repository = backend.open().expect("open repository");
        assert_eq!(
            String::from_utf8(
                index_content(&repository, &path)
                    .expect("read index")
                    .expect("index file")
            )
            .expect("utf8 index"),
            "alpha\nfirst addition\nvalue = old\nomega\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join(&path)).expect("read worktree"),
            "alpha\nfirst addition\nvalue = old\nsecond addition\nomega\n"
        );
        let snapshot = backend.snapshot(100).expect("snapshot split file");
        let file = snapshot
            .working
            .files
            .iter()
            .find(|file| file.path == path)
            .expect("split file status");
        assert!(file.staged.is_some() && file.unstaged.is_some());
    }

    #[test]
    fn discard_selected_range_restores_only_selected_changes() {
        let (directory, _repository) = repository_with_commit();
        let path = PathBuf::from("tracked.txt");
        std::fs::write(
            directory.path().join(&path),
            "alpha\nvalue = new\nadded value\nomega\n",
        )
        .expect("write changed range");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let diff = backend
            .diff(&DiffRequest {
                path: path.clone(),
                scope: DiffScope::Unstaged,
            })
            .expect("load changed range");
        let selection = diff
            .rows
            .iter()
            .filter(|row| matches!(row.kind, DiffRowKind::Changed | DiffRowKind::Added))
            .map(|row| DiffLineSelection {
                old_line: row.old_number,
                new_line: row.new_number,
            })
            .collect::<Vec<_>>();
        backend
            .discard_lines(&path, &selection)
            .expect("discard selected range");
        assert_eq!(
            std::fs::read_to_string(directory.path().join(&path)).expect("read restored file"),
            "alpha\nvalue = old\nomega\n"
        );
    }

    #[test]
    fn stash_file_stashes_only_the_selected_path() {
        let (directory, mut repository) = repository_with_commit();
        std::fs::write(directory.path().join("tracked.txt"), "stashed change\n")
            .expect("modify tracked file");
        std::fs::write(directory.path().join("kept.txt"), "keep me\n")
            .expect("write untouched file");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend
            .stash_file(Path::new("tracked.txt"))
            .expect("stash single file");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("tracked.txt"))
                .expect("read restored file"),
            "alpha\nvalue = old\nomega\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("kept.txt")).expect("read kept file"),
            "keep me\n"
        );
        let mut stashes = 0;
        repository
            .stash_foreach(|_, _, _| {
                stashes += 1;
                true
            })
            .expect("iterate stashes");
        assert_eq!(stashes, 1);
    }

    #[test]
    fn append_ignore_appends_each_pattern_once() {
        let (directory, _repository) = repository_with_commit();
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend.append_ignore("target/").expect("append pattern");
        backend
            .append_ignore("*.log")
            .expect("append second pattern");
        backend.append_ignore("target/").expect("skip duplicate");
        assert_eq!(
            std::fs::read_to_string(directory.path().join(".gitignore")).expect("read .gitignore"),
            "target/\n*.log\n"
        );
        assert!(backend.append_ignore("   ").is_err());
    }

    #[test]
    fn discard_file_restores_index_content_and_removes_untracked() {
        let (directory, _repository) = repository_with_commit();
        std::fs::write(directory.path().join("tracked.txt"), "dirty\n")
            .expect("modify tracked file");
        std::fs::write(directory.path().join("untracked.txt"), "new\n")
            .expect("write untracked file");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend
            .discard_file(Path::new("tracked.txt"))
            .expect("discard tracked change");
        backend
            .discard_file(Path::new("untracked.txt"))
            .expect("discard untracked file");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("tracked.txt"))
                .expect("read restored file"),
            "alpha\nvalue = old\nomega\n"
        );
        assert!(!directory.path().join("untracked.txt").exists());
    }

    #[test]
    fn unstage_single_line_preserves_other_staged_lines() {
        let (directory, _repository) = repository_with_commit();
        let path = PathBuf::from("tracked.txt");
        std::fs::write(
            directory.path().join(&path),
            "alpha\nfirst addition\nvalue = old\nsecond addition\nomega\n",
        )
        .expect("write two added lines");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let unstaged = backend
            .diff(&DiffRequest {
                path: path.clone(),
                scope: DiffScope::Unstaged,
            })
            .expect("load unstaged diff");
        let first = unstaged
            .rows
            .iter()
            .find(|row| row.new_text == "first addition")
            .expect("first added row");
        backend
            .stage_lines(
                &path,
                &[DiffLineSelection {
                    old_line: first.old_number,
                    new_line: first.new_number,
                }],
            )
            .expect("stage first line");
        let staged = backend
            .diff(&DiffRequest {
                path: path.clone(),
                scope: DiffScope::Staged,
            })
            .expect("load staged diff");
        let first = staged
            .rows
            .iter()
            .find(|row| row.new_text == "first addition")
            .expect("first staged row");
        backend
            .unstage_lines(
                &path,
                &[DiffLineSelection {
                    old_line: first.old_number,
                    new_line: first.new_number,
                }],
            )
            .expect("unstage first line");
        let repository = backend.open().expect("open repository");
        assert_eq!(
            String::from_utf8(
                index_content(&repository, &path)
                    .expect("read index")
                    .expect("index file")
            )
            .expect("utf8 index"),
            "alpha\nvalue = old\nomega\n"
        );
    }

    #[test]
    fn full_diff_rows_keep_context_outside_patch_hunks() {
        let old = (1..=100)
            .map(|line| format!("content line {line:03}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut new = old.clone();
        new = new.replacen("content line 050", "content line 050 changed", 1);

        let (rows, hunks) = full_diff_rows(&old, &new);

        assert_eq!(rows.len(), 100);
        assert_eq!(rows[0].old_number, Some(1));
        assert_eq!(rows[99].new_number, Some(100));
        assert_eq!(rows[49].kind, DiffRowKind::Changed);
        assert_eq!(hunks, vec![49]);
    }

    #[test]
    fn expand_tabs_uses_four_column_stops_for_display() {
        assert_eq!(expand_tabs("\t\tconst x = 1;"), "        const x = 1;");
        assert_eq!(expand_tabs("ab\tcd"), "ab  cd");
        assert_eq!(expand_tabs("abcd\tx"), "abcd    x");
        assert_eq!(expand_tabs("a\nb\tc"), "a\nb   c");
        assert_eq!(expand_tabs("no tabs"), "no tabs");
        let content = text_content(b"\tindented\n", "UTF-8").expect("textual content");
        assert_eq!(content, "    indented\n");
    }

    #[test]
    fn stage_and_unstage_rename_update_both_index_paths() {
        let (directory, _repository) = repository_with_commit();
        std::fs::rename(
            directory.path().join("tracked.txt"),
            directory.path().join("renamed.txt"),
        )
        .expect("rename tracked file");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let renamed = backend.snapshot(100).expect("snapshot working rename");
        assert_eq!(
            renamed.working.files.len(),
            1,
            "{:#?}",
            renamed.working.files
        );
        assert_eq!(renamed.working.files[0].path, PathBuf::from("renamed.txt"));
        assert_eq!(
            renamed.working.files[0].old_path,
            Some(PathBuf::from("tracked.txt"))
        );

        backend
            .stage(&[PathBuf::from("renamed.txt")])
            .expect("stage rename");
        let staged = backend.snapshot(100).expect("snapshot staged rename");
        assert_eq!(staged.working.files.len(), 1);
        assert_eq!(staged.working.files[0].staged, Some(ChangeKind::Renamed));
        assert!(staged.working.files[0].unstaged.is_none());

        backend
            .unstage(&[PathBuf::from("renamed.txt")])
            .expect("unstage rename");
        let unstaged = backend.snapshot(100).expect("snapshot unstaged rename");
        assert_eq!(unstaged.working.files.len(), 1);
        assert_eq!(
            unstaged.working.files[0].unstaged,
            Some(ChangeKind::Renamed)
        );
        assert!(unstaged.working.files[0].staged.is_none());
    }

    fn commit_file(repository: &Repository, path: &str, content: &str, message: &str) -> Oid {
        let repository = Repository::open(repository.workdir().expect("repository worktree"))
            .expect("reopen repository for commit");
        let workdir = repository.workdir().expect("repository worktree");
        std::fs::write(workdir.join(path), content).expect("write commit file");
        let mut index = repository.index().expect("open commit index");
        index.add_path(Path::new(path)).expect("stage commit file");
        index.write().expect("persist commit index");
        let tree_id = index.write_tree().expect("write commit tree");
        let tree = repository.find_tree(tree_id).expect("load commit tree");
        let signature = repository.signature().expect("configured signature");
        let parents = repository
            .head()
            .ok()
            .and_then(|head| head.peel_to_commit().ok())
            .into_iter()
            .collect::<Vec<_>>();
        let parent_refs = parents.iter().collect::<Vec<_>>();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                message,
                &tree,
                &parent_refs,
            )
            .expect("create file commit")
    }

    fn configure_identity(repository: &Repository) {
        let mut config = repository.config().expect("open repository config");
        config
            .set_str("user.name", "Kraken Test")
            .expect("set repository user");
        config
            .set_str("user.email", "test@kraken.local")
            .expect("set repository email");
    }

    #[test]
    fn branch_checkout_stash_and_pop_mutate_the_real_worktree() {
        let (directory, _repository) = repository_with_commit();
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let original = backend.snapshot(100).expect("initial snapshot").head;
        backend
            .create_branch("feature/operations", None)
            .expect("create and checkout branch");
        assert_eq!(
            backend.snapshot(100).expect("branch snapshot").head,
            "feature/operations"
        );
        backend
            .checkout(&original)
            .expect("checkout original branch");
        assert_eq!(
            backend.snapshot(100).expect("checkout snapshot").head,
            original
        );

        std::fs::write(directory.path().join("tracked.txt"), "stashed work\n")
            .expect("modify tracked file");
        std::fs::write(directory.path().join("untracked.txt"), "untracked stash\n")
            .expect("write untracked file");
        backend.stash().expect("stash tracked and untracked files");
        let stashed = backend.snapshot(100).expect("stashed snapshot");
        assert!(stashed.working.files.is_empty());
        assert_eq!(stashed.stashes.len(), 1);
        backend.pop_stash().expect("pop newest stash");
        let restored = backend.snapshot(100).expect("restored snapshot");
        assert_eq!(restored.stashes.len(), 0);
        assert_eq!(restored.working.files.len(), 2);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("tracked.txt"))
                .expect("read restored file"),
            "stashed work\n"
        );
    }

    #[test]
    fn create_branch_switches_at_head_without_touching_dirty_worktree() {
        let (directory, repository) = repository_with_commit();
        let workdir = directory.path();
        std::fs::write(workdir.join("tracked.txt"), "staged change\n")
            .expect("write staged change");
        let mut index = repository.index().expect("open index");
        index
            .add_path(Path::new("tracked.txt"))
            .expect("stage change");
        index.write().expect("write index");
        std::fs::write(workdir.join("tracked.txt"), "unstaged change\n")
            .expect("write unstaged change");

        let backend = GitBackend::discover(workdir).expect("discover repository");
        backend
            .create_branch("feature/dirty-switch", None)
            .expect("create and switch without checkout");

        let snapshot = backend.snapshot(100).expect("snapshot dirty branch");
        assert_eq!(snapshot.head, "feature/dirty-switch");
        let change = snapshot
            .working
            .files
            .iter()
            .find(|file| file.path == Path::new("tracked.txt"))
            .expect("tracked dirty file");
        assert_eq!(change.staged, Some(ChangeKind::Modified));
        assert_eq!(change.unstaged, Some(ChangeKind::Modified));
        assert_eq!(
            std::fs::read_to_string(workdir.join("tracked.txt")).expect("read working file"),
            "unstaged change\n"
        );
    }

    #[test]
    fn fast_forward_merge_and_rebase_update_commit_topology() {
        let (directory, repository) = repository_with_commit();
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let main = current_branch(&repository).expect("current branch");

        backend
            .create_branch("feature/forward", None)
            .expect("create forward branch");
        let forward = commit_file(
            &repository,
            "forward.txt",
            "forward\n",
            "feat: advanced forward branch",
        );
        backend.checkout(&main).expect("return to main");
        backend
            .fast_forward("feature/forward")
            .expect("fast-forward main");
        assert_eq!(
            repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .expect("fast-forward head")
                .id(),
            forward
        );

        backend
            .create_branch("feature/merge", None)
            .expect("create merge branch");
        commit_file(
            &repository,
            "merge.txt",
            "merge branch\n",
            "feat: added merge branch work",
        );
        backend.checkout(&main).expect("return before divergence");
        commit_file(
            &repository,
            "main.txt",
            "main branch\n",
            "feat: added main branch work",
        );
        backend
            .merge("feature/merge")
            .expect("merge divergent branch");
        let merge = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .expect("load merge commit");
        assert_eq!(merge.parent_count(), 2);

        backend
            .create_branch("feature/rebase", None)
            .expect("create rebase branch");
        commit_file(
            &repository,
            "rebase.txt",
            "topic\n",
            "feat: added rebase topic",
        );
        backend
            .checkout(&main)
            .expect("checkout main for rebase base");
        let base = commit_file(
            &repository,
            "base.txt",
            "base advanced\n",
            "feat: advanced rebase base",
        );
        backend
            .checkout("feature/rebase")
            .expect("checkout rebase branch");
        let before_rebase = backend.snapshot(100).expect("snapshot before rebase");
        assert!(
            before_rebase.working.files.is_empty(),
            "{:#?}",
            before_rebase.working.files
        );
        backend.rebase(&main).expect("rebase topic onto main");
        let rebased = repository
            .head()
            .and_then(|head| head.peel_to_commit())
            .expect("load rebased commit");
        assert_eq!(rebased.parent_id(0).expect("rebased parent"), base);
        assert_eq!(rebased.summary(), Some("feat: added rebase topic"));
    }

    #[test]
    fn local_remote_fetch_push_and_pull_round_trip() {
        let (directory, repository) = repository_with_commit();
        let remote_directory = tempfile::tempdir().expect("temporary bare remote");
        Repository::init_bare(remote_directory.path()).expect("initialize bare remote");
        repository
            .remote(
                "origin",
                remote_directory.path().to_str().expect("remote UTF-8"),
            )
            .expect("add origin remote");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let branch_name = current_branch(&repository).expect("current branch");

        backend.push().expect("push initial branch");
        let branch = repository
            .find_branch(&branch_name, BranchType::Local)
            .expect("find pushed branch");
        assert_eq!(
            branch
                .upstream()
                .expect("first push records upstream")
                .name()
                .expect("read upstream")
                .expect("upstream UTF-8"),
            format!("origin/{branch_name}")
        );

        let clone_directory = tempfile::tempdir().expect("temporary second worktree");
        let clone = Repository::clone(
            remote_directory.path().to_str().expect("clone URL UTF-8"),
            clone_directory.path(),
        )
        .expect("clone bare remote");
        configure_identity(&clone);
        let remote_commit = commit_file(
            &clone,
            "remote.txt",
            "remote update\n",
            "feat: added remote update",
        );
        let clone_backend = GitBackend::discover(clone_directory.path()).expect("discover clone");
        clone_backend.push().expect("push remote update");

        backend.fetch(false).expect("fetch updated remote");
        let fetched = repository
            .find_reference(&format!("refs/remotes/origin/{branch_name}"))
            .expect("find fetched tracking reference")
            .target()
            .expect("tracking target");
        assert_eq!(fetched, remote_commit);
        backend
            .pull(PullOperation::FastForward)
            .expect("pull remote update");
        assert_eq!(
            repository
                .head()
                .and_then(|head| head.peel_to_commit())
                .expect("pulled head")
                .id(),
            remote_commit
        );

        let local_commit = commit_file(
            &repository,
            "local.txt",
            "local update\n",
            "feat: added local update",
        );
        backend.push().expect("push local update");
        let remote = Repository::open_bare(remote_directory.path()).expect("open bare remote");
        assert_eq!(
            remote
                .find_reference(&format!("refs/heads/{branch_name}"))
                .expect("find pushed remote branch")
                .target(),
            Some(local_commit)
        );
    }

    #[test]
    fn create_tag_supports_lightweight_and_annotated_targets() {
        let (directory, repository) = repository_with_commit();
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        let head = repository
            .head()
            .expect("head")
            .target()
            .expect("head target");
        backend
            .create_tag("v1-light", "HEAD", None)
            .expect("lightweight tag");
        backend
            .create_tag("v1-note", "HEAD", Some("release notes"))
            .expect("annotated tag");
        let repository = Repository::open(directory.path()).expect("reopen repository");
        assert_eq!(
            repository
                .refname_to_id("refs/tags/v1-light")
                .expect("light ref"),
            head
        );
        let annotated = repository
            .find_reference("refs/tags/v1-note")
            .expect("annotated ref");
        assert_eq!(
            annotated.peel_to_commit().expect("annotated target").id(),
            head
        );
        assert_eq!(
            repository
                .find_tag(annotated.target().expect("tag object"))
                .expect("tag")
                .message(),
            Some("release notes")
        );
    }

    #[test]
    fn cherry_pick_and_revert_create_expected_commits() {
        let (directory, repository) = repository_with_commit();
        let original = repository
            .head()
            .expect("head")
            .target()
            .expect("head target");
        let feature = repository
            .branch(
                "feature/pick",
                &repository.find_commit(original).expect("head commit"),
                false,
            )
            .expect("feature branch");
        checkout_local(&repository, &feature, "feature/pick").expect("checkout feature");
        let picked = commit_file(&repository, "picked.txt", "picked\n", "feat: picked");
        let main = repository
            .find_branch("master", BranchType::Local)
            .or_else(|_| repository.find_branch("main", BranchType::Local))
            .expect("default branch");
        checkout_local(
            &repository,
            &main,
            main.name().expect("branch name").expect("UTF-8"),
        )
        .expect("checkout default");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend
            .cherry_pick(&picked.to_string())
            .expect("cherry pick");
        let cherry = Repository::open(directory.path())
            .expect("reopen")
            .head()
            .expect("head")
            .target()
            .expect("cherry commit");
        assert_ne!(cherry, picked);
        backend.revert(&cherry.to_string()).expect("revert");
        let repository = Repository::open(directory.path()).expect("reopen");
        let reverted = repository
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("revert commit");
        assert_eq!(reverted.parent_id(0).expect("revert parent"), cherry);
        assert!(
            reverted
                .message()
                .expect("revert message")
                .contains("Revert")
        );
    }

    #[test]
    fn reset_modes_preserve_their_index_and_worktree_contracts() {
        let (directory, repository) = repository_with_commit();
        let base = repository.head().expect("head").target().expect("base");
        let commit = commit_file(&repository, "tracked.txt", "committed\n", "feat: second");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend
            .reset(&base.to_string(), "soft")
            .expect("soft reset");
        let soft = backend.snapshot(100).expect("soft snapshot");
        assert_eq!(soft.head_id.as_deref(), Some(base.to_string().as_str()));
        assert_eq!(soft.working.staged_count(), 1);
        backend
            .reset(&commit.to_string(), "hard")
            .expect("restore commit");
        std::fs::write(directory.path().join("tracked.txt"), "mixed\n")
            .expect("write mixed change");
        backend
            .stage(&[PathBuf::from("tracked.txt")])
            .expect("stage mixed change");
        backend
            .reset(&base.to_string(), "mixed")
            .expect("mixed reset");
        let mixed = backend.snapshot(100).expect("mixed snapshot");
        assert_eq!(mixed.working.staged_count(), 0);
        assert_eq!(mixed.working.unstaged_count(), 1);
        backend
            .reset(&base.to_string(), "hard")
            .expect("hard reset");
        assert!(
            backend
                .snapshot(100)
                .expect("hard snapshot")
                .working
                .files
                .is_empty()
        );
    }

    #[test]
    fn gitflow_init_writes_config_and_creates_develop_branch() {
        let (directory, _) = repository_with_commit();
        let mut settings = Settings::default();
        settings.gitflow_main_branch = "mainline".to_owned();
        settings.gitflow_develop_branch = "develop".to_owned();
        let backend =
            GitBackend::discover_with_settings(directory.path(), settings).expect("discover");
        backend.init_gitflow().expect("initialize Gitflow");
        let repository = Repository::open(directory.path()).expect("reopen");
        assert_eq!(
            repository
                .config()
                .expect("config")
                .get_string("gitflow.branch.master")
                .expect("main config"),
            "mainline"
        );
        assert!(repository.find_branch("develop", BranchType::Local).is_ok());
    }

    #[test]
    fn sparse_checkout_disable_is_an_honest_git_operation() {
        let (directory, _) = repository_with_commit();
        let backend = GitBackend::discover(directory.path()).expect("discover");
        backend
            .sparse_checkout(None)
            .expect("disable sparse checkout");
    }

    #[test]
    fn branch_delete_and_rename_mutate_real_refs() {
        let (directory, repository) = repository_with_commit();
        let head = repository
            .head()
            .expect("head")
            .peel_to_commit()
            .expect("head commit");
        repository
            .branch("feature/old", &head, false)
            .expect("create feature");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend
            .rename_branch("feature/old", "feature/new")
            .expect("rename branch");
        assert!(
            Repository::open(directory.path())
                .expect("reopen")
                .find_branch("feature/new", BranchType::Local)
                .is_ok()
        );
        backend.delete_branch("feature/new").expect("delete branch");
        assert!(
            Repository::open(directory.path())
                .expect("reopen")
                .find_branch("feature/new", BranchType::Local)
                .is_err()
        );
    }

    #[test]
    fn apply_stash_preserves_entry_while_drop_removes_it() {
        let (directory, _repository) = repository_with_commit();
        std::fs::write(directory.path().join("tracked.txt"), "stashed\n")
            .expect("write stash change");
        let backend = GitBackend::discover(directory.path()).expect("discover repository");
        backend.stash().expect("stash change");
        backend.apply_stash(0).expect("apply stash");
        assert_eq!(
            backend
                .snapshot(100)
                .expect("applied snapshot")
                .stashes
                .len(),
            1
        );
        backend.drop_stash(0).expect("drop stash");
        assert!(
            backend
                .snapshot(100)
                .expect("dropped snapshot")
                .stashes
                .is_empty()
        );
    }
}
