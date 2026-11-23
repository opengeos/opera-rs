//! Command-line interface and MCP server for `opera-rs`.

use std::ffi::OsString;

use clap::Parser;

/// Command-line interface for NASA OPERA search, visualization, and analysis.
#[derive(Debug, Parser)]
#[command(
    name = "opera",
    version,
    about = "Search, visualize, and analyze NASA OPERA satellite data"
)]
pub struct Cli {}

/// Runs the CLI with the given arguments and returns the process exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(_) => {
            println!("opera-rs {}", opera_rs::VERSION);
            0
        }
        Err(err) => {
            let code = err.exit_code();
            let _ = err.print();
            code
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_invocation() {
        assert_eq!(run(["opera"]), 0);
    }
}
