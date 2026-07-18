use crate::{
    app::state::AppState,
    ui::{
        FontFace, Rect, Scene, Theme,
        action::{CursorHint, ScrollTarget, UiAction},
        icons,
        widgets::{button, checkbox, divider, scrollbar, section_label},
    },
};

const GLOBAL_PAGES: &[&str] = &[
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
const REPO_PAGES: &[&str] = &["Encoding", "Gitflow", "LFS", "Sparse Checkout"];

pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme) {
    let viewport = scene.viewport();
    scene.rect(0, viewport, viewport, theme.window);
    let top = Rect::new(0.0, 0.0, viewport.width, 46.0);
    scene.rect(1, top, viewport, theme.top_bar);
    divider(
        scene,
        Rect::new(0.0, top.bottom() - 1.0, top.width, 1.0),
        theme,
    );
    let exit = Rect::new(14.0, 9.0, 142.0, 29.0);
    button(
        scene,
        exit,
        format!("{}  Exit Preferences", icons::CHEVRON_LEFT),
        UiAction::ExitPreferences,
        state.mouse,
        theme,
        false,
        true,
        Some("Return to repository"),
    );
    scene.text(
        "PREFERENCES",
        [viewport.width * 0.5 - 48.0, 16.0],
        top,
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );

    let sidebar = Rect::new(0.0, top.bottom(), 250.0, viewport.height - top.height);
    scene.rect(0, sidebar, viewport, theme.panel);
    divider(
        scene,
        Rect::new(sidebar.right() - 1.0, sidebar.y, 1.0, sidebar.height),
        theme,
    );
    build_nav(scene, state, theme, sidebar);
    let content = Rect::new(
        sidebar.right(),
        top.bottom(),
        viewport.width - sidebar.width,
        viewport.height - top.height,
    );
    build_page(scene, state, theme, content);
}

