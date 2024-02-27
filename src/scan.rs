use std::cmp::{Ordering, Reverse};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::audio::AudioFile;

pub struct Playlist {
    pub cover: Option<PathBuf>,
    pub entries: Vec<AudioFile>,
}

/// List music files, sort them appropriately, build the queue/playlist
pub fn dir_to_playlist(path: &Path, beets_db: Option<&Path>) -> anyhow::Result<Playlist> {
    let mut entries = Vec::new();
    let mut cover: Option<PathBuf> = None;

    let beets_db = if let Some(beets_db) = beets_db {
        use rusqlite::OpenFlags;
        Some(rusqlite::Connection::open_with_flags(
            beets_db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_EXRESCODE,
        )?)
    } else {
        None
    };

    for dent in walkdir::WalkDir::new(path)
        .same_file_system(true)
        .into_iter()
        .filter_entry(|dent| {
            if dent.file_name().as_bytes().starts_with(b".") {
                return false;
            };
            true
        })
    {
        let dent = dent?;
        // If we don't want symlinks we could filter them out in both
        // places we open files (for metadata and from the http server).
        // If we want them, we could be configured with a realpath
        // whitelist.
        //if dent.file_type().is_file() {
        if !dent.file_type().is_dir() {
            let path = dent.into_path();
            let Some(ext) = path.extension().and_then(OsStr::to_str) else {
                continue;
            };
            if matches!(ext, "jpg" | "jpeg" | "png") {
                if let Some(ref c0) = cover {
                    if compare_covers(c0, &path) == Ordering::Less {
                        log::info!("Preferring cover {} to {}", path.display(), c0.display());
                        cover = Some(path)
                    }
                } else {
                    cover = Some(path)
                }
            } else if let Some(af) = AudioFile::load_if_supported(path, beets_db.as_ref())? {
                entries.push(af);
            }
        }
    }
    entries.sort_by(|a, b| {
        natord::compare(&a.path.to_string_lossy(), &b.path.to_string_lossy())
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(Playlist { cover, entries })
}

fn cover_score(path: &Path) -> impl Ord {
    // Other options to consider: art album folder
    const KNOWN_STEMS: &[&str; 4] = &["cover", "front", "00 - cover", "front cover"];
    if let Some(stem) = path.file_stem().and_then(OsStr::to_str) {
        if let Some(pos) = KNOWN_STEMS
            .iter()
            .position(|e| e == &stem.to_ascii_lowercase())
        {
            // Lower pos -> higher score
            return Reverse(pos);
        }
    }
    // Lowest possible score
    Reverse(usize::MAX)
}

fn compare_covers(c0: &Path, c1: &Path) -> Ordering {
    cover_score(c0).cmp(&cover_score(c1))
}
