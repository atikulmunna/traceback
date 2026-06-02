use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "traceback")]
#[command(about = "Explainable backup and restore tool")]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a backup repository.
    Init {
        /// Repository directory to create.
        repo: PathBuf,
    },
    /// Create a snapshot from one or more source paths.
    Backup {
        /// Files or directories to include in the snapshot.
        #[arg(required = true)]
        paths: Vec<PathBuf>,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// List published snapshots.
    Snapshots {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
    /// Restore a snapshot or a path inside a snapshot.
    Restore {
        /// Snapshot ID or snapshot path expression.
        snapshot: String,

        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,

        /// Directory or file path to restore into.
        #[arg(long)]
        target: PathBuf,
    },
    /// Verify repository integrity.
    Check {
        /// Backup repository directory.
        #[arg(long)]
        repo: PathBuf,
    },
}

pub fn run(cli: Cli) {
    match cli.command {
        Command::Init { repo } => {
            println!(
                "Repository initialization is not implemented yet: {}",
                repo.display()
            );
        }
        Command::Backup { paths, repo } => {
            println!(
                "Backup is not implemented yet: {} source path(s) -> {}",
                paths.len(),
                repo.display()
            );
        }
        Command::Snapshots { repo } => {
            println!(
                "Snapshot listing is not implemented yet: {}",
                repo.display()
            );
        }
        Command::Restore {
            snapshot,
            repo,
            target,
        } => {
            println!(
                "Restore is not implemented yet: {snapshot} from {} -> {}",
                repo.display(),
                target.display()
            );
        }
        Command::Check { repo } => {
            println!(
                "Repository check is not implemented yet: {}",
                repo.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    use super::{Cli, Command};

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_each_core_command() {
        let cases = [
            vec!["traceback", "init", "./repo"],
            vec!["traceback", "backup", "./source", "--repo", "./repo"],
            vec!["traceback", "snapshots", "--repo", "./repo"],
            vec![
                "traceback",
                "restore",
                "snap_001",
                "--repo",
                "./repo",
                "--target",
                "./restored",
            ],
            vec!["traceback", "check", "--repo", "./repo"],
        ];

        for args in cases {
            Cli::try_parse_from(args).expect("core command should parse");
        }
    }

    #[test]
    fn backup_requires_at_least_one_source_path() {
        let error = Cli::try_parse_from(["traceback", "backup", "--repo", "./repo"])
            .expect_err("backup without a source path should fail");

        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn command_variants_remain_available() {
        let cli = Cli::parse_from(["traceback", "check", "--repo", "./repo"]);

        assert!(matches!(cli.command, Command::Check { .. }));
    }
}
