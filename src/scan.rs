use std::cmp::{Ordering, Reverse};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::audio::AudioFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CoverKind {
    Jpeg,
    Png,
}

impl CoverKind {
    fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "jpeg" | "jpg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }

    const fn mime_type(self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
        }
    }
}

pub struct CoverFile {
    pub path: PathBuf,
    pub mime_type: &'static str,
}

pub struct Playlist {
    pub cover: Option<CoverFile>,
    pub entries: Vec<AudioFile>,
}

/// List music files, sort them appropriately, build the queue/playlist
pub fn dir_to_playlist(path: &Path, beets_db: Option<&Path>) -> anyhow::Result<Playlist> {
    let mut entries = Vec::new();
    let mut cover: Option<CoverFile> = None;
    let mut coverscore = None;

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
            let ext = ext.to_ascii_lowercase();
            let ext = ext.as_str();
            if let Some(ckind) = CoverKind::from_ext(ext) {
                let cover1 = CoverFile {
                    path: path.clone(),
                    mime_type: ckind.mime_type(),
                };
                if let Some(ref c0) = cover {
                    let sc0 = coverscore.get_or_insert_with(|| cover_score(&c0.path));
                    let sc1 = cover_score(&path);
                    if sc1.cmp(sc0) == Ordering::Greater {
                        log::info!(
                            "Preferring cover {} to {}",
                            path.display(),
                            c0.path.display()
                        );
                        cover = Some(cover1);
                        coverscore = Some(sc1);
                    }
                } else {
                    cover = Some(cover1);
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
        let stem_lcase = stem.to_ascii_lowercase();
        if let Some(pos) = KNOWN_STEMS.iter().position(|e| e == &stem_lcase) {
            // Lower pos -> higher score
            return Reverse(pos);
        }
    }
    // Lowest possible score
    Reverse(usize::MAX)
}
