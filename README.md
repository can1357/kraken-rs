# Kraken Native

Kraken Native is a native, GPU-rendered Git desktop client written in Rust. It uses `winit` for windowing, `wgpu` for rendering, `glyphon` for text, and `libgit2` for repository operations; no browser or web view is embedded in the application shell.

The project is under active development. Core local Git workflows are real repository operations, while the hosted-service surfaces listed under [Current boundaries](#current-boundaries) are not connected.

## Current capabilities

- Discover, open, create, and clone repositories; search recent repositories and keep multiple repositories in tabs.
- Browse a virtualized topological commit graph with stable lanes, merge curves, branch and tag labels, WIP rows, linked-worktree state, search, and resizable columns.
- Inspect commit metadata, parent links, conflicts, changed paths, and complete commit trees.
- Stage or unstage individual files, all files, or selected diff lines; discard selected unstaged lines; create, amend, and optionally push commits.
- Read inline or split diffs with aligned lines, syntax highlighting, intraline marks, staged/unstaged scopes, search, hunk navigation, a change minimap, text selection, and full-file views.
- Create, check out, rename, and delete branches; create, check out, and delete tags; apply, pop, and drop stashes.
- Fetch, pull with selectable integration behavior, push, fast-forward, merge, rebase, cherry-pick, revert, and reset.
- Undo and redo recorded commit, checkout, reset, and branch mutations.
- Use an in-app PTY terminal, command palettes, notifications, persisted preferences, commit profiles, SSH settings, commit signing, external tools, Gitflow, Git LFS, and sparse checkout.
- Render deterministic offscreen PNGs and expose a loopback automation endpoint for semantic UI driving.
- Request commit explanations or rewritten commit messages from an optional HTTP AI provider.

## Requirements

- Rust 1.85 or newer (the crate uses Rust 2024 edition).
- A desktop and graphics backend supported by `winit` and `wgpu`.
- Git on `PATH` for CLI-backed features such as signed commits, Gitflow, sparse checkout, and Git LFS operations.
- Optional: Git LFS and GPG for their respective workflows.

## Run

Open a repository explicitly:

```sh
cargo run --release -- --repo /path/to/repository
```

If `--repo` is omitted, Kraken discovers a repository from the current directory. When discovery fails, it opens the welcome screen, where repositories can be opened, created, or cloned.

Build and run the binary directly:

```sh
cargo build --release
./target/release/kraken --repo /path/to/repository
```

See every command-line option with:

```sh
cargo run -- --help
```

## Deterministic rendering

Render a view to a PNG without opening a window:

```sh
cargo run --release -- \
  --repo /path/to/repository \
  --screenshot graph \
  --out graph.png \
  --width 1600 \
  --height 900
```

Available views are `graph`, `wip`, `diff`, `file`, `preferences`, and `tabs`. Output defaults to `kraken.png`; the default render size is 2404 by 1354 physical pixels.

Start the headless semantic automation endpoint with:

```sh
cargo run --release -- --repo /path/to/repository --automation-port 0
```

The server binds to loopback only. Port `0` selects an available port; startup prints an `Automation.ready` JSON message containing the host, selected port, and protocol version.

## Optional AI provider

AI requests require both an endpoint and API key:

| Variable | Meaning |
|---|---|
| `KRAKEN_AI_ENDPOINT` | HTTP endpoint accepting a chat-completion-shaped JSON request |
| `KRAKEN_AI_API_KEY` | Bearer token sent to the endpoint |
| `KRAKEN_AI_MODEL` | Optional model name; defaults to `claude-sonnet-4-6` |

Example:

```sh
KRAKEN_AI_ENDPOINT=https://example.test/v1/chat/completions \
KRAKEN_AI_API_KEY=secret \
KRAKEN_AI_MODEL=my-model \
cargo run --release -- --repo /path/to/repository
```

Without both required variables, the UI reports that the provider is unconfigured and sends no request.

## Development

Run the repository test suite:

```sh
cargo test
```

Check all targets without producing binaries:

```sh
cargo check --all-targets
```

### Architecture

| Path | Responsibility |
|---|---|
| `src/app/` | Event loop, application state, automation protocol, and AI worker |
| `src/git/` | Repository models, `libgit2` backend, filesystem watching, and background Git worker |
| `src/graph/` | Commit-lane layout and avatar loading |
| `src/views/` | Immediate-mode construction of graph, WIP, diff, preferences, welcome, and terminal views |
| `src/ui/` | Scene primitives, hit regions, widgets, geometry, actions, and theme |
| `src/gpu/` | Windowed and offscreen `wgpu` renderers |
| `src/term/` | PTY process management and terminal-grid parsing |
| `src/settings.rs` | Platform-native TOML settings persistence |

Repository work runs off the window event loop. Each Git job opens a fresh repository handle, and versioned results prevent stale background responses from replacing newer UI state.

## Current boundaries

- Pull request, issue, team, organization, and cloud-patch sections are visual shell entries, not hosted-service integrations.
- There is no conflict-resolution editor or interactive rebase editor.
- File History currently presents the selected diff in a history layout; it does not enumerate revisions for the path.
- Linked worktrees are discovered and monitored, but the UI does not create or open worktrees.