fn build_nav(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let profile = Rect::new(rect.x + 12.0, rect.y + 16.0, rect.width - 24.0, 50.0);
    scene.rounded_rect(
        1,
        profile,
        rect,
        theme.panel_alt,
        theme.border_strong,
        0.0,
        1.0,
    );
    scene.rounded_rect(
        2,
        Rect::new(profile.x + 10.0, profile.y + 10.0, 30.0, 30.0),
        rect,
        theme.green,
        theme.green,
        0.0,
        0.0,
    );
    scene.text(
        "OSS",
        [profile.x + 48.0, profile.y + 12.0],
        Rect::new(profile.x + 45.0, profile.y + 6.0, 130.0, 36.0),
        theme.text,
        13.0,
        17.0,
        FontFace::Sans,
    );
    scene.text(
        "CURRENT PROFILE",
        [profile.x + 48.0, profile.y + 28.0],
        Rect::new(
            profile.x + 45.0,
            profile.y + 24.0,
            profile.width - 55.0,
            22.0,
        ),
        theme.text_dim,
        10.0,
        13.0,
        FontFace::Monospace,
    );
    scene.text(
        icons::CHEVRON_DOWN,
        [profile.right() - 20.0, profile.y + 18.0],
        profile,
        theme.text_muted,
        12.0,
        16.0,
        FontFace::Sans,
    );
    let organization = Rect::new(
        rect.x + 12.0,
        profile.bottom() + 8.0,
        rect.width - 24.0,
        36.0,
    );
    scene.rounded_rect(
        1,
        organization,
        rect,
        theme.input,
        theme.border_strong,
        0.0,
        1.0,
    );
    scene.text(
        "ORGANIZATION",
        [organization.x + 10.0, organization.y + 10.0],
        organization.inset(5.0),
        theme.text_dim,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    scene.text(
        "NA",
        [organization.x + 85.0, organization.y + 10.0],
        organization.inset(5.0),
        theme.text_muted,
        11.0,
        14.0,
        FontFace::Sans,
    );
    scene.text(
        icons::CHEVRON_DOWN,
        [organization.right() - 20.0, organization.y + 10.0],
        organization,
        theme.text_muted,
        12.0,
        16.0,
        FontFace::Sans,
    );

    let mut y = organization.bottom() + 24.0;
    section_label(
        scene,
        Rect::new(rect.x + 14.0, y, rect.width - 28.0, 18.0),
        "PREFERENCES",
        theme,
    );
    y += 20.0;
    for page in GLOBAL_PAGES {
        y = nav_row(scene, state, theme, rect, y, page);
    }
    y += 18.0;
    section_label(
        scene,
        Rect::new(rect.x + 14.0, y, rect.width - 28.0, 18.0),
        "REPO-SPECIFIC PREFERENCES",
        theme,
    );
    y += 20.0;
    scene.text(
        format!(
            "REPO: {}",
            state
                .snapshot
                .as_ref()
                .map_or("NONE", |snapshot| snapshot.name.as_str())
                .to_uppercase()
        ),
        [rect.x + 16.0, y],
        Rect::new(rect.x + 14.0, y, rect.width - 28.0, 18.0),
        theme.text_dim,
        10.0,
        13.0,
        FontFace::Monospace,
    );
    y += 20.0;
    for page in REPO_PAGES {
        y = nav_row(scene, state, theme, rect, y, page);
    }
}

fn nav_row(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    y: f32,
    page: &str,
) -> f32 {
    let row = Rect::new(clip.x, y, clip.width, 26.0);
    if row.bottom() >= clip.y && row.y <= clip.bottom() {
        let selected = state.preference_page == page;
        if selected {
            scene.rect(
                1,
                Rect::new(row.x, row.y, 2.0, row.height),
                clip,
                theme.accent,
            );
        } else if row.contains(state.mouse) {
            scene.rect(1, row, clip, theme.panel_alt);
        }
        scene.text(
            page,
            [row.x + 22.0, row.y + 6.0],
            Rect::new(row.x + 20.0, row.y, row.width - 24.0, row.height),
            if selected {
                theme.text
            } else {
                theme.text_muted
            },
            13.0,
            17.0,
            FontFace::Sans,
        );
        scene.hit(
            row,
            UiAction::SelectPreferencePage(page.to_owned()),
            CursorHint::Pointer,
            None,
        );
    }
    y + 26.0
}

fn build_page(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    scene.rect(0, rect, scene.viewport(), theme.window);
    let content_width = 870.0_f32.min(rect.width - 72.0);
    let content = Rect::new(
        rect.x + 42.0,
        rect.y + 25.0,
        content_width,
        rect.height - 45.0,
    );
    scene.text(
        format!("01 · {}", state.preference_page.to_uppercase()),
        [content.x, content.y],
        Rect::new(content.x, content.y, content.width, 18.0),
        theme.accent,
        10.0,
        14.0,
        FontFace::Monospace,
    );
    scene.text(
        &state.preference_page,
        [content.x, content.y + 20.0],
        Rect::new(content.x, content.y + 20.0, content.width, 28.0),
        theme.text,
        20.0,
        26.0,
        FontFace::Sans,
    );
    divider(
        scene,
        Rect::new(content.x, content.y + 51.0, content.width, 1.0),
        theme,
    );
    let body = Rect::new(
        content.x,
        content.y + 70.0,
        content.width,
        content.height - 70.0,
    );
    match state.preference_page.as_str() {
        "General" => build_general(scene, state, theme, body),
        "Profiles" => build_profiles(scene, state, theme, body),
        "SSH" => build_ssh(scene, state, theme, body),
        "External Tools" => build_external_tools(scene, state, theme, body),
        "Commit Signing" => build_signing(scene, state, theme, body),
        "Notifications" => build_notifications(scene, state, theme, body),
        "Experimental" => build_experimental(scene, state, theme, body),
        "UI Customization" => build_customization(scene, state, theme, body),
        "Editor" => build_editor(scene, state, theme, body),
        "In-App Terminal" => build_terminal(scene, state, theme, body),
        "Encoding" => build_encoding(scene, state, theme, body),
        "Gitflow" => build_gitflow(scene, state, theme, body),
        "LFS" => build_lfs(scene, state, theme, body),
        "Sparse Checkout" => build_sparse_checkout(scene, state, theme, body),
        _ => build_general(scene, state, theme, body),
    }
    let content_height = page_content_height(&state.preference_page);
    let scroll = state
        .preferences_scroll
        .min((content_height - body.height).max(0.0));
    scrollbar(
        scene,
        body,
        content_height,
        scroll,
        ScrollTarget::Preferences,
        theme,
    );
}

fn page_content_height(page: &str) -> f32 {
    match page {
        "Profiles" | "Gitflow" => 480.0,
        "SSH" | "External Tools" | "Commit Signing" | "Sparse Checkout" => 360.0,
        "LFS" => 340.0,
        "General" => 464.0,
        "UI Customization" => 306.0,
        _ => 260.0,
    }
}

fn build_general(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y
        - state
            .preferences_scroll
            .min((page_content_height("General") - rect.height).max(0.0));
    numeric_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Auto-Fetch Interval",
        &format!("{} min", state.settings.auto_fetch_minutes),
        "auto_fetch_minutes",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Auto-Prune",
        state.settings.auto_prune,
        "auto_prune",
    );
    numeric_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Initial Commits in Graph",
        &state.settings.initial_commits.to_string(),
        "initial_commits",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Lazy Load Commits in Graph",
        state.settings.lazy_load_commits,
        "lazy_load_commits",
    );
}

