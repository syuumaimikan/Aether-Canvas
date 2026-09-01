//! Command line handling.
//!
//! Deliberately tiny: the application is a GUI, and the only argument it needs
//! is an optional file to open. Keeping this hand-rolled avoids pulling an
//! argument parser into the desktop binary for three flags.

use std::path::PathBuf;

/// What the command line asked for.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cli {
    /// A project or image to open at startup.
    pub open: Option<PathBuf>,
    /// Print help and exit.
    pub help: bool,
    /// Print the version and exit.
    pub version: bool,
    /// Arguments that were not understood.
    pub unknown: Vec<String>,
}

/// Text shown by `--help`.
pub const USAGE: &str = "\
Aether Canvas — integrated 2D creative environment

USAGE:
    aether-canvas [OPTIONS] [FILE]

ARGS:
    <FILE>    A .aether project, or an image to import

OPTIONS:
    -h, --help       Print this help
    -V, --version    Print version information
";

/// Parse arguments (excluding the executable name).
pub fn parse<I, S>(args: I) -> Cli
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut cli = Cli::default();
    for arg in args {
        let arg = arg.as_ref();
        match arg {
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            other if other.starts_with('-') => cli.unknown.push(other.to_string()),
            other => {
                if cli.open.is_none() {
                    cli.open = Some(PathBuf::from(other));
                } else {
                    cli.unknown.push(other.to_string());
                }
            }
        }
    }
    cli
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_arguments_opens_a_blank_document() {
        assert_eq!(parse(Vec::<String>::new()), Cli::default());
    }

    #[test]
    fn a_path_is_taken_as_the_file_to_open() {
        let cli = parse(["drawing.aether"]);
        assert_eq!(cli.open, Some(PathBuf::from("drawing.aether")));
        assert!(cli.unknown.is_empty());
    }

    #[test]
    fn help_and_version_flags_are_recognised() {
        assert!(parse(["--help"]).help);
        assert!(parse(["-h"]).help);
        assert!(parse(["-V"]).version);
    }

    #[test]
    fn unknown_flags_are_collected_rather_than_ignored() {
        let cli = parse(["--frobnicate", "a.png", "b.png"]);
        assert_eq!(cli.unknown, vec!["--frobnicate".to_string(), "b.png".to_string()]);
        assert_eq!(cli.open, Some(PathBuf::from("a.png")));
    }
}
