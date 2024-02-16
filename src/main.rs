#![forbid(unsafe_code)]

use std::future::IntoFuture;
use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
use std::num::NonZeroU16;
use std::os::unix::ffi::OsStrExt;

use mdns_sd::{ServiceDaemon, ServiceEvent};
use rust_cast::channels::media::{Media, MediaQueue, QueueItem, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use tokio::io::AsyncWriteExt;

mod audio;
mod cli;
mod http;

use audio::AudioFile;

// I'd like rust_cast to export those constants
const SERVICE_TYPE: &str = "_googlecast._tcp.local.";
const DEFAULT_DESTINATION_ID: &str = "receiver-0";

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
        SocketAddr::V4(v4) => SocketAddr::V4(SocketAddrV4::new(*v4.ip(), 0)),
        SocketAddr::V6(v6) => SocketAddr::V6(SocketAddrV6::new(*v6.ip(), 0, 0, v6.scope_id())),
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
    let server = http::make_app(entries.as_slice());
    let join_server = tokio::spawn(axum::serve(listener, server).into_future());

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
                    content_type: ent.mime.to_owned(),
                    metadata: ent
                        .metadata
                        .map(|m| rust_cast::channels::media::Metadata::MusicTrack(m.cast_metadata)),
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
