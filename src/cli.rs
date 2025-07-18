// Picked bpaf:
// syn-free and fast to compile (not using the optional derive),
// combinator-based, more gnu-compliant than xflags and pico-args,
// higher-level than lexopt, less ad-hoc than clap_lex
// https://github.com/rosetta-rs/argparse-rosetta-rs

use std::fmt::Display;
use std::num::{NonZeroU16, ParseIntError};
use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::str::FromStr;

use bpaf::{construct, OptionParser, Parser};

#[derive(Debug, Clone)]
pub enum Command {
    Play {
        paths: Vec<PathBuf>,
        playlist_start: NonZeroU16,
    },
    Listen,
}

#[derive(Debug, Clone)]
pub enum PortOrRange {
    RandomPort,
    SinglePort(NonZeroU16),
    Range(RangeInclusive<NonZeroU16>),
}

impl IntoIterator for PortOrRange {
    type Item = u16;

    type IntoIter = RangeInclusive<u16>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::RandomPort => 0..=0,
            Self::SinglePort(port) => port.get()..=port.get(),
            Self::Range(inclrange) => inclrange.start().get()..=inclrange.end().get(),
        }
    }
}

pub enum RangeParseError {
    BadInt(ParseIntError),
    EmptyRange,
}

impl From<ParseIntError> for RangeParseError {
    fn from(value: ParseIntError) -> Self {
        Self::BadInt(value)
    }
}

impl Display for RangeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadInt(e) => e.fmt(f),
            Self::EmptyRange => write!(f, "Empty range (start must be <= end)"),
        }
    }
}

impl FromStr for PortOrRange {
    type Err = RangeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // No input will parse to Self::RandomPort, omit the flag for that
        match s.split_once(':') {
            Some((start, end)) => {
                let start = start.parse()?;
                let end = end.parse()?;
                if start <= end {
                    Ok(Self::Range(start..=end))
                } else {
                    Err(RangeParseError::EmptyRange)
                }
            }
            None => Ok(Self::SinglePort(s.parse()?)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct App {
    pub port: PortOrRange,
    pub beets_db: Option<PathBuf>,
    pub cmd: Command,
}

fn play_command() -> OptionParser<Command> {
    let playlist_start = bpaf::long("playlist-start")
        .help("Start playing at INDEX (not necessarily the first track)")
        .argument("INDEX")
        .fallback(NonZeroU16::MIN);
    // Should we validate for files/directories early on?
    // Directories are only handled if there is a single positional arg
    // If passed a list of files, should we accept covers within them?
    // In which case they might apply to all later entries?
    let paths = bpaf::positional::<PathBuf>("path")
        .help("Paths to play (either a directory or a list of music files)")
        .some("Need at least one path to play");

    construct!(Command::Play {
        playlist_start,
        paths,
    })
    .to_options()
    .descr("Cast a music directory to a Chromecast device")
}

fn listen_command() -> OptionParser<Command> {
    bpaf::pure(Command::Listen)
        .to_options()
        .descr("Listen to events from the Chromecast device")
}

fn parser() -> OptionParser<App> {
    // Subcommands
    let play_cmd = play_command()
        .command("play")
        .help("Cast a music directory to a Chromecast device");
    let listen_cmd = listen_command()
        .command("listen")
        .help("Listen to events (playback…) from the Chromecast device");

    // Common arguments (use a basic-toml conffile at some point)
    let port = bpaf::long("port")
        .help(
            "Port to listen on, can be picked within a range.\n \
            Please ensure your local network can access it.",
        )
        .argument("PORT[:PORT]")
        .fallback(PortOrRange::RandomPort);
    let beets_db = bpaf::long("beets-db")
        .help(
            "Path to beets library.db.\n \
            Tracks that match a path within the beets library will be cast \
            with metadata from the library",
        )
        .argument("PATH")
        .optional();
    let cmd = construct!([play_cmd, listen_cmd]);
    construct!(App {
        port,
        beets_db,
        cmd
    })
    .to_options()
}

pub fn parse_cli() -> App {
    parser().run()
}

#[test]
fn check_options() {
    parser().check_invariants(false)
}
