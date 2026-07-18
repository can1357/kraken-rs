use crate::{
    app::state::{AppState, FocusField},
    ui::{
        FontFace, Rect, Scene, Theme,
        action::{CursorHint, UiAction},
        icons,
        widgets::truncated_text,
    },
};

/// GitKraken-style empty repository tab.
pub(super) fn build(scene: &mut Scene, state: &AppState, theme: &Theme, rect: Rect) {
    let left = Rect::new(
        rect.x + 138.0,
        rect.y + 24.0,
        (rect.width * 0.5 - 150.0).max(360.0),
        rect.height - 48.0,
    );
    let divider_x = rect.x + rect.width * 0.585;
    let right = Rect::new(
        divider_x + 16.0,
        rect.y + 24.0,
        rect.right() - divider_x - 80.0,
        rect.height - 48.0,
    );
    scene.rect(
        1,
        Rect::new(divider_x, rect.y + 20.0, 1.0, rect.height - 40.0),
        rect,
        theme.border,
    );

    scene.text(
        "Repositories",
        [left.x, left.y],
        left,
        theme.text,
        15.0,
        21.0,
        FontFace::Sans,
    );
    chip(
        scene,
        Rect::new(left.x, left.y + 30.0, 64.0, 32.0),
        format!("{}  Open", icons::FOLDER_OPEN),
        UiAction::OpenRepositoryPicker,
        state,
        theme,
    );
    chip(
        scene,
        Rect::new(left.x + 72.0, left.y + 30.0, 68.0, 32.0),
        format!("{}  Clone", icons::REPOSITORY_CLONE),
        UiAction::ToggleCloneForm,
        state,
        theme,
    );
    chip(
        scene,
        Rect::new(left.x + 148.0, left.y + 30.0, 72.0, 32.0),
        format!("{}  Create", icons::NEW_FOLDER),
        UiAction::CreateRepositoryPicker,
        state,
        theme,
    );

    let search = Rect::new(left.x, left.y + 78.0, left.width, 30.0);
    input(
        scene,
        search,
        &state.welcome_search,
        "Search repositories",
        state.focus == FocusField::WelcomeSearch,
        UiAction::FocusWelcomeSearch,
        theme,
    );
    scene.text(
        "Recent",
        [left.x, search.bottom() + 16.0],
        left,
        theme.text_muted,
        11.0,
        16.0,
        FontFace::Sans,
    );
    let filter = state.welcome_search.to_lowercase();
    let mut y = search.bottom() + 34.0;
    for recent in state.settings.recent_repos.iter().filter(|recent| {
        filter.is_empty()
            || recent.name.to_lowercase().contains(&filter)
            || recent
                .path
                .to_string_lossy()
                .to_lowercase()
                .contains(&filter)
    }) {
        let row = Rect::new(left.x, y, left.width, 19.0);
        if row.contains(state.hover()) {
            scene.rect(1, row, left, theme.panel_alt);
        }
        truncated_text(
            scene,
            &recent.name,
            [row.x + 2.0, row.y + 2.0],
            Rect::new(row.x + 2.0, row.y, 116.0, row.height),
            left,
            theme.accent,
            11.5,
            15.0,
            FontFace::Sans,
        );
        truncated_text(
            scene,
            &recent.path.to_string_lossy(),
            [row.x + 122.0, row.y + 2.0],
            Rect::new(row.x + 122.0, row.y, row.width - 122.0, row.height),
            left,
            theme.text_dim,
            11.0,
            15.0,
            FontFace::Sans,
        );
        scene.hit_clipped(
            row,
            left,
            UiAction::OpenRepository(recent.path.clone()),
            CursorHint::Pointer,
            None,
        );
        y += 20.0;
    }

    if state.clone_form {
        let form = Rect::new(left.x, y + 14.0, left.width, 116.0);
        scene.rounded_rect(1, form, left, theme.surface_3, theme.border, 3.0, 1.0);
        scene.text(
            "Clone a repository",
            [form.x + 12.0, form.y + 10.0],
            form,
            theme.text,
            12.0,
            16.0,
            FontFace::Sans,
        );
        input(
            scene,
            Rect::new(form.x + 12.0, form.y + 32.0, form.width - 24.0, 28.0),
            &state.clone_url,
            "Repository URL",
            state.focus == FocusField::CloneUrl,
            UiAction::FocusCloneUrl,
            theme,
        );
        let destination = state.clone_destination.as_ref().map_or_else(
            || "Choose destination".to_owned(),
            |path| path.to_string_lossy().into_owned(),
        );
        chip(
            scene,
            Rect::new(form.x + 12.0, form.y + 70.0, form.width - 108.0, 28.0),
            &destination,
            UiAction::PickCloneDestination,
            state,
            theme,
        );
        chip(
            scene,
            Rect::new(form.right() - 88.0, form.y + 70.0, 76.0, 28.0),
            "Clone",
            UiAction::CloneRepository,
            state,
            theme,
        );
    }

    scene.text(
        "Connect More Integrations",
        [right.x, right.y],
        right,
        theme.text,
        15.0,
        21.0,
        FontFace::Sans,
    );
    scene.text("Speed up your workflow and reduce context switching with powerful integrations for pull requests,\nissues, CI/CD, and more.", [right.x, right.y + 30.0], Rect::new(right.x, right.y + 30.0, right.width, 42.0), theme.text_muted, 11.5, 16.0, FontFace::Sans);
    chip(
        scene,
        Rect::new(right.x, right.y + 76.0, 146.0, 32.0),
        "Connect Integrations",
        UiAction::OpenPreferences,
        state,
        theme,
    );
    scene.text(
        "Resources",
        [right.x, right.y + 126.0],
        right,
        theme.text_muted,
        11.0,
        16.0,
        FontFace::Sans,
    );
    link(
        scene,
        Rect::new(right.x, right.y + 144.0, 150.0, 18.0),
        "Intro Tutorials",
        "https://help.gitkraken.com/",
        theme,
    );
    link(
        scene,
        Rect::new(right.x, right.y + 162.0, 150.0, 18.0),
        "Release Notes",
        "https://help.gitkraken.com/gitkraken-client/current-release-notes/",
        theme,
    );
    link(
        scene,
        Rect::new(right.x, right.y + 180.0, 150.0, 18.0),
        format!("Documentation {}", icons::EXTERNAL_LINK),
        "https://help.gitkraken.com/",
        theme,
    );
}

