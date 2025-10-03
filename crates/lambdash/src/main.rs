mod bindings;
mod bootstrap;
mod cli;
mod defaults;
mod multi;
mod paths;
mod run;
mod state;

use anyhow::Result;
use cli::{Command, DefaultsAction};
use defaults::{describe_paths, enumerate_defaults, sync_defaults, SyncOptions};
use paths::AppPaths;
use state::AppState;

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
    let mut state = bootstrap::bootstrap_filesystem(&paths)?;

    match action {
        DefaultsAction::Sync(args) => run_defaults_sync(&paths, &mut state, args.dry_run),
        DefaultsAction::List => run_defaults_list(&paths),
        DefaultsAction::Where => run_defaults_where(&paths, &state),
    }
}

fn run_defaults_sync(paths: &AppPaths, state: &mut AppState, dry_run: bool) -> Result<()> {
    let previous_version = state.defaults_version.clone();
    let previous_sync = state.last_defaults_sync.clone();
    let report = sync_defaults(paths, state, SyncOptions { dry_run })?;

    if let Some(version) = &report.share_version {
        println!("System defaults version: {version}");
    } else {
        println!("System defaults version: (not provided)");
    }

    if report.copied_assets.is_empty() {
        if dry_run {
            println!("Dry-run: no new defaults would be installed.");
        } else {
            println!("All bundled defaults already installed.");
        }
    } else {
        if dry_run {
            println!("Dry-run: the following defaults would be installed:");
        } else {
            println!("Installed bundled defaults:");
        }

        for copy in &report.copied_assets {
            let category = if copy.source.is_dir() {
                "shader"
            } else if copy.source.extension().and_then(|ext| ext.to_str()) == Some("toml") {
                "playlist"
            } else {
                "asset"
            };
            println!(
                "  {category:<8} {} -> {}",
                copy.source.display(),
                copy.target.display()
            );
        }
    }

    if !dry_run
        && (state.defaults_version != previous_version || state.last_defaults_sync != previous_sync)
    {
        state.persist(&paths.state_file())?;
    }

    Ok(())
}

fn run_defaults_list(paths: &AppPaths) -> Result<()> {
    let entries = enumerate_defaults(paths)?;
    if entries.is_empty() {
        println!(
            "No bundled defaults were found at {}",
            paths.share_dir().display()
        );
        return Ok(());
    }

    println!("Bundled defaults:");
    for entry in entries {
        let category = if entry.source.is_dir() {
            "shader"
        } else if entry.source.extension().and_then(|ext| ext.to_str()) == Some("toml") {
            "playlist"
        } else {
            "asset"
        };
        println!(
            "  {category:<8} {:<28} status={:<9} target={} source={}",
            entry.name,
            if entry.installed {
                "installed"
            } else {
                "missing"
            },
            entry.target.display(),
            entry.source.display()
        );
    }

    Ok(())
}

fn run_defaults_where(paths: &AppPaths, state: &AppState) -> Result<()> {
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
    if let Some(version) = &state.defaults_version {
        println!("Installed defaults version: {version}");
    }
    if let Some(timestamp) = &state.last_defaults_sync {
        println!("Last defaults sync (epoch seconds): {timestamp}");
    }
    Ok(())
}
