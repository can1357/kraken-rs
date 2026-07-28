# Repository Guidelines

## Project Overview

Kraken Native (`kraken-native`, binary `kraken`) is a native, GPU-rendered Git desktop client. The UI is authored in Slab (`ui/app.slab`), compiled to Rust at build time, solved by the Slab kernel, and painted with `wgpu`. No browser or web view is embedded.

## Architecture & Data Flow

```
winit input ─► slab_kernel dispatch ─► UiAction (src/ui/slab.rs)
      ─► AppState::dispatch (src/app/state.rs)
            ├─ direct state mutation ─► scene rebuild ─► SlabRenderer ─► wgpu
            └─ GitJob ─► GitRunner thread (crossbeam) ─► GitEvent
                     ─► UserEvent via EventLoopProxy wakes the winit loop
```

- All UI interactions become `UiAction` variants; `AppState` is the single state root.
- Repository work never runs on the event loop. `GitRunner` (`src/git/runner.rs`) owns a worker thread, opens a fresh repo handle per job, and versions results so stale responses cannot overwrite newer UI state. Same pattern for AI (`src/app/ai.rs`), avatars (`src/graph/avatars.rs`), and the PTY reader (`src/term/pty.rs`).
- `slab_macro::include_doc!` in `src/ui/slab.rs` compiles `ui/app.slab` plus the two fonts in `assets/fonts/` into the typed `generated` module at macro expansion time; the font pairs keep glyph ids aligned with the faces `src/gpu/slab.rs` registers. There is no `build.rs`.

### Adding a feature

1. Author visuals/signals in `ui/app.slab`.
2. Add a `UiAction` variant in `src/ui/action.rs`; map the signal in `src/ui/slab.rs`.
3. Handle it in `AppState::dispatch` (`src/app/state.rs`). Modals use `Overlay`; text inputs use `FocusField`.
4. New Git operations: extend the `Backend` trait (`src/git/backend.rs`), add a `GitJobKind`, and execute it in `src/git/runner.rs`.

## Key Directories

| Path | Responsibility |
|---|---|
| `src/app/` | winit event loop, `AppState`, automation endpoint, AI worker, command palette, native menus |
| `src/git/` | `libgit2` backend with CLI fallbacks, background worker, filesystem watching, domain models |
| `src/graph/` | Topological commit-lane layout and avatar fetching/atlas |
| `src/ui/` | Slab bridge, `UiAction`, layout math, menus, icons, geometry, text fields |
| `src/gpu/` | Windowed and offscreen `wgpu` renderers |
| `src/term/` | PTY spawning, VTE grid, terminal hole mounted in the Slab UI |
| `ui/` | `app.slab` — the authoritative declarative UI |
| `assets/fonts/` | Instrument Sans (UI) and Berkeley Mono Nerd Font (code, icons) |
| `.omp/tools/` | `kraken-qa.ts` — QA driver for the automation protocol |

## Development Commands

```sh
cargo check --all-targets            # fast validation
cargo test                           # full test suite
cargo clippy --all-targets           # lints (pedantic is on)
cargo run --release -- --repo <path> # run against a repository
```

Headless rendering and automation:

```sh
cargo run --release -- --repo <path> --screenshot graph --out graph.png
cargo run --release -- --repo <path> --automation-port 0
```

## Code Conventions & Common Patterns

- Rust 2024 edition. `unsafe_code = "deny"`. Clippy `all` + `pedantic` at `warn` (see `[lints]` in `Cargo.toml`).
- Error handling is `anyhow` throughout (`Result`, `Context`, `bail!`); no custom error enums in `src/`.
- Internal items use `pub(crate)`, not `pub`.
- Concurrency: `crossbeam-channel` for the Git worker, `EventLoopProxy<UserEvent>` to wake the main loop. Never block the event loop on I/O.
- State lives in `AppState`; UI is a projection of it. Do not store derived UI state elsewhere.

## Important Files

- `src/main.rs` — CLI (`clap`), repo discovery, `LaunchOptions`, hands off to `app::run`.
- `src/app/state.rs` — `AppState`, `Overlay`, `FocusField`, `AppState::dispatch`.
- `src/ui/action.rs` / `src/ui/slab.rs` — action enum and Slab signal bridge.
- `ui/app.slab` + the `include_doc!` invocation in `src/ui/slab.rs` — UI source and its compilation pipeline.
- `src/settings.rs` — `SettingsStore`; atomic TOML persistence under the platform config dir (`directories`).
- `.cargo/config.toml` — local-only, gitignored `[patch]` mapping slab crates to `../slab-lang` (see below).
- `codicon_mapping.json` — Nerd Font codepoint → icon name mapping.

## Runtime/Tooling Preferences

- **Slab dependencies**: `Cargo.toml` pulls `slab-*` crates from `https://github.com/stencil-hq/slab.git` (branch `main`). Local development builds against the sibling checkout via the gitignored `.cargo/config.toml` `[patch]` section. Keep that file local; never commit it, and never convert the deps back to `path = "../…"`.
- Rust 1.85+ with `cargo`. JS tooling (icon sources: `lucide`, `lucide-static`) uses **Bun**, not npm.
- Runtime settings are TOML at the platform config dir; writes are atomic via a `.tmp` file.

## Testing & QA

- Tests are inline `#[cfg(test)]` modules; `tempfile` builds throwaway Git repos. Run with `cargo test`.
- Densest coverage: `src/git/backend.rs` (staging, diffs, stash, reword), `src/app/state.rs` (dispatch/selection/mutations), `src/graph/layout.rs` (lanes, merges), `src/ui/slab.rs` (signal dispatch contract), `src/settings.rs` (persistence round-trip).
- Test behavior and edge values, not field wiring. New Git operations need a `backend.rs`-style test against a temp repo.
- Visual/semantic QA: offscreen PNG rendering (`--screenshot graph|wip|diff|file|preferences|tabs`) and a loopback JSON automation endpoint (`--automation-port 0`, prints `Automation.ready`). `.omp/tools/kraken-qa.ts` (the `kraken_qa` tool) drives it: launch, inspect semantic UI, click, type, screenshot.
- `artifacts/` and `qa/` are gitignored output directories for QA runs.
