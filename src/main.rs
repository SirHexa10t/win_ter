//! The `win_ter` binary. Everything it does is [`win_ter::cli`]; this hands it
//! the process — first to the sandbox's own first step, when this run is one
//! ([`win_ter::serve_filtered_if_asked`]).

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    // That step never comes back here: it becomes Proton's run, or ends the process saying why not.
    if win_ter::serve_filtered_if_asked() {
        return ExitCode::FAILURE;
    }
    ExitCode::from(win_ter::cli::Cli::parse().run())
}