fn build_customization(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y
        - state
            .preferences_scroll
            .min((page_content_height("UI Customization") - rect.height).max(0.0));
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Show toolbar icon labels",
        state.settings.show_toolbar_labels,
        "show_toolbar_labels",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Show commit author avatar",
        state.settings.show_commit_author,
        "show_commit_author",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Show commit date/time",
        state.settings.show_commit_date,
        "show_commit_date",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Show commit SHA",
        state.settings.show_commit_sha,
        "show_commit_sha",
    );
}

fn build_editor(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y;
    numeric_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Font Size",
        &state.settings.editor_font_size.to_string(),
        "editor_font_size",
    );
}

fn build_terminal(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y;
    numeric_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Font Size",
        &state.settings.terminal_font_size.to_string(),
        "terminal_font_size",
    );
}

fn build_profiles(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    button(
        scene,
        Rect::new(rect.x, y, 132.0, 28.0),
        "+ Add profile",
        UiAction::AddCommitProfile,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
    y += 42.0;
    for profile in &state.settings.profiles {
        let selected = state.settings.selected_profile.as_deref() == Some(profile.name.as_str());
        button(
            scene,
            Rect::new(rect.x, y, rect.width, 28.0),
            &format!(
                "{}  ·  {}  ·  {}",
                profile.name, profile.author_name, profile.author_email
            ),
            UiAction::SelectCommitProfile(profile.name.clone()),
            state.mouse,
            theme,
            selected,
            true,
            None,
        );
        y += 32.0;
    }
    if state.settings.selected_profile.is_some() {
        text_setting(
            scene,
            state,
            theme,
            rect,
            &mut y,
            "Profile name",
            &state.preference_text_value("profile_name"),
            "profile_name",
            None,
        );
        text_setting(
            scene,
            state,
            theme,
            rect,
            &mut y,
            "Author name",
            &state.preference_text_value("profile_author_name"),
            "profile_author_name",
            None,
        );
        text_setting(
            scene,
            state,
            theme,
            rect,
            &mut y,
            "Author email",
            &state.preference_text_value("profile_author_email"),
            "profile_author_email",
            None,
        );
    }
}

fn build_ssh(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Use local SSH agent",
        state.settings.use_local_ssh_agent,
        "use_local_ssh_agent",
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Private key",
        &state.settings.ssh_private_key,
        "ssh_private_key",
        Some("Browse"),
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Public key",
        &state.settings.ssh_public_key,
        "ssh_public_key",
        Some("Browse"),
    );
}

