use std::ffi::OsStr;
use std::path::PathBuf;

use rust_cast::channels::media::MusicTrackMediaMetadata;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta;
use symphonia::core::meta::MetadataReader as _;
use symphonia::default::formats::{FlacReader, OggReader};

#[derive(Debug)]
pub struct Metadata {
    // in rust_cast format
    pub cast_metadata: MusicTrackMediaMetadata,
    // still in Symphonia format
    pub visual: Option<meta::Visual>,
}

#[derive(Debug)]
pub struct AudioFile {
    pub path: PathBuf,
    pub mime_type: &'static str,
    pub metadata: Option<Metadata>,
}

impl AudioFile {
    /// Load known audio files (based on extension)
    /// Ok(None) if not a known extension
    /// Err if a known extension but parsing failed
    pub fn load_if_supported(path: PathBuf) -> anyhow::Result<Option<Self>> {
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or_default();
        if let Some(ckind) = ContainerKind::from_ext(ext) {
            let mime_type = ckind.mime_type();
            let metadata = read_metadata(&path, ckind)?;
            Ok(Some(AudioFile {
                path,
                mime_type,
                metadata,
            }))
        } else {
            Ok(None)
        }
    }
}

fn string_value(tag: &meta::Tag) -> Option<String> {
    if let meta::Value::String(ref str) = tag.value {
        Some(str.to_owned())
    } else {
        None
    }
}

fn u32_value(tag: &meta::Tag) -> Option<u32> {
    if let meta::Value::UnsignedInt(unum) = tag.value {
        unum.try_into().ok()
    } else {
        None
    }
}

/*fn compare_visuals(v1: &meta::Visual, v2: &meta::Visual) -> Ordering {
    // Make best lowest
    match (v1.usage, v2.usage) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(u1), Some(u2)) => compare_visual_keys(u1, u2),
    }
}

fn compare_visual_keys(u1: meta::StandardVisualKey, u2: meta::StandardVisualKey) -> Ordering {
    // Make best lowest
    todo!()
}*/

// converts tags to rust cast format, keeps visuals in Symphonia
// format until a URL can be built to serve them
fn convert_metadata(meta: &meta::MetadataRevision) -> Metadata {
    use symphonia::core::meta::StandardTagKey::*;
    let mut cmeta = MusicTrackMediaMetadata::default();
    for tag in meta.tags() {
        let Some(stdtag) = tag.std_key else { continue };
        match stdtag {
            Album => cmeta.album_name = string_value(tag),
            TrackTitle => cmeta.title = string_value(tag),
            AlbumArtist => cmeta.album_artist = string_value(tag),
            Artist => cmeta.artist = string_value(tag),
            Composer => cmeta.composer = string_value(tag),
            TrackNumber => cmeta.track_number = u32_value(tag),
            DiscNumber => cmeta.disc_number = u32_value(tag),
            ReleaseDate => cmeta.release_date = string_value(tag),
            _ => (),
        }
    }

    // using min_by because it will return the first of equal elements
    /*let visual = meta
    .visuals()
    .iter()
    .min_by(|a, b| compare_visuals(*a, *b))
    .cloned();*/
    // First seems good enough, ordering would require experimentation
    let visual = meta.visuals().first().cloned();

    Metadata {
        cast_metadata: cmeta,
        visual,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Flac,
    Ogg,
    Mp3,
}

impl ContainerKind {
    fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "flac" => Some(Self::Flac),
            "ogg" | "oga" | "opus" => Some(Self::Ogg),
            "mp3" => Some(Self::Mp3),
            // mp4 metadata for aac? meh
            // wav? only if metadata can be made to work
            _ => None,
        }
    }

    fn mime_type(self) -> &'static str {
        match self {
            ContainerKind::Flac => "audio/flac",
            ContainerKind::Ogg => "audio/ogg",
            ContainerKind::Mp3 => "audio/mpeg",
        }
    }
}

fn read_metadata(
    path: &std::path::Path,
    container_kind: ContainerKind,
) -> anyhow::Result<Option<Metadata>> {
    let src = std::fs::File::open(path)?;
    // Default options for buffering
    let mut mss = MediaSourceStream::new(Box::new(src), Default::default());

    let mut reader: Box<dyn FormatReader>;
    match container_kind {
        // For Mp3 metadata we just require id3v2, which is a container
        // around the mp3 file.  id3v1 would be 128 bytes tacked on after
        // the mp3 frames and immediately before EOF, can't really be
        // detected unambiguously.
        ContainerKind::Mp3 => {
            let mut mreader = symphonia_metadata::id3v2::Id3v2Reader::new(&Default::default());
            let meta = mreader.read_all(&mut mss)?;
            return Ok(Some(convert_metadata(&meta)));
        }
        // Don't use the probe system, which currently ignores the extension hint
        // build a reader directly
        ContainerKind::Flac => reader = Box::new(FlacReader::try_new(mss, &Default::default())?),
        ContainerKind::Ogg => reader = Box::new(OggReader::try_new(mss, &Default::default())?),
    }

    let meta = reader.metadata();
    let Some(meta) = meta.current() else {
        return Ok(None);
    };

    Ok(Some(convert_metadata(meta)))
}
