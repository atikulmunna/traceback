use clap::Parser;
use traceback_cli::{Cli, run};

fn main() {
    run(Cli::parse());
}
