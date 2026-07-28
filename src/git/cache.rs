//! Cross-session caches backed by LMDB.
//!
//! Opening a repository is dominated by the first snapshot round trip. These
//! stores keep the last delivered [`RepoSnapshot`] per repository, plus
//! fetched author avatars, so the next launch can paint the graph and sidebar
//! immediately while the authoritative snapshot is computed behind it. Entries
//! are provisional by construction: nothing here is trusted for mutations or
//! detail loads, only for paint.
//!
//! LMDB is a good fit for the access pattern — one small keyed read at
//! startup, one keyed write per repository state change, no rewrite of
//! unrelated repositories, and safe concurrent access from several Kraken
//! processes sharing the cache directory.

use std::path::Path;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use heed::{
    Database, Env, EnvOpenOptions,
    types::{Bytes, SerdeBincode, Str},
};

use crate::git::models::RepoSnapshot;

/// Snapshot table name; bump the suffix whenever [`RepoSnapshot`] changes
/// shape so entries written by an older build simply miss instead of
/// mis-decoding.
const SNAPSHOT_DATABASE: &str = "snapshots-v2";

/// Avatar table name, keyed by the stable avatar key the graph requests.
const AVATAR_DATABASE: &str = "avatars-v1";

/// Upper bound on the snapshot memory map. Snapshots run one to a few MiB
/// each, so this holds a large working set of repositories without resizing.
const SNAPSHOT_MAP_SIZE: usize = 256 * 1024 * 1024;

/// Upper bound on the avatar memory map; avatars are a few KiB apiece.
const AVATAR_MAP_SIZE: usize = 64 * 1024 * 1024;

/// Keyed store of provisional repository snapshots.
pub(crate) struct SnapshotCache {
    env: Env,
    db: Database<Str, SerdeBincode<RepoSnapshot>>,
}

impl SnapshotCache {
    /// Opens the cache in the platform cache directory, or returns `None`
    /// when it is unavailable. A missing cache only costs the instant paint.
    pub(crate) fn platform() -> Option<Self> {
        let directory = ProjectDirs::from("ac", "Kraken Native", "Kraken Native")?
            .cache_dir()
            .join("snapshots");
        Self::at(&directory).ok()
    }

    /// Opens (creating if needed) the LMDB environment rooted at `directory`.
    pub(crate) fn at(directory: &Path) -> Result<Self> {
        let env = open_env(directory, SNAPSHOT_MAP_SIZE)?;
        let mut transaction = env.write_txn().context("begin cache setup")?;
        let db = env
            .create_database(&mut transaction, Some(SNAPSHOT_DATABASE))
            .context("open snapshot table")?;
        transaction.commit().context("commit cache setup")?;
        Ok(Self { env, db })
    }

    /// Reads the snapshot stored for `repo`, if any. Decode failures are
    /// treated as a miss so a stale or corrupt entry can never block startup.
    pub(crate) fn load(&self, repo: &Path) -> Option<RepoSnapshot> {
        let key = repo.to_str()?;
        let transaction = self.env.read_txn().ok()?;
        self.db.get(&transaction, key).ok().flatten()
    }

    /// Replaces the snapshot stored for `repo`.
    pub(crate) fn store(&self, repo: &Path, snapshot: &RepoSnapshot) -> Result<()> {
        let key = repo
            .to_str()
            .context("repository path is not valid UTF-8")?;
        let mut transaction = self.env.write_txn().context("begin cache write")?;
        self.db
            .put(&mut transaction, key, snapshot)
            .context("write cached snapshot")?;
        transaction.commit().context("commit cached snapshot")
    }
}

/// Opens (creating if needed) a single-table LMDB environment.
fn open_env(directory: &Path, map_size: usize) -> Result<Env> {
    std::fs::create_dir_all(directory)
        .with_context(|| format!("create cache directory {}", directory.display()))?;
    // SAFETY: LMDB memory-maps the database file, so the process would
    // observe torn reads if something outside LMDB's locking protocol
    // truncated or rewrote it. These directories are created and written
    // exclusively through this module, and concurrent Kraken processes
    // coordinate through LMDB's own lock file.
    #[allow(unsafe_code)]
    let env = unsafe {
        EnvOpenOptions::new()
            .map_size(map_size)
            .max_dbs(1)
            .open(directory)
    }
    .with_context(|| format!("open cache {}", directory.display()))?;
    Ok(env)
}

