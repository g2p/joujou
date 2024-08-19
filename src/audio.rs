use std::ffi::OsStr;
use std::io::{Seek, SeekFrom};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use rust_cast::channels::media::MusicTrackMediaMetadata;
use symphonia::core::codecs;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta;
use symphonia::core::meta::MetadataReader as _;
use symphonia::default::formats::{FlacReader, IsoMp4Reader, MkvReader, MpaReader, OggReader};

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
    pub fn load(path: PathBuf, beets_db: Option<&rusqlite::Connection>) -> anyhow::Result<Self> {
        if let Some(r) = Self::load_if_supported(path, beets_db)? {
            Ok(r)
        } else {
            Err(symphonia::core::errors::Error::Unsupported("Not a known extension").into())
        }
    }

    /// Load known audio files (based on extension)
    /// Ok(None) if not a known extension
    /// Err if a known extension but parsing failed
    pub fn load_if_supported(
        path: PathBuf,
        beets_db: Option<&rusqlite::Connection>,
    ) -> anyhow::Result<Option<Self>> {
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or_default();
        if let Some(ckind) = ContainerKind::from_ext(ext) {
            let mime_type = ckind.mime_type();
            let mut metadata = read_metadata(&path, ckind)?;
            if let Some(beets_db) = beets_db {
                // We still call read_metadata above while discarding
                // successful results, it validates codecs.
                // Also, we might want to merge metadata, maybe
                // pick up attached visuals when they aren't in the
                // beets db.
                if let Some(beets_meta) = beets_metadata(beets_db, &path)? {
                    metadata = Some(beets_meta);
                }
            }
            Ok(Some(Self {
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

// converts tags to rust cast format, keeps visuals in Symphonia
// format until a URL can be built to serve them
// Maps between
// https://docs.rs/symphonia-core/latest/symphonia_core/meta/enum.StandardTagKey.html
// https://developers.google.com/cast/docs/media/messages#MusicTrackMediaMetadata
fn convert_metadata(meta: &meta::MetadataRevision) -> Metadata {
    use symphonia::core::meta::StandardTagKey::*;
    let mut cmeta = MusicTrackMediaMetadata::default();
    // XXX for multi-valued tags, last one will win
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
    Matroska,
    Mp3,
    Mp4,
}

impl ContainerKind {
    fn from_ext(ext: &str) -> Option<Self> {
        match ext {
            "flac" => Some(Self::Flac),
            "ogg" | "oga" | "opus" => Some(Self::Ogg),
            "mka" => Some(Self::Matroska),
            "mp3" => Some(Self::Mp3),
            // mp4 metadata for aac? meh
            // Also the m4a extension is shared with ALAC, a pointless format the Chromecast won't handle
            "m4a" => Some(Self::Mp4),
            // wav? only if metadata can be made to work
            _ => None,
        }
    }

    const fn mime_type(self) -> &'static str {
        match self {
            Self::Flac => "audio/flac",
            Self::Ogg => "audio/ogg",
            Self::Matroska => "audio/webm",
            Self::Mp3 => "audio/mpeg",
            Self::Mp4 => "audio/m4a",
        }
    }
}

fn read_metadata(path: &Path, container_kind: ContainerKind) -> anyhow::Result<Option<Metadata>> {
    let src = std::fs::File::open(path)?;
    // Default options for buffering
    let mut mss = MediaSourceStream::new(Box::new(src), Default::default());

    // Don't use the probe system, which currently ignores the extension hint
    // build a reader directly
    let mut reader: Box<dyn FormatReader> = match container_kind {
        // For Mp3 metadata we just require id3v2, which is a container
        // around the mp3 file.  id3v1 would be 128 bytes tacked on after
        // the mp3 frames and immediately before EOF, can't really be
        // detected unambiguously.
        ContainerKind::Mp3 => {
            let mut mreader = symphonia_metadata::id3v2::Id3v2Reader::new(&Default::default());
            match mreader.read_all(&mut mss) {
                Ok(meta) => return Ok(Some(convert_metadata(&meta))),
                Err(err) => {
                    if !matches!(err, symphonia::core::errors::Error::Unsupported(_)) {
                        return Err(err.into());
                    }
                }
            }
            log::warn!("{} does not start with ID3v2 frames", path.display());
            // This just validates this is an MPEG stream
            let reader = MpaReader::try_new(mss, &Default::default())?;
            let mut mss = Box::new(reader).into_inner();
            mss.seek(SeekFrom::End(-128))?;
            let mut meta = meta::MetadataBuilder::new();
            symphonia_metadata::id3v1::read_id3v1(&mut mss, &mut meta)?;
            return Ok(Some(convert_metadata(&meta.metadata())));
        }
        ContainerKind::Flac => Box::new(FlacReader::try_new(mss, &Default::default())?),
        ContainerKind::Ogg => Box::new(OggReader::try_new(mss, &Default::default())?),
        ContainerKind::Matroska => Box::new(MkvReader::try_new(mss, &Default::default())?),
        ContainerKind::Mp4 => Box::new(IsoMp4Reader::try_new(mss, &Default::default())?),
    };

    validate_codecs(&*reader, container_kind)?;

    let meta = reader.metadata();
    let Some(meta) = meta.current() else {
        return Ok(None);
    };

    Ok(Some(convert_metadata(meta)))
}

fn beets_metadata(
    beets_db: &rusqlite::Connection,
    path: &Path,
) -> anyhow::Result<Option<Metadata>> {
    let mut stmt = beets_db.prepare_cached(
        "SELECT album, title, albumartist, artist, composer, \
        track, disc, year, month, day \
        FROM items WHERE path = ?1",
    )?;
    Ok(stmt
        .query_row([path.as_os_str().as_bytes()], |row| {
            log::info!("Row {row:?}");
            let release_date = Some(format!(
                "{}-{}-{}",
                row.get_unwrap::<usize, u16>(7),
                row.get_unwrap::<usize, u16>(8),
                row.get_unwrap::<usize, u16>(9),
            ));
            // Assuming beets has fetchart enabled with default settings,
            // we don't need to do anything for images,
            // they will be in cover.jpg which we autodetect.
            Ok(Metadata {
                cast_metadata: MusicTrackMediaMetadata {
                    album_name: row.get_unwrap(0),
                    title: row.get_unwrap(1),
                    album_artist: row.get_unwrap(2),
                    artist: row.get_unwrap(3),
                    composer: row.get_unwrap(4),
                    track_number: row.get_unwrap(5),
                    disc_number: row.get_unwrap(6),
                    release_date,
                    images: Vec::new(),
                },
                visual: None,
            })
        })
        .optional()?)
}

// https://developer.mozilla.org/en-US/docs/Web/Media/Formats/codecs_parameter
fn validate_codecs(reader: &dyn FormatReader, container_kind: ContainerKind) -> anyhow::Result<()> {
    for track in reader.tracks() {
        let codec = track.codec_params.codec;
        log::debug!("track {:?} codec {:x?}", track, codec);
        if match container_kind {
            ContainerKind::Flac => codec != codecs::CODEC_TYPE_FLAC,
            // If the extension is opus, we might want to be stricter
            ContainerKind::Ogg | ContainerKind::Matroska => {
                codec != codecs::CODEC_TYPE_VORBIS && codec != codecs::CODEC_TYPE_OPUS
            }
            ContainerKind::Mp3 => codec != codecs::CODEC_TYPE_MP3,
            ContainerKind::Mp4 => codec != codecs::CODEC_TYPE_AAC,
        } {
            anyhow::bail!(
                "Unexpected codec {:04x?} for container {}",
                codec,
                container_kind.mime_type()
            )
        }
    }
    Ok(())
}
