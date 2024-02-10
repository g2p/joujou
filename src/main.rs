mod cli;

fn main() {
    let app = cli::parse_cli();
    match app.cmd {
        cli::Command::Play { path } => println!("path {}", path.display()),
    }
}
