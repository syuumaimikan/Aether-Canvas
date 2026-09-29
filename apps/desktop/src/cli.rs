//! Command line handling.
//!
//! Deliberately tiny: the application is a GUI, and mostly needs an optional
//! file to open. Keeping this hand-rolled avoids pulling an argument parser
//! into the desktop binary for a handful of flags.

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
    /// Export the file as a runtime model into this directory, without
    /// opening a window.
    pub export_model: Option<PathBuf>,
    /// Export the file as a Live2D Cubism model into this directory,
    /// without opening a window.
    pub export_live2d: Option<PathBuf>,
    /// Rig the file from its layer names before exporting.
    pub auto_rig: bool,
    /// List what the folder or archive holds that can be opened, and exit.
    pub list: bool,
    /// Arguments that were not understood.
    pub unknown: Vec<String>,
}

/// Text shown by `--help`.
pub const USAGE: &str = "\
Aether Canvas — integrated 2D creative environment

USAGE:
    aether-canvas [OPTIONS] [FILE]

ARGS:
    <FILE>    A .aether project, a layered .psd, a Live2D .model3.json,
              or an image to import; or a .zip holding them — name the
              one to open as pack.zip#path/inside (see --list)

OPTIONS:
        --list [FOLDER|ZIP]     List what a folder of samples or an archive
                                holds that can be opened (by default the
                                assets_sample folder), then exit
        --export-model <DIR>    Write FILE as a runtime model (model.json and
                                texture atlases) for games and the web player,
                                then exit without opening a window
        --export-live2d <DIR>   Write FILE as a Live2D Cubism model (.moc3,
                                .model3.json, physics, motions...) for VTube
                                Studio and the Cubism SDKs, then exit
        --auto-rig              With an export: rig FILE from its layer
                                names first (for a PSD straight from a
                                painting app)
    -h, --help                  Print this help
    -V, --version               Print version information

EXAMPLE:
    aether-canvas --auto-rig --export-model web/model character.psd
    aether-canvas --auto-rig --export-live2d vtuber character.psd
    aether-canvas --list assets_sample
    aether-canvas \"assets_sample/haru.zip#runtime/haru.model3.json\"
";

/// Parse arguments (excluding the executable name).
pub fn parse<I, S>(args: I) -> Cli
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut cli = Cli::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let arg = arg.as_ref();
        match arg {
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            "--auto-rig" => cli.auto_rig = true,
            "--list" => cli.list = true,
            "--export-model" => match args.next() {
                Some(dir) => cli.export_model = Some(PathBuf::from(dir.as_ref())),
                None => cli.unknown.push("--export-model (needs a directory)".to_string()),
            },
            other if other.starts_with("--export-model=") => {
                cli.export_model = Some(PathBuf::from(&other["--export-model=".len()..]));
            }
            "--export-live2d" => match args.next() {
                Some(dir) => cli.export_live2d = Some(PathBuf::from(dir.as_ref())),
                None => cli
                    .unknown
                    .push("--export-live2d (needs a directory)".to_string()),
            },
            other if other.starts_with("--export-live2d=") => {
                cli.export_live2d = Some(PathBuf::from(&other["--export-live2d=".len()..]));
            }
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

    #[test]
    fn model_export_takes_a_directory() {
        let cli = parse(["--auto-rig", "--export-model", "out/model", "character.psd"]);
        assert!(cli.auto_rig);
        assert_eq!(cli.export_model, Some(PathBuf::from("out/model")));
        assert_eq!(cli.open, Some(PathBuf::from("character.psd")));
        assert!(cli.unknown.is_empty());

        let cli = parse(["--export-model=web", "a.aether"]);
        assert_eq!(cli.export_model, Some(PathBuf::from("web")));
        assert_eq!(parse(["--export-model"]).unknown.len(), 1);
        let cli = parse(["--auto-rig", "--export-live2d", "vtuber", "character.psd"]);
        assert_eq!(cli.export_live2d, Some(PathBuf::from("vtuber")));
        assert!(cli.auto_rig);
        assert_eq!(
            parse(["--export-live2d=out", "a.aether"]).export_live2d,
            Some(PathBuf::from("out"))
        );
        assert_eq!(parse(["--export-live2d"]).unknown.len(), 1);
    }

    #[test]
    fn list_takes_an_optional_folder_or_archive() {
        let cli = parse(["--list"]);
        assert!(cli.list && cli.open.is_none());
        let cli = parse(["--list", "assets_sample/haru.zip"]);
        assert_eq!(cli.open, Some(PathBuf::from("assets_sample/haru.zip")));
        let cli = parse(["pack.zip#runtime/haru.model3.json"]);
        assert_eq!(cli.open, Some(PathBuf::from("pack.zip#runtime/haru.model3.json")));
    }
}
