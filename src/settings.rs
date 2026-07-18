use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

/// A repository offered on the welcome screen.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct RecentRepo {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) last_opened: i64,
}

/// A commit identity selectable from Preferences > Profiles.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct CommitProfile {
    pub(crate) name: String,
    pub(crate) author_name: String,
    pub(crate) author_email: String,
}

/// Persisted graph, chrome, editor, and repository behavior settings.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) auto_fetch_minutes: u16,
    pub(crate) auto_prune: bool,
    pub(crate) keep_submodules_updated: bool,
    pub(crate) default_branch_name: String,
    pub(crate) default_pull_operation: crate::git::models::PullOperation,
    pub(crate) delete_orig_after_merge: bool,
    pub(crate) show_all_commits: bool,
    pub(crate) initial_commits: usize,
    pub(crate) lazy_load_commits: bool,
    pub(crate) remember_tabs: bool,
    pub(crate) extended_logging: bool,
    pub(crate) proactive_conflict_detection: bool,
    pub(crate) share_branch_status: bool,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) show_agents: bool,
    pub(crate) spell_check: bool,
    pub(crate) show_commit_author: bool,
    pub(crate) show_commit_date: bool,
    pub(crate) show_commit_sha: bool,
    pub(crate) editor_font_size: u16,
    pub(crate) terminal_font_size: u16,
    pub(crate) recent_repos: Vec<RecentRepo>,
    pub(crate) profiles: Vec<CommitProfile>,
    pub(crate) selected_profile: Option<String>,
    pub(crate) use_local_ssh_agent: bool,
    pub(crate) ssh_private_key: String,
    pub(crate) ssh_public_key: String,
    pub(crate) external_editor: String,
    pub(crate) external_terminal: String,
    pub(crate) show_external_tool_arguments: bool,
    pub(crate) gpg_program: String,
    pub(crate) gpg_key_id: String,
    pub(crate) sign_commits_by_default: bool,
    pub(crate) sign_tags_by_default: bool,
    pub(crate) use_git_executable: bool,
    pub(crate) git_executable: String,
    pub(crate) notify_operation_success: bool,
    pub(crate) notify_operation_failure: bool,
    pub(crate) notify_fetch_results: bool,
    pub(crate) default_encoding: String,
    pub(crate) gitflow_main_branch: String,
    pub(crate) gitflow_develop_branch: String,
    pub(crate) gitflow_feature_prefix: String,
    pub(crate) gitflow_release_prefix: String,
    pub(crate) gitflow_hotfix_prefix: String,
    pub(crate) lfs_patterns: Vec<String>,
    pub(crate) sparse_checkout_paths: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            auto_fetch_minutes: 5,
            auto_prune: true,
            default_pull_operation: crate::git::models::PullOperation::FastForward,
            keep_submodules_updated: true,
            default_branch_name: "main".to_owned(),
            delete_orig_after_merge: false,
            show_all_commits: false,
            initial_commits: 500,
            lazy_load_commits: true,
            remember_tabs: true,
            extended_logging: false,
            proactive_conflict_detection: false,
            share_branch_status: false,
            sidebar_collapsed: false,
            show_agents: true,
            spell_check: true,
            show_commit_author: false,
            show_commit_date: true,
            show_commit_sha: true,
            editor_font_size: 12,
            terminal_font_size: 12,
            recent_repos: Vec::new(),
            profiles: Vec::new(),
            selected_profile: None,
            use_local_ssh_agent: true,
            ssh_private_key: String::new(),
            ssh_public_key: String::new(),
            external_editor: String::new(),
            external_terminal: if cfg!(target_os = "macos") {
                "open -a Terminal".to_owned()
            } else {
                "x-terminal-emulator".to_owned()
            },
            show_external_tool_arguments: false,
            gpg_program: "gpg".to_owned(),
            gpg_key_id: String::new(),
            sign_commits_by_default: false,
            sign_tags_by_default: false,
            use_git_executable: false,
            git_executable: "git".to_owned(),
            notify_operation_success: true,
            notify_operation_failure: true,
            notify_fetch_results: true,
            default_encoding: "UTF-8".to_owned(),
            gitflow_main_branch: "main".to_owned(),
            gitflow_develop_branch: "develop".to_owned(),
            gitflow_feature_prefix: "feature/".to_owned(),
            gitflow_release_prefix: "release/".to_owned(),
            gitflow_hotfix_prefix: "hotfix/".to_owned(),
            lfs_patterns: Vec::new(),
            sparse_checkout_paths: String::new(),
        }
    }
}

