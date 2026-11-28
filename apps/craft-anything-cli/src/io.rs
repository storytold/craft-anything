//! The two streams a command writes to, so `run` can be called with any writers and a test reads
//! what a command printed.

use crate::output::{self, Format};
use serde_json::Value;
use std::io::Write;

/// Standard output and standard error of one run.
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// Pretty-print JSON: set when standard output is a terminal.
    pub pretty: bool,
}

impl Io<'_> {
    /// A line on stdout. A closed pipe (`| head`) is not an error worth failing over.
    pub fn line(&mut self, s: &str) {
        let _ = self.out.write_all(s.as_bytes()).and_then(|()| self.out.write_all(b"\n")).and_then(|()| self.out.flush());
    }

    /// Text on stdout as it is, with no line end added.
    pub fn raw(&mut self, s: &str) {
        let _ = self.out.write_all(s.as_bytes()).and_then(|()| self.out.flush());
    }

    /// A value in the chosen format, on stdout.
    pub fn emit(&mut self, v: &Value, f: Format) {
        let text = output::render(v, f, self.pretty);
        self.line(&text);
    }

    /// `error: <message>` on stderr.
    pub fn error(&mut self, message: impl std::fmt::Display) {
        let _ = writeln!(self.err, "error: {message}");
    }
}

/// `println!` to the run's stdout.
macro_rules! outln {
    ($io:expr) => {{ $io.line("") }};
    ($io:expr, $($arg:tt)*) => {{ $io.line(&format!($($arg)*)) }};
}

/// `print!` to the run's stdout.
macro_rules! outp {
    ($io:expr, $($arg:tt)*) => {{ $io.raw(&format!($($arg)*)) }};
}

/// `eprintln!` to the run's stderr.
macro_rules! errln {
    ($io:expr) => {{ let _ = std::io::Write::write_all($io.err, b"\n"); }};
    ($io:expr, $($arg:tt)*) => {{ let _ = writeln!($io.err, $($arg)*); }};
}

/// `eprint!` to the run's stderr, flushed so a prompt shows before the answer is read.
macro_rules! errp {
    ($io:expr, $($arg:tt)*) => {{ let _ = write!($io.err, $($arg)*); let _ = $io.err.flush(); }};
}
