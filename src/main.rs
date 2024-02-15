#![forbid(unsafe_code)]

use std::ffi::OsStr;
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::num::NonZeroU16;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::headers::Range;
use axum_extra::TypedHeader;
use axum_range::{KnownSize, Ranged};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use rust_cast::channels::media::{
    Media, MediaQueue, MusicTrackMediaMetadata, QueueItem, StreamType,
};
use rust_cast::channels::receiver::CastDeviceApp;
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta;
use symphonia::core::meta::MetadataReader as _;
use symphonia::default::formats::{FlacReader, MpaReader, OggReader};
use tokio::io::AsyncWriteExt;

mod cli;

// I'd like rust_cast to export those constants
const SERVICE_TYPE: &str = "_googlecast._tcp.local.";
const DEFAULT_DESTINATION_ID: &str = "receiver-0";

#[derive(Debug, Clone)]
struct AudioFile {
    path: PathBuf,
    //mime: &'static str,
    metadata: Option<MusicTrackMediaMetadata>,
}

impl AudioFile {
    /// Load known audio files (based on extension)
    /// Ok(None) if not a known extension
    /// Err if a known extension but parsing failed
    fn load_if_supported(path: PathBuf) -> anyhow::Result<Option<Self>> {
        let ext = path.extension().and_then(OsStr::to_str).unwrap_or_default();
        if let Some(ckind) = ContainerKind::from_ext(ext) {
            let metadata = read_metadata(&path, ckind)?;
            Ok(Some(AudioFile { path, metadata }))
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

fn convert_metadata(meta: &meta::MetadataRevision) -> MusicTrackMediaMetadata {
    use symphonia::core::meta::StandardTagKey::*;
    let mut rmeta = MusicTrackMediaMetadata::default();
    for tag in meta.tags() {
        let Some(stdtag) = tag.std_key else { continue };
        match stdtag {
            Album => rmeta.album_name = string_value(tag),
            TrackTitle => rmeta.title = string_value(tag),
            AlbumArtist => rmeta.album_artist = string_value(tag),
            Artist => rmeta.artist = string_value(tag),
            Composer => rmeta.composer = string_value(tag),
            TrackNumber => rmeta.track_number = u32_value(tag),
            DiscNumber => rmeta.disc_number = u32_value(tag),
            ReleaseDate => rmeta.release_date = string_value(tag),
            _ => (),
        }
    }

    rmeta
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
}

fn make_reader(
    mss: MediaSourceStream,
    container_kind: ContainerKind,
) -> symphonia::core::errors::Result<Box<dyn FormatReader>> {
    // Don't use the probe system, which currently ignores the extension hint
    // build a reader directly
    let fmt_opts = Default::default();
    Ok(match container_kind {
        ContainerKind::Flac => Box::new(FlacReader::try_new(mss, &fmt_opts)?),
        ContainerKind::Ogg => Box::new(OggReader::try_new(mss, &fmt_opts)?),
        ContainerKind::Mp3 => Box::new(MpaReader::try_new(mss, &fmt_opts)?),
    })
}

fn read_metadata(
    path: &std::path::Path,
    container_kind: ContainerKind,
) -> anyhow::Result<Option<MusicTrackMediaMetadata>> {
    let src = std::fs::File::open(path)?;
    // Default options for buffering
    let mut mss = MediaSourceStream::new(Box::new(src), Default::default());
    // For Mp3 metadata we just require id3v2, which is a container
    // around the mp3 file.  id3v1 would be 128 bytes tacked on after
    // the mp3 frames and immediately before EOF, can't really be
    // detected unambiguously.
    if container_kind == ContainerKind::Mp3 {
        let mut mreader = symphonia_metadata::id3v2::Id3v2Reader::new(&Default::default());
        let meta = mreader.read_all(&mut mss)?;
        return Ok(Some(convert_metadata(&meta)));
    }
    let mut reader = make_reader(mss, container_kind)?;

    let meta = reader.metadata();
    let Some(meta) = meta.current() else {
        return Ok(None);
    };

    Ok(Some(convert_metadata(meta)))
}

/// List music files, sort them appropriately, build the queue/playlist
fn scan_to_playlist(path: &std::path::Path) -> anyhow::Result<Vec<AudioFile>> {
    walkdir::WalkDir::new(path)
        .same_file_system(true)
        .sort_by(|a, b| {
            natord::compare(
                &a.file_name().to_string_lossy(),
                &b.file_name().to_string_lossy(),
            )
            .then_with(|| a.file_name().cmp(b.file_name()))
        })
        .into_iter()
        .filter_entry(|dent| {
            if dent.file_name().as_bytes().starts_with(b".") {
                return false;
            };
            true
        })
        .filter_map(|dent_r| {
            match dent_r {
                Ok(dent) => {
                    // !dent.file_type().is_dir()
                    // With !is_dir:
                    // This could still be a symlink (to anything,
                    // broken, etc) or a block special, etc
                    // If we don't want symlinks we could filter them
                    // out in both places we open files (for metadata
                    // and from the http server).  If we want them, we
                    // could do a realpath whitelist.
                    if dent.file_type().is_file() {
                        let path = dent.into_path();
                        AudioFile::load_if_supported(path).transpose()
                    } else {
                        None
                    }
                }
                // Always pass on errors, we'll use them to break out of iteration
                Err(err) => Some(Err(err.into())),
            }
        })
        .collect::<Result<_, _>>()
}

async fn play(path: &std::path::Path, playlist_start: NonZeroU16) -> anyhow::Result<()> {
    let entries = scan_to_playlist(path)?;
    if entries.is_empty() {
        anyhow::bail!("Found no playable entries");
    }
    // From 1-based (UI) to 0-based
    let start_index: u16 = playlist_start.get() - 1;
    if !(..entries.len()).contains(&start_index.into()) {
        // greater than is accurate for the 1-based index
        anyhow::bail!("Playlist start index greater than {}", entries.len());
    }
    for entry in entries.iter() {
        println!("{}", entry.path.display());
    }
    // XXX I would like mdns-sd to tell on which interface services
    // are discovered, so I can expose sender only on these.
    // XXX This is one-shot
    let Some((address, port)) = discover().await else {
        anyhow::bail!("Could not find Chromecast.");
    };
    // XXX Could I access the socket and call socket2 local_addr
    // (libc getsockname)?  CastDevice builds the TcpStream
    // but does not expose it.
    let device = rust_cast::CastDevice::connect_without_host_verification(address.as_str(), port)?;
    let mut tcp1 = tokio::net::TcpStream::connect((address.as_str(), port)).await?;
    let local_addr = tcp1.local_addr()?;
    tcp1.shutdown().await?;

    // Rebuild with only the stuff we want
    // (we could also just clear port and v6 flow info)
    let listen_addr = match local_addr {
        SocketAddr::V4(v4) => SocketAddr::V4(SocketAddrV4::new(v4.ip().clone(), 0)),
        SocketAddr::V6(v6) => {
            SocketAddr::V6(SocketAddrV6::new(v6.ip().clone(), 0, 0, v6.scope_id()))
        }
    };

    // XXX A random port is harder to whitelist in a firewall,
    // provide a way to keep the same port?
    // In which case, AppState and URLs would have to include
    // a UUID to distinguish playlists/sessions.
    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    // Fill in the port
    let mut expose_addr = listener.local_addr()?;
    // Clear scope_id, Display would expose it but it's host-internal
    if let SocketAddr::V6(ref mut v6) = expose_addr {
        v6.set_scope_id(0);
    }
    let join_server = tokio::spawn(serve(listener, entries.clone()));

    device
        .connection
        .connect(DEFAULT_DESTINATION_ID.to_string())?;
    //device.heartbeat.ping()?;

    let app = device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)?;
    device.connection.connect(app.transport_id.as_str())?;
    let media_queue = MediaQueue {
        items: entries
            .into_iter()
            .enumerate()
            .map(|(i, ent)| QueueItem {
                media: Media {
                    content_id: format!("http://{expose_addr}/{i}"),
                    stream_type: StreamType::Buffered,
                    content_type: String::from("audio/flac"), // TODO: mime
                    metadata: ent
                        .metadata
                        .map(rust_cast::channels::media::Metadata::MusicTrack),
                    duration: None,
                },
            })
            .collect(),
        start_index,
    };
    device
        .media
        .load_queue(app.transport_id, app.session_id, &media_queue)?;
    join_server.await??;
    Ok(())
}

struct AppState {
    served_files: Vec<PathBuf>,
}

async fn serve_one_track(
    extract::Path(track_id): extract::Path<u16>,
    range: Option<TypedHeader<Range>>,
    State(state): State<Arc<AppState>>,
) -> Result<Ranged<KnownSize<tokio::fs::File>>, StatusCode> {
    let path = state
        .served_files
        .get(usize::from(track_id))
        .ok_or(StatusCode::NOT_FOUND)?;
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let body = KnownSize::file(file)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    let range = range.map(|TypedHeader(range)| range);
    Ok(Ranged::new(range, body))
}

async fn serve(listener: tokio::net::TcpListener, entries: Vec<AudioFile>) -> anyhow::Result<()> {
    let state = AppState {
        served_files: entries.into_iter().map(|de| de.path).collect(),
    };
    let app = axum::Router::new()
        .route("/:track_id", axum::routing::get(serve_one_track))
        .with_state(Arc::new(state));
    axum::serve(listener, app).await?;
    Ok(())
}

async fn discover() -> Option<(String, u16)> {
    let mdns = ServiceDaemon::new().expect("Failed to create mDNS daemon.");

    let receiver = mdns
        .browse(SERVICE_TYPE)
        .expect("Failed to browse mDNS services.");

    while let Ok(event) = receiver.recv_async().await {
        match event {
            ServiceEvent::ServiceResolved(info) => {
                let mut addresses = info
                    .get_addresses()
                    .iter()
                    .map(|address| address.to_string())
                    .collect::<Vec<_>>();
                println!(
                    "Resolved a new service: {} ({})",
                    info.get_fullname(),
                    addresses.join(", ")
                );

                return Some((addresses.remove(0), info.get_port()));
            }
            other_event => {
                println!("Received other service event: {:?}", other_event);
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "logging")]
    env_logger::init();
    let app = cli::parse_cli();
    match app.cmd {
        cli::Command::Play {
            path,
            playlist_start,
        } => play(&path, playlist_start).await,
    }
}
