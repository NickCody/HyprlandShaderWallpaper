mod bindings;
mod bootstrap;
mod cli;
mod multi;
mod run;

use anyhow::Result;

fn main() -> Result<()> {
    let args = cli::parse();
    run::run(args)
}
