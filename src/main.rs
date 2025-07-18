#![forbid(unsafe_code)]

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::path::Path;

use anyhow::Context;
use rust_cast::channels::media::{Media, MediaQueue, QueueItem, QueueType, RepeatMode, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

mod audio;
mod cli;
mod http;
mod net;
mod player;
mod scan;

use player::DEFAULT_DESTINATION_ID;

async fn play<P: AsRef<Path>>(
    paths: &[P],
    playlist_start: NonZeroU16,
    port: &cli::PortOrRange,
    beets_db: Option<&Path>,
) -> anyhow::Result<()> {
    let beets_db = if let Some(beets_db) = beets_db {
        use rusqlite::OpenFlags;
        Some(rusqlite::Connection::open_with_flags(
            beets_db,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_EXRESCODE,
        )?)
    } else {
        None
    };

    let mut playlist;
    // TODO: loop over args, recurse into directories, take files as-is
    if let [path] = paths {
        playlist = scan::dir_to_playlist(path.as_ref(), beets_db.as_ref())?;
        if playlist.entries.is_empty() {
            anyhow::bail!("Found no playable entries");
        }
    } else {
        playlist = scan::files_to_playlist(paths, beets_db.as_ref())?;
    }

    // From 1-based (UI) to 0-based
    let start_index: u16 = playlist_start.get() - 1;
    let entlen = playlist.entries.len();
    if !(..entlen).contains(&start_index.into()) {
        // greater than is accurate for the 1-based index
        anyhow::bail!("Playlist start index greater than {}", entlen);
    }
    for entry in playlist.entries.iter() {
        println!("{}", entry.path.display());
    }
    // XXX I would like mdns-sd to tell on which interface services
    // are discovered, so I can expose sender only on these (SO_BINDTODEVICE).
    // XXX This is one-shot
    let (remote_address, remote_port) = net::discover()
        .await
        .with_context(|| "Could not find Chromecast.")?;
    // XXX Could I access the socket and call socket2 local_addr
    // (libc getsockname)?  CastDevice builds the TcpStream
    // but does not expose it.
    let device = rust_cast::CastDevice::connect_without_host_verification(
        remote_address.clone(),
        remote_port,
    )
    .await?;
    let mut tcp1 = tokio::net::TcpStream::connect((remote_address.as_str(), remote_port)).await?;
    let local_addr = tcp1.local_addr()?;
    tcp1.shutdown().await?;

    let listener = net::bind(&local_addr, port).await?;
    // Like local_addr but with the effective port
    let mut expose_addr = listener.local_addr()?;
    // Clear scope_id, Display would expose it but it's host-internal
    if let SocketAddr::V6(ref mut v6) = expose_addr {
        v6.set_scope_id(0);
    }
    let base = format!("http://{expose_addr}").parse().unwrap();
    let uuid = uuid::Uuid::new_v4();
    let server = http::make_app(uuid, &mut playlist, &base);

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join_server = tokio::spawn(
        axum::serve(listener, server)
            .with_graceful_shutdown(async { shutdown_rx.await.unwrap() })
            .into_future(),
    );

    device
        .connection
        .connect(DEFAULT_DESTINATION_ID.to_string())
        .await?;
    //device.heartbeat.ping()?;

    let app = device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)
        .await?;
    // This gets reused between invocations; we do need our own UUID generation
    log::info!("App transport_id {}", app.transport_id);
    device.connection.connect(app.transport_id.as_str()).await?;
    let media_queue = MediaQueue {
        items: playlist
            .entries
            .into_iter()
            .enumerate()
            .map(|(i, ent)| QueueItem {
                media: Media {
                    content_id: http::base_with_path(&base, &format!("/{uuid}/track/{i}")).into(),
                    stream_type: StreamType::Buffered,
                    content_type: ent.mime_type.to_owned(),
                    metadata: ent
                        .metadata
                        .map(|m| rust_cast::channels::media::Metadata::MusicTrack(m.cast_metadata)),
                    duration: None,
                },
                item_id: None,
            })
            .collect(),
        start_index,
        queue_type: QueueType::Playlist,
        repeat_mode: RepeatMode::Off,
    };
    let mut status = device
        .media
        .load_queue(&app.transport_id, &app.session_id, &media_queue)
        .await?;
    let media_status = status.entries.remove(0);
    let receiver_status = device.receiver.get_status().await?;
    let player =
        player::Player::from_status(device, app.transport_id, media_status, receiver_status);
    let busname = format!("com.github.g2p.joujou.u{uuid}");
    let mpris_server = mpris_server::Server::new(&busname, player).await?;
    // XXX mpris-server is lacking a way
    // to close the connection and await that.
    player::run_player(&mpris_server).await;
    log::debug!("Shutting down our HTTP server");
    shutdown_tx.send(()).unwrap();
    join_server.await??;
    Ok(())
}

async fn listen() -> anyhow::Result<()> {
    let (remote_address, remote_port) = net::discover()
        .await
        .with_context(|| "Could not find Chromecast.")?;
    // XXX Could I access the socket and call socket2 local_addr
    // (libc getsockname)?  CastDevice builds the TcpStream
    // but does not expose it.
    let device =
        rust_cast::CastDevice::connect_without_host_verification(remote_address, remote_port)
            .await?;
    println!("Connecting to device and {}", DEFAULT_DESTINATION_ID);
    device
        .connection
        .connect(DEFAULT_DESTINATION_ID.to_string())
        .await?;
    println!("Connected to device and {}", DEFAULT_DESTINATION_ID);

    println!("Connecting to default media receiver");
    let status = device.receiver.get_status().await?;

    // Bail if the media receiver is not running
    let app = status
        .applications
        .iter()
        .find(|app| app.app_id.as_str().parse() == Ok(CastDeviceApp::DefaultMediaReceiver))
        .ok_or_else(|| anyhow::anyhow!("Default media receiver not running"))?;

    // We found the default media receiver running, connect to it
    // Presumably we could also join media sessions of other running apps
    // by looking for apps where {"name":"urn:x-cast:com.google.cast.media"}
    // appears within the app.namespaces[] array
    device.connection.connect(&app.transport_id).await?;
    println!("Connected to default media receiver {:?}", app);

    // We can ask for media status actively:
    let mut status = device.media.get_status(&app.transport_id, None).await?;
    if status.entries.is_empty() {
        return Err(anyhow::anyhow!("Media receiver is not currently playing"));
    }
    let media_status = status.entries.remove(0);
    assert!(status.entries.is_empty());
    let receiver_status = device.receiver.get_status().await?;
    let player = player::Player::from_status(
        device,
        app.transport_id.to_owned(),
        media_status,
        receiver_status,
    );
    let uuid = uuid::Uuid::new_v4();
    let busname = format!("com.github.g2p.joujou.u{uuid}");
    let mpris_server = mpris_server::Server::new(&busname, player).await?;
    player::run_player(&mpris_server).await;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "logging")]
    env_logger::init();
    let app = cli::parse_cli();
    match app.cmd {
        cli::Command::Play {
            paths,
            playlist_start,
        } => play(&paths, playlist_start, &app.port, app.beets_db.as_deref()).await,
        cli::Command::Listen => listen().await,
    }
}
