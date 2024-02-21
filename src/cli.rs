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
        path: PathBuf,
        playlist_start: NonZeroU16,
    },
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
            PortOrRange::SinglePort(port) => port.get()..=port.get(),
            PortOrRange::Range(inclrange) => inclrange.start().get()..=inclrange.end().get(),
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
    pub cmd: Command,
}

fn play_command() -> OptionParser<Command> {
    let playlist_start = bpaf::long("playlist-start")
        .argument("INDEX")
        .help("Start playing at INDEX (not necessarily the first track)")
        .fallback(NonZeroU16::MIN);
    let path = bpaf::positional::<PathBuf>("path").help("Directory path to play");

    construct!(Command::Play {
        playlist_start,
        path,
    })
    .to_options()
    .descr("Cast a directory to chromecast audio")
}

fn parser() -> OptionParser<App> {
    // Subcommands
    let play_cmd = play_command()
        .command("play")
        .help("Cast a directory to chromecast audio");

    let port = bpaf::long("port")
        .argument("PORT[:PORT]")
        .help(
            "Port to listen on, can be picked within a range.\n \
            Please ensure your local network can access it.",
        )
        .fallback(PortOrRange::RandomPort);
    let cmd = construct!([play_cmd]);
    construct!(App { port, cmd }).to_options()
}

pub fn parse_cli() -> App {
    parser().run()
}

#[test]
fn check_options() {
    parser().check_invariants(false)
}
