//! The `proton_sandboxed` binary. Everything it does is [`proton_sandboxed::cli`]; this hands it
//! the process — first to the sandbox's own first step, when this run is one
//! ([`proton_sandboxed::serve_filtered_if_asked`]).

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    // That step never comes back here: it becomes Proton's run, or ends the process saying why not.
    if proton_sandboxed::serve_filtered_if_asked() {
        return ExitCode::FAILURE;
    }
    ExitCode::from(proton_sandboxed::cli::Cli::parse().run())
}
