#![forbid(unsafe_code)]

use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};
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
use rust_cast::channels::media::{Media, MediaQueue, QueueItem, StreamType};
use rust_cast::channels::receiver::CastDeviceApp;
use tokio::io::AsyncWriteExt;

mod cli;

// I'd like rust_cast to export those constants
const SERVICE_TYPE: &str = "_googlecast._tcp.local.";
const DEFAULT_DESTINATION_ID: &str = "receiver-0";

async fn play(path: &std::path::Path) -> anyhow::Result<()> {
    println!("path {}", path.display());
    // List music files beforehand, sort them appropriately,
    // build the queue/playlist.
    // natord could work with OsStr; human_sort can't.
    // Using lossy for now anyway.
    // uutils has src/uucore/src/lib/features/version_cmp.rs
    // which mimics gnu version sort (with deliberate divergence due to bugs in GNU?)
    // uutils doesn't handle non-unicode though.
    // XXX Looking at walkdir source code (2.4.0), dirs are sorted
    // unconditionally (in IntoIter::push which is called whenever a directory
    // is recursed into), filter_entry is applied later.  So, for filtering music
    // extensions, we could use Iterator::filter(), no point bothering with
    // filter_entry.
    let entries: Vec<_> = walkdir::WalkDir::new(path)
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
        .filter(|dent_r| {
            if let Ok(dent) = dent_r {
                // !dent.file_type().is_dir()
                // With !is_dir:
                // This could still be a symlink (to anything, broken, etc) or a block special, etc
                // We'll count on the HTTP server to filter those out (at open time to prevent races).
                // Though, maybe we don't want to open symlinks, in which case we could filter them
                // out in both places.  If we want them, we could do a realpath whitelist.
                dent.file_type().is_file()
            } else {
                // Always pass on errors, we'll use them to break out of iteration
                true
            }
        })
        .collect::<Result<_, _>>()?;
    for entry in entries.iter() {
        println!("{}", entry.path().display());
    }
    // XXX I would like mdns-sd to tell on which interface services
    // are discovered, so I can expose sender only on these.
    // XXX This is one-shot
    let Some((address, port)) = discover().await else {
        anyhow::bail!("Could not find Chromecast.");
    };
    eprintln!("Here!");
    // XXX Could I access the socket and call socket2 local_addr
    // (libc getsockname)?  CastDevice builds the TcpStream
    // but does not expose it.
    let device = rust_cast::CastDevice::connect_without_host_verification(address.as_str(), port)?;
    let mut tcp1 = tokio::net::TcpStream::connect((address.as_str(), port)).await?;
    let local_addr = tcp1.local_addr()?;
    tcp1.shutdown().await?;
    eprintln!("There!");

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

    eprintln!("Prior!");
    device
        .connection
        .connect(DEFAULT_DESTINATION_ID.to_string())?;
    //device.heartbeat.ping()?;

    let app = device
        .receiver
        .launch_app(&CastDeviceApp::DefaultMediaReceiver)?;
    eprintln!("Before!");
    device.connection.connect(app.transport_id.as_str())?;
    eprintln!("After!");
    let media_queue = MediaQueue {
        items: (0..entries.len())
            .map(|i| QueueItem {
                media: Media {
                    content_id: format!("http://{expose_addr}/{i}"),
                    stream_type: StreamType::Buffered,
                    content_type: String::from("audio/flac"), // TODO: mime
                    metadata: None,
                    duration: None,
                },
            })
            .collect(),
    };
    println!("Asking to play {media_queue:?}");
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

async fn serve(
    listener: tokio::net::TcpListener,
    entries: Vec<walkdir::DirEntry>,
) -> anyhow::Result<()> {
    let state = AppState {
        served_files: entries.into_iter().map(|de| de.path().to_owned()).collect(),
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
    env_logger::init();
    let app = cli::parse_cli();
    match app.cmd {
        cli::Command::Play { path } => play(&path).await,
    }
}
