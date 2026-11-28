//! `craft-anything-cli`: every tool as a subcommand, the MCP server, setup, doctor and headless chat.
//!
//! This file only wires the process: the command line, the two streams and Ctrl-C go to
//! `craft_anything_cli::run`, and the exit code it returns is the process's. Exit codes: 0 success,
//! 1 the command failed (a structured error is printed), 2 usage error, 130 interrupted.
use std::sync::atomic::Ordering;

fn main() {
    let _ = ctrlc::set_handler(|| {
        if craft_anything_cli::INTERRUPTED.swap(true, Ordering::SeqCst) {
            // second Ctrl-C: leave now
            std::process::exit(130);
        }
    });
    let code = craft_anything_cli::run(std::env::args_os(), &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
