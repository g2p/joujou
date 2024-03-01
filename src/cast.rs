use rust_cast::channels::connection::ConnectionResponse;
use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{ExtendedPlayerState, MediaResponse, PlayerState};
use rust_cast::{CastDevice, ChannelMessage};

// I'd like rust_cast to export those constants
pub const DEFAULT_DESTINATION_ID: &str = "receiver-0";

/// Blocking function that reads device messages,
/// until the peer closes the connection
pub fn sender_loop(device: CastDevice, media_session_id: i32) {
    loop {
        match device.receive() {
            Ok(ChannelMessage::Heartbeat(response)) => {
                if let HeartbeatResponse::Ping = response {
                    device.heartbeat.pong().unwrap();
                }
            }
            Ok(ChannelMessage::Connection(response)) => {
                log::debug!("[Connection] {:?}", response);
                if matches!(response, ConnectionResponse::Close) {
                    return;
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
                                // Exit when at the end of the playlist
                                return;
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
                device
                    .connection
                    .disconnect(DEFAULT_DESTINATION_ID)
                    .unwrap();
                return;
            }
        }
    }
}