fn chip(
    scene: &mut Scene,
    rect: Rect,
    label: impl Into<String>,
    action: UiAction,
    state: &AppState,
    theme: &Theme,
) {
    let hovered = rect.contains(state.hover());
    scene.rounded_rect(
        1,
        rect,
        scene.viewport(),
        if hovered {
            theme.panel_alt
        } else {
            theme.surface_3
        },
        theme.border,
        3.0,
        1.0,
    );
    scene.text(
        label,
        [rect.x + 9.0, rect.y + 8.0],
        rect.inset(4.0),
        theme.text,
        11.5,
        15.0,
        FontFace::Sans,
    );
    scene.hit(rect, action, CursorHint::Pointer, None);
}

fn input(
    scene: &mut Scene,
    rect: Rect,
    field: &crate::ui::TextField,
    placeholder: &str,
    focused: bool,
    action: UiAction,
    theme: &Theme,
) {
    scene.rounded_rect(
        1,
        rect,
        scene.viewport(),
        theme.window,
        if focused { theme.accent } else { theme.border },
        3.0,
        if focused { 2.0 } else { 1.0 },
    );
    let text = if field.is_empty() {
        placeholder
    } else {
        field.text()
    };
    scene.text(
        text,
        [rect.x + 8.0, rect.y + 7.0],
        rect.inset(4.0),
        if field.is_empty() {
            theme.text_dim
        } else {
            theme.text
        },
        11.0,
        15.0,
        FontFace::Sans,
    );
    if focused {
        crate::ui::widgets::caret_overlay(
            scene,
            2,
            [rect.x + 8.0, rect.y + 7.0],
            rect.inset(2.0),
            field,
            6.0,
            15.0,
            theme,
        );
    }
    scene.hit(rect, action, CursorHint::Text, None);
}

fn link(scene: &mut Scene, rect: Rect, label: impl Into<String>, url: &str, theme: &Theme) {
    scene.text(
        label,
        [rect.x, rect.y + 1.0],
        rect,
        theme.accent,
        11.5,
        15.0,
        FontFace::Sans,
    );
    scene.hit(
        rect,
        UiAction::OpenExternalUrl(url.to_owned()),
        CursorHint::Pointer,
        None,
    );
}
