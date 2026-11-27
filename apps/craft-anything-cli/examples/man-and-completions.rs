//! Writes craft-anything-cli's shell completions ([`clap_complete`]: bash, zsh, fish, elvish,
//! PowerShell) and man pages ([`clap_mangen`]: `craft-anything-cli.1` and one per subcommand, such as
//! `craft-anything-cli-setup.1`) into a directory. The Linux and FreeBSD packages ship the bash, zsh
//! and fish completions and the man pages (`packaging/linux/package.sh`,
//! `packaging/freebsd/package.sh`):
//!
//! ```sh
//! cargo run -p craft-anything-cli --example man-and-completions -- <out-dir>
//! ```
//!
//! Tools are not subcommands in the clap definition (`craft-anything-cli commands` lists them), so
//! the pages and completions cover the built-in commands and the global options.

use std::path::Path;
use std::process::ExitCode;

use clap::ValueEnum;
use clap_complete::Shell;

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let (Some(dir), None) = (args.next(), args.next()) else {
        eprintln!("usage: man-and-completions <out-dir>");
        return ExitCode::from(2);
    };
    let dir = Path::new(&dir);
    match generate(dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {}: {e}", dir.display());
            ExitCode::FAILURE
        }
    }
}

/// Also run by the tests.
pub(crate) fn generate(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut cmd = craft_anything_cli::command();
    for &shell in Shell::value_variants() {
        clap_complete::generate_to(shell, &mut cmd, "craft-anything-cli", dir)?;
    }
    // A fresh command: `clap_complete::generate_to` built `cmd`, adding the `help` subcommand,
    // which `clap_mangen` would otherwise give pages of its own (`craft-anything-cli-help-setup.1`).
    clap_mangen::generate_to(craft_anything_cli::command(), dir)
}
