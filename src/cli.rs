// Picked bpaf:
// syn-free and fast to compile (not using the optional derive),
// combinator-based, more gnu-compliant than xflags and pico-args,
// higher-level than lexopt, less ad-hoc than clap_lex
// https://github.com/rosetta-rs/argparse-rosetta-rs

use std::path::PathBuf;

use bpaf::{construct, OptionParser, Parser};

#[derive(Debug, Clone)]
pub enum Command {
    Play { path: PathBuf },
}

#[derive(Debug, Clone)]
pub struct App {
    pub cmd: Command,
}

fn play_command() -> OptionParser<Command> {
    let path = bpaf::positional::<PathBuf>("path").help("Directory path to play");

    construct!(Command::Play { path })
        .to_options()
        .descr("Cast a directory to chromecast audio")
}

fn parser() -> OptionParser<App> {
    // Subcommands
    let play_cmd = play_command()
        .command("play")
        .help("Cast a directory to chromecast audio");

    let cmd = construct!([play_cmd]);
    construct!(App { cmd }).to_options()
}

pub fn parse_cli() -> App {
    parser().run()
}

#[test]
fn check_options() {
    parser().check_invariants(false)
}
