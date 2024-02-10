use std::path::Path;

mod cli;

fn play(path: &Path) -> anyhow::Result<()> {
    println!("path {}", path.display());
    // List music files beforehand, sort them appropriately,
    // build the queue/playlist.
    // natord could work with OsStr; human_sort can't
    // uutils has src/uucore/src/lib/features/version_cmp.rs
    // which mimics gnu version sort (with deliberate divergence due to bugs in GNU?)
    // uutils doesn't handle non-unicode though.
    for entry in walkdir::WalkDir::new(path).sort_by(|a, b| {
        natord::compare(
            &a.file_name().to_string_lossy(),
            &b.file_name().to_string_lossy(),
        )
    }) {
        println!("{}", entry?.path().display());
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let app = cli::parse_cli();
    match app.cmd {
        cli::Command::Play { path } => play(&path),
    }
}