/// Resolves and writes the platform-native TOML preferences file.
#[derive(Clone, Debug)]
pub(crate) struct SettingsStore {
    path: Option<PathBuf>,
}

impl SettingsStore {
    /// Uses the operating system's application-support directory when available.
    pub(crate) fn platform() -> Self {
        let path = ProjectDirs::from("ac", "Kraken Native", "Kraken Native")
            .map(|dirs| dirs.config_dir().join("settings.toml"));
        Self { path }
    }

    /// Uses an explicit path for deterministic tests and automation.
    #[cfg(test)]
    pub(crate) fn at(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    /// Loads settings, falling back only when the file does not exist.
    pub(crate) fn load(&self) -> Result<Settings> {
        let Some(path) = &self.path else {
            return Ok(Settings::default());
        };
        if !path.exists() {
            return Ok(Settings::default());
        }
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("read settings {}", path.display()))?;
        toml::from_str(&source).with_context(|| format!("parse settings {}", path.display()))
    }

    /// Atomically replaces the persisted settings file.
    pub(crate) fn save(&self, settings: &Settings) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create settings directory {}", parent.display()))?;
        let source = toml::to_string_pretty(settings).context("serialize settings")?;
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, source)
            .with_context(|| format!("write settings temporary {}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .with_context(|| format!("replace settings {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_settings_round_trip_preserves_behavior() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = SettingsStore::at(directory.path().join("settings.toml"));
        let configured = Settings {
            auto_fetch_minutes: 17,
            auto_prune: false,
            keep_submodules_updated: false,
            default_pull_operation: crate::git::models::PullOperation::Rebase,
            default_branch_name: "trunk".to_owned(),
            delete_orig_after_merge: true,
            show_all_commits: true,
            initial_commits: 10_000,
            lazy_load_commits: false,
            remember_tabs: false,
            extended_logging: true,
            proactive_conflict_detection: true,
            share_branch_status: true,
            show_agents: false,
            spell_check: false,
            show_commit_author: true,
            show_commit_date: false,
            show_commit_sha: false,
            editor_font_size: 19,
            terminal_font_size: 12,
            recent_repos: Vec::new(),
            profiles: vec![CommitProfile {
                name: "Work".to_owned(),
                author_name: "Ada".to_owned(),
                author_email: "ada@example.com".to_owned(),
            }],
            selected_profile: Some("Work".to_owned()),
            ssh_private_key: "/tmp/id_ed25519".to_owned(),
            external_editor: "code --wait".to_owned(),
            sign_commits_by_default: true,
            gitflow_develop_branch: "integration".to_owned(),
            sparse_checkout_paths: "src\nCargo.toml".to_owned(),
            ..Settings::default()
        };
        store.save(&configured).expect("save settings");
        let loaded = store.load().expect("load settings");
        assert_eq!(loaded.auto_fetch_minutes, 17);
        assert_eq!(loaded.default_branch_name, "trunk");
        assert_eq!(loaded.initial_commits, 10_000);
        assert!(loaded.proactive_conflict_detection);
        assert_eq!(loaded.selected_profile.as_deref(), Some("Work"));
        assert_eq!(loaded.profiles[0].author_email, "ada@example.com");
        assert_eq!(loaded.ssh_private_key, "/tmp/id_ed25519");
        assert!(loaded.sign_commits_by_default);
        assert_eq!(loaded.gitflow_develop_branch, "integration");
    }
}
