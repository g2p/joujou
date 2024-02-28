#![forbid(unsafe_code)]

use std::future::IntoFuture;
use std::net::SocketAddr;
use std::num::NonZeroU16;
use std::path::Path;

use anyhow::Context;
use rust_cast::channels::connection::ConnectionResponse;
use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{
    ExtendedPlayerState, Media, MediaQueue, MediaResponse, PlayerState, QueueItem, QueueType,
    StreamType,
};
use rust_cast::channels::receiver::CastDeviceApp;
use rust_cast::ChannelMessage;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot;

mod audio;
mod cli;
mod http;
mod net;
mod scan;

// I'd like rust_cast to export those constants
const DEFAULT_DESTINATION_ID: &str = "receiver-0";

async fn play(
    path: &Path,
    playlist_start: NonZeroU16,
    port: &cli::PortOrRange,
    beets_db: Option<&Path>,
) -> anyhow::Result<()> {
    let mut playlist = scan::dir_to_playlist(path, beets_db)?;
    if playlist.entries.is_empty() {
        anyhow::bail!("Found no playable entries");
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
        remote_address.as_str(),
        remote_port,
    )?;
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
        .connect(DEFAULT_DESTINATION_ID.to_string())?;
    //device.heartbeat.ping()?;

    let app = device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)?;
    // This gets reused between invocations; we do need our own UUID generation
    log::info!("App transport_id {}", app.transport_id);
    device.connection.connect(app.transport_id.as_str())?;
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
            })
            .collect(),
        start_index,
        queue_type: QueueType::Playlist,
    };
    let status = device
        .media
        .load_queue(app.transport_id, app.session_id, &media_queue)?;
    log::debug!("Load status len {}", status.entries.len());
    for stat_ent in status.entries.iter() {
        log::debug!("Load status msid {}", stat_ent.media_session_id);
        log::debug!("Load status entry {:?}", stat_ent);
    }
    let media_session_id = status.entries.first().unwrap().media_session_id;
    // This loop will get [Media] status entries
    'messages: loop {
        match device.receive() {
            Ok(ChannelMessage::Heartbeat(response)) => {
                if let HeartbeatResponse::Ping = response {
                    device.heartbeat.pong().unwrap();
                }
            }
            Ok(ChannelMessage::Connection(response)) => {
                log::debug!("[Connection] {:?}", response);
                if matches!(response, ConnectionResponse::Close) {
                    break 'messages;
                }
            }
            Ok(ChannelMessage::Media(response)) => {
                log::debug!("[Media] {:?}", response);
                if let MediaResponse::Status(stat) = response {
                    for stat_ent in stat.entries {
                        if stat_ent.media_session_id != media_session_id {
                            continue;
                        }
                        // The player became idle, and not because it hasn't started yet
                        // Either it's Finished (ran out of playlist), or the user explicitly stopped it,
                        // or some fatal error happened.  Either way, time to exit.
                        if let Some(_reason) = stat_ent.idle_reason {
                            assert!(matches!(stat_ent.player_state, PlayerState::Idle));
                            // Added the missing impl
                            assert_eq!(stat_ent.player_state, PlayerState::Idle);
                            let Some(es) = stat_ent.extended_status else {
                                break 'messages;
                            };
                            // At the moment the enum has just this element,
                            // use this so any additions must be handled.
                            match es.player_state {
                                ExtendedPlayerState::Loading => (),
                            }
                        }
                    }
                }
            }
            Ok(ChannelMessage::Receiver(response)) => log::debug!("[Receiver] {:?}", response),
            Ok(ChannelMessage::Raw(response)) => log::debug!(
                "Support for the following message type is not yet supported: {:?}",
                response
            ),
            Err(error) => {
                log::error!("Error occurred while receiving message {}", error);
                break 'messages;
            }
        }
    }
    // TODO: tell the chromecast we're shutting down
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
    let device = rust_cast::CastDevice::connect_without_host_verification(
        remote_address.as_str(),
        remote_port,
    )?;
    println!("Connecting to device and {}", DEFAULT_DESTINATION_ID);
    device
        .connection
        .connect(DEFAULT_DESTINATION_ID.to_string())?;
    println!("Connected to device and {}", DEFAULT_DESTINATION_ID);

    println!("Connecting to default media receiver");
    let status = device.receiver.get_status()?;

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
    device.connection.connect(&app.transport_id)?;
    println!("Connected to default media receiver {:?}", app);

    // We can act for media status actively:
    device.media.get_status(&app.transport_id, None)?;

    // We will also get media status updates on track changes
    loop {
        match device.receive() {
            Ok(ChannelMessage::Heartbeat(response)) => {
                if let HeartbeatResponse::Ping = response {
                    device.heartbeat.pong().unwrap();
                }
            }
            Ok(ChannelMessage::Connection(response)) => println!("[Connection] {:?}", response),
            Ok(ChannelMessage::Media(response)) => println!("[Media] {:?}", response),
            Ok(ChannelMessage::Receiver(response)) => println!("[Receiver] {:?}", response),
            Ok(ChannelMessage::Raw(response)) => println!(
                "Support for the following message type is not yet supported: {:?}",
                response
            ),
            Err(error) => println!("Error occurred while receiving message {}", error),
        }
    }
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
        } => play(&path, playlist_start, &app.port, app.beets_db.as_deref()).await,
        cli::Command::Listen => listen().await,
    }
}
