use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use tokio::net::TcpListener;

use crate::cli::PortOrRange;

// I'd like rust_cast to export those constants
const SERVICE_TYPE: &str = "_googlecast._tcp.local.";

pub async fn bind(local_addr: &SocketAddr, port: &PortOrRange) -> std::io::Result<TcpListener> {
    // Rebuild with only the stuff we want
    // (we could also just clear port and v6 flow info)
    // Not taking a plain IpAddr, we do want the scope_id from SocketAddrV6
    let mut listen_addr = match local_addr {
        SocketAddr::V4(v4) => SocketAddr::V4(SocketAddrV4::new(*v4.ip(), 0)),
        SocketAddr::V6(v6) => SocketAddr::V6(SocketAddrV6::new(*v6.ip(), 0, 0, v6.scope_id())),
    };

    // TODO: Now that we can reuse ports (through the cli flag and tokio
    // setting SO_REUSEPORT), should AppState and URLs include a UUID to
    // distinguish playlists/sessions?
    match port {
        PortOrRange::RandomPort => tokio::net::TcpListener::bind(listen_addr).await,
        PortOrRange::SinglePort(port) => {
            listen_addr.set_port(port.get());
            tokio::net::TcpListener::bind(listen_addr).await
        }
        PortOrRange::Range(_range) => {
            let mut firsterr = None;
            // It's easier to implement IntoIterator on the enum itself
            for port in port.clone() {
                listen_addr.set_port(port);
                match tokio::net::TcpListener::bind(listen_addr).await {
                    Ok(listener) => return Ok(listener),
                    Err(err) if firsterr.is_none() => firsterr = Some(err),
                    Err(_) => (),
                }
            }
            // Won't be None, at least if the port range is constructed
            // through FromStr in the CLI parser it can't be empty
            Err(firsterr.unwrap())
        }
    }
}

pub async fn discover() -> Option<(String, u16)> {
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