fn build_external_tools(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "External editor command",
        &state.settings.external_editor,
        "external_editor",
        None,
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "External terminal command",
        &state.settings.external_terminal,
        "external_terminal",
        None,
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Show arguments when launching",
        state.settings.show_external_tool_arguments,
        "show_external_tool_arguments",
    );
    button(
        scene,
        Rect::new(rect.x, y, 180.0, 28.0),
        "Open external editor",
        UiAction::OpenExternalEditor,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
    button(
        scene,
        Rect::new(rect.x + 190.0, y, 190.0, 28.0),
        "Open external terminal",
        UiAction::OpenExternalTerminal,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

fn build_signing(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "GPG program",
        &state.settings.gpg_program,
        "gpg_program",
        Some("Browse"),
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Signing key ID",
        &state.settings.gpg_key_id,
        "gpg_key_id",
        None,
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Sign commits by default",
        state.settings.sign_commits_by_default,
        "sign_commits_by_default",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Sign tags by default",
        state.settings.sign_tags_by_default,
        "sign_tags_by_default",
    );
}

fn build_notifications(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Operation successes",
        state.settings.notify_operation_success,
        "notify_operation_success",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Operation failures",
        state.settings.notify_operation_failure,
        "notify_operation_failure",
    );
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Fetch results",
        state.settings.notify_fetch_results,
        "notify_fetch_results",
    );
}

fn build_experimental(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    toggle_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Use Git executable",
        state.settings.use_git_executable,
        "use_git_executable",
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Git executable path",
        &state.settings.git_executable,
        "git_executable",
        Some("Browse"),
    );
}

fn build_encoding(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Default encoding (UTF-8 or Latin-1)",
        &state.settings.default_encoding,
        "default_encoding",
        None,
    );
}

