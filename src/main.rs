mod app;
mod git;
mod gpu;
mod graph;
mod settings;
mod term;
mod ui;
mod views;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

use crate::app::{LaunchOptions, ScreenshotView};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ScreenshotArg {
    Graph,
    Wip,
    Diff,
    File,
    Preferences,
    Tabs,
}

impl From<ScreenshotArg> for ScreenshotView {
    fn from(value: ScreenshotArg) -> Self {
        match value {
            ScreenshotArg::Graph => Self::Graph,
            ScreenshotArg::Wip => Self::Wip,
            ScreenshotArg::Diff => Self::Diff,
            ScreenshotArg::File => Self::File,
            ScreenshotArg::Preferences => Self::Preferences,
            ScreenshotArg::Tabs => Self::Tabs,
        }
    }
}

#[derive(Debug, Parser)]
#[command(name = "kraken", about = "Native wgpu Git desktop client")]
struct Args {
    /// Repository to open; defaults to discovery from the current directory.
    #[arg(long)]
    repo: Option<PathBuf>,

    /// Render one deterministic frame without opening a window.
    #[arg(long, value_enum)]
    screenshot: Option<ScreenshotArg>,

    /// Start the headless CDP-like automation endpoint on this port; use 0 for any free port.
    #[arg(long, conflicts_with = "screenshot")]
    automation_port: Option<u16>,

    /// PNG path used by --screenshot.
    #[arg(long, default_value = "kraken.png")]
    out: PathBuf,

    /// Render width in physical pixels.
    #[arg(long, default_value_t = 2404)]
    width: u32,

    /// Render height in physical pixels.
    #[arg(long, default_value_t = 1354)]
    height: u32,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let requested = args.repo.or_else(|| std::env::current_dir().ok());
    let repo = requested.and_then(|path| {
        git2::Repository::discover(path)
            .ok()
            .and_then(|repository| repository.workdir().map(std::path::Path::to_path_buf))
    });
    let options = LaunchOptions {
        repo,
        screenshot: args.screenshot.map(Into::into),
        automation_port: args.automation_port,
        output: args.out,
        width: args.width.max(640),
        height: args.height.max(480),
    };
    app::run(options)
}
