mod bindings;
mod bootstrap;
mod cli;
mod multi;
mod paths;
mod run;
mod state;

use anyhow::Result;

fn main() -> Result<()> {
    let args = cli::parse();
    run::run(args)
}
