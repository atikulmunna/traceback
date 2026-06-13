use std::process::ExitCode;

use clap::Parser;
use serde::Serialize;
use traceback_cli::{Cli, error_code, run};

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'static str,
    message: &'a str,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json {
                let message = error.to_string();
                let envelope = ErrorEnvelope {
                    error: ErrorBody {
                        code: error_code(error.as_ref()),
                        message: &message,
                    },
                };
                eprintln!(
                    "{}",
                    serde_json::to_string(&envelope).expect("error envelope should serialize")
                );
            } else {
                eprintln!("Error: {error}");
            }
            ExitCode::FAILURE
        }
    }
}