/// Keyed store of fetched author avatars, shared across sessions.
///
/// Images are immutable for a given key, so entries never need invalidation;
/// a miss costs one network round trip on a background thread.
pub(crate) struct AvatarCache {
    env: Env,
    db: Database<Str, Bytes>,
}

impl AvatarCache {
    /// Opens the cache in the platform cache directory, or returns `None`
    /// when it is unavailable; avatars then refetch every session.
    pub(crate) fn platform() -> Option<Self> {
        let directory = ProjectDirs::from("ac", "Kraken Native", "Kraken Native")?
            .cache_dir()
            .join("avatars");
        Self::at(&directory).ok()
    }

    /// Opens (creating if needed) the LMDB environment rooted at `directory`.
    pub(crate) fn at(directory: &Path) -> Result<Self> {
        let env = open_env(directory, AVATAR_MAP_SIZE)?;
        let mut transaction = env.write_txn().context("begin avatar cache setup")?;
        let db = env
            .create_database(&mut transaction, Some(AVATAR_DATABASE))
            .context("open avatar table")?;
        transaction.commit().context("commit avatar cache setup")?;
        Ok(Self { env, db })
    }

    /// Reads the encoded image stored for `key`, if any.
    pub(crate) fn load(&self, key: &str) -> Option<Vec<u8>> {
        let transaction = self.env.read_txn().ok()?;
        self.db
            .get(&transaction, key)
            .ok()
            .flatten()
            .map(<[u8]>::to_vec)
    }

    /// Replaces the encoded image stored for `key`.
    pub(crate) fn store(&self, key: &str, image: &[u8]) -> Result<()> {
        let mut transaction = self.env.write_txn().context("begin avatar write")?;
        self.db
            .put(&mut transaction, key, image)
            .context("write cached avatar")?;
        transaction.commit().context("commit cached avatar")
    }
}

#[cfg(test)]
mod tests {
    use super::{AvatarCache, SnapshotCache};
    use crate::git::models::{RepoSnapshot, WorkingTree};
    use std::path::PathBuf;

    fn snapshot(head: &str) -> RepoSnapshot {
        RepoSnapshot {
            path: PathBuf::from("/tmp/example"),
            name: "example".to_owned(),
            head: head.to_owned(),
            head_id: None,
            branches: Vec::new(),
            tags: Vec::new(),
            stashes: Vec::new(),
            worktrees: Vec::new(),
            commits: Vec::new(),
            working: WorkingTree::default(),
            loaded_limit: 200,
            has_more: false,
            refs_sig: 7,
            remote_url: None,
        }
    }

    #[test]
    fn stored_snapshots_round_trip_and_overwrite_by_repository() {
        let directory = tempfile::tempdir().expect("temp dir");
        let cache = SnapshotCache::at(directory.path()).expect("open cache");
        let repo = PathBuf::from("/tmp/example");
        assert!(cache.load(&repo).is_none(), "empty cache misses");

        cache.store(&repo, &snapshot("main")).expect("store");
        let loaded = cache.load(&repo).expect("hit after store");
        assert_eq!(loaded.head, "main");
        assert_eq!(loaded.refs_sig, 7);

        cache.store(&repo, &snapshot("release")).expect("overwrite");
        assert_eq!(cache.load(&repo).expect("hit").head, "release");
        assert!(
            cache.load(&PathBuf::from("/tmp/other")).is_none(),
            "keys are per repository"
        );
    }

    #[test]
    fn stored_avatars_round_trip_and_overwrite_by_key() {
        let directory = tempfile::tempdir().expect("temp dir");
        let cache = AvatarCache::at(directory.path()).expect("open cache");
        assert!(cache.load("abc").is_none(), "empty cache misses");

        cache.store("abc", b"\x89PNG-first").expect("store");
        assert_eq!(cache.load("abc").as_deref(), Some(&b"\x89PNG-first"[..]));

        cache.store("abc", b"\x89PNG-second").expect("overwrite");
        assert_eq!(cache.load("abc").as_deref(), Some(&b"\x89PNG-second"[..]));
        assert!(cache.load("def").is_none(), "keys are independent");
    }
}
