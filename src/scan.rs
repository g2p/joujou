use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use crate::audio::AudioFile;

pub struct Playlist {
    pub cover: Option<PathBuf>,
    pub items: Vec<AudioFile>,
}

/// List music files, sort them appropriately, build the queue/playlist
pub fn dir_to_playlist(path: &std::path::Path) -> anyhow::Result<Playlist> {
    let mut items = Vec::new();
    //let mut covers = Vec::new();

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
        if dent.file_type().is_file() {
            let path = dent.into_path();
            if let Some(af) = AudioFile::load_if_supported(path)? {
                items.push(af);
            }
        }
    }
    items.sort_by(|a, b| {
        natord::compare(&a.path.to_string_lossy(), &b.path.to_string_lossy())
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(Playlist { cover: None, items })
}
