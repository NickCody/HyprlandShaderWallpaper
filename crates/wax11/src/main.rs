//! Entry point wiring that stitches together the CLI surface, filesystem bootstrap, and
//! single- or multi-playlist runtime paths before delegating to `run.rs`, while exposing
//! utility commands like `wax11 defaults where`.
//!
//! Types:
//!
//! - None; this module focuses on orchestrating submodules.
//!
//! Functions:
//!
//! - `main` parses CLI input, initialises tracing, and dispatches to modes.
//! - `handle_defaults_command` and `run_defaults_where` back the defaults subcommand.

mod bindings;
mod bootstrap;
mod cli;
mod defaults;
mod diagnostics;
mod handles;
mod multi;
mod paths;
mod run;

use anyhow::Result;
use cli::{Command, DefaultsAction};
use defaults::describe_paths;
use paths::AppPaths;

fn main() -> Result<()> {
    let cli = cli::parse();
    run::initialise_tracing();

    match cli.command {
        Some(Command::Defaults(defaults_cmd)) => handle_defaults_command(defaults_cmd.action),
        None => run::run(cli.run),
    }
}

fn handle_defaults_command(action: DefaultsAction) -> Result<()> {
    let paths = AppPaths::discover()?;
    bootstrap::bootstrap_filesystem(&paths)?;

    match action {
        DefaultsAction::Where => run_defaults_where(&paths),
    }
}
fn run_defaults_where(paths: &AppPaths) -> Result<()> {
    let overview = describe_paths(paths);
    println!("Configuration directories:");
    println!("  config:     {}", overview.config_dir.display());
    println!("  data:       {}", overview.data_dir.display());
    println!("  cache:      {}", overview.cache_dir.display());
    println!("  share:      {}", overview.share_dir.display());
    println!("  state:      {}", overview.state_file.display());
    println!("  shadertoy:  {}", overview.shadertoy_cache.display());
    println!("Shader search roots:");
    for root in overview.shader_roots {
        println!("  {}", root.display());
    }
    Ok(())
}