fn build_gitflow(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Main branch",
        &state.settings.gitflow_main_branch,
        "gitflow_main_branch",
        None,
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Develop branch",
        &state.settings.gitflow_develop_branch,
        "gitflow_develop_branch",
        None,
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Feature prefix",
        &state.settings.gitflow_feature_prefix,
        "gitflow_feature_prefix",
        None,
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Release prefix",
        &state.settings.gitflow_release_prefix,
        "gitflow_release_prefix",
        None,
    );
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Hotfix prefix",
        &state.settings.gitflow_hotfix_prefix,
        "gitflow_hotfix_prefix",
        None,
    );
    button(
        scene,
        Rect::new(rect.x, y, 160.0, 28.0),
        "Initialize Gitflow",
        UiAction::InitializeGitflow,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

fn build_lfs(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Tracking pattern",
        &state.preference_text_value("lfs_pattern"),
        "lfs_pattern",
        None,
    );
    button(
        scene,
        Rect::new(rect.x, y, 170.0, 28.0),
        "Add tracking pattern",
        UiAction::AddLfsPattern,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
    y += 40.0;
    let mut patterns = state.settings.lfs_patterns.clone();
    if let Some(repository) = state.snapshot.as_ref()
        && let Ok(attributes) = std::fs::read_to_string(repository.path.join(".gitattributes"))
    {
        patterns.extend(
            attributes
                .lines()
                .filter_map(|line| {
                    line.split_whitespace()
                        .next()
                        .filter(|_| line.contains("filter=lfs"))
                })
                .map(str::to_owned),
        );
    }
    patterns.sort();
    patterns.dedup();
    for pattern in patterns {
        scene.text(
            format!("{pattern} filter=lfs"),
            [rect.x, y],
            rect,
            theme.text_muted,
            12.0,
            16.0,
            FontFace::Monospace,
        );
        y += 22.0;
    }
}

fn build_sparse_checkout(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let mut y = rect.y - state.preferences_scroll;
    text_setting(
        scene,
        state,
        theme,
        rect,
        &mut y,
        "Paths (space-separated)",
        &state.settings.sparse_checkout_paths,
        "sparse_checkout_paths",
        None,
    );
    button(
        scene,
        Rect::new(rect.x, y, 132.0, 28.0),
        "Apply",
        UiAction::ApplySparseCheckout,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
    button(
        scene,
        Rect::new(rect.x + 142.0, y, 132.0, 28.0),
        "Disable",
        UiAction::DisableSparseCheckout,
        state.mouse,
        theme,
        false,
        true,
        None,
    );
}

#[allow(clippy::too_many_arguments)]
fn text_setting(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    y: &mut f32,
    label: &str,
    value: &str,
    key: &str,
    browse: Option<&str>,
) {
    let row = Rect::new(clip.x, *y, clip.width, 36.0);
    if let Some(visible) = row.intersection(clip) {
        scene.text(
            label.to_uppercase(),
            [row.x, row.y + 11.0],
            visible,
            theme.text_muted,
            11.0,
            17.0,
            FontFace::Monospace,
        );
        let field = Rect::new(row.x + 250.0, row.y + 4.0, 300.0, 28.0);
        scene.rounded_rect(1, field, clip, theme.input, theme.border_strong, 0.0, 1.0);
        scene.text(
            value,
            [field.x + 8.0, field.y + 7.0],
            field.inset(6.0),
            theme.text,
            11.5,
            15.0,
            FontFace::Monospace,
        );
        if state.focus == crate::app::state::FocusField::PreferenceText
            && state.preference_text_key.as_deref() == Some(key)
        {
            crate::ui::widgets::caret_overlay(
                scene,
                2,
                [field.x + 8.0, field.y + 7.0],
                field.inset(2.0),
                &state.preference_text,
                6.9,
                15.0,
                theme,
            );
        }
        scene.hit(
            field,
            UiAction::FocusPreferenceText(key.to_owned()),
            CursorHint::Text,
            None,
        );
        if browse.is_some() {
            button(
                scene,
                Rect::new(field.right() + 8.0, field.y, 76.0, 28.0),
                "Browse",
                UiAction::BrowsePreferencePath(key.to_owned()),
                state.mouse,
                theme,
                false,
                true,
                None,
            );
        }
    }
    *y += 38.0;
}

#[allow(clippy::too_many_arguments)]
fn toggle_setting(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    y: &mut f32,
    label: &str,
    value: bool,
    key: &str,
) {
    let row = Rect::new(clip.x, *y, clip.width, 32.0);
    if row.y >= clip.y && row.bottom() <= clip.bottom() {
        checkbox(
            scene,
            row,
            label,
            value,
            UiAction::TogglePreference(key.to_owned()),
            state.mouse,
            theme,
        );
    }
    *y += 32.0;
}

#[allow(clippy::too_many_arguments)]
fn numeric_setting(
    scene: &mut Scene,
    state: &AppState,
    theme: &Theme,
    clip: Rect,
    y: &mut f32,
    label: &str,
    value: &str,
    key: &str,
) {
    let row = Rect::new(clip.x, *y, clip.width, 36.0);
    if let Some(visible_row) = row.intersection(clip) {
        scene.text(
            label.to_uppercase(),
            [row.x, row.y + 11.0],
            Rect::new(row.x, row.y, 250.0, row.height)
                .intersection(clip)
                .unwrap_or(visible_row),
            theme.text_muted,
            11.0,
            17.0,
            FontFace::Monospace,
        );
        let field = Rect::new(row.x + 250.0, row.y + 4.0, 160.0, 28.0);
        scene.rounded_rect(1, field, clip, theme.input, theme.border_strong, 0.0, 1.0);
        scene.text(
            value,
            [field.x + 8.0, field.y + 7.0],
            Rect::new(field.x + 8.0, field.y, field.width - 50.0, field.height)
                .intersection(clip)
                .unwrap_or(visible_row),
            theme.text_muted,
            11.5,
            15.0,
            FontFace::Monospace,
        );
        if row.y >= clip.y && row.bottom() <= clip.bottom() {
            button(
                scene,
                Rect::new(field.right() - 52.0, field.y + 2.0, 24.0, 24.0),
                icons::REMOVE,
                UiAction::AdjustPreference {
                    key: key.to_owned(),
                    delta: -1,
                },
                state.mouse,
                theme,
                false,
                true,
                None,
            );
            button(
                scene,
                Rect::new(field.right() - 26.0, field.y + 2.0, 24.0, 24.0),
                icons::ADD,
                UiAction::AdjustPreference {
                    key: key.to_owned(),
                    delta: 1,
                },
                state.mouse,
                theme,
                false,
                true,
                None,
            );
        }
    }
    *y += 38.0;
}
