use std::ops::Deref;
use std::sync::Arc;

use arc_swap::ArcSwap;
use rust_cast::channels::connection::ConnectionResponse;
use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::{self, ExtendedPlayerState, MediaResponse, PlayerState};
use rust_cast::{CastDevice, ChannelMessage};

// I'd like rust_cast to export those constants
pub const DEFAULT_DESTINATION_ID: &str = "receiver-0";

pub struct Player<'a> {
    pub receiver: CastDevice<'a>,
    pub transport_id: String,
    pub media_session_id: i32,
    media_status: ArcSwap<media::StatusEntry>,
    //pub receiver_status: Mutex<Option<receiver::Status>>,
}

impl<'a> Player<'a> {
    pub fn from_status(
        receiver: CastDevice<'a>,
        transport_id: String,
        media_status: media::StatusEntry,
    ) -> Self {
        Self {
            receiver,
            transport_id,
            media_session_id: media_status.media_session_id,
            media_status: ArcSwap::from_pointee(media_status),
        }
    }

    pub fn media_status(&self) -> impl Deref<Target = Arc<media::StatusEntry>> {
        self.media_status.load()
    }

    fn set_media_status(&self, ms: media::StatusEntry) {
        // Make sure items (queue subset) is set;
        // many updates don't include it
        if ms.items.is_some() {
            self.media_status.store(Arc::new(ms));
        } else {
            self.media_status.rcu(|prev| {
                let mut ms = ms.clone();
                ms.items = prev.items.clone();
                ms
            });
        }
    }

    /// Read device messages and update our state
    /// until the peer closes the connection
    /// or indicates it is done playing.
    pub async fn listen_to_receiver(&self) {
        loop {
            match self.receiver.receive().await {
                Ok(ChannelMessage::Heartbeat(response)) => {
                    if matches!(response, HeartbeatResponse::Ping) {
                        self.receiver.heartbeat.pong().await.unwrap();
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
                        for statent in stat.entries {
                            if statent.media_session_id != self.media_session_id {
                                continue;
                            }
                            // The player became idle, and not because it hasn't started yet
                            // Either it's Finished (ran out of playlist), or the user explicitly stopped it,
                            // or some fatal error happened.  Either way, time to exit.
                            if let Some(_reason) = statent.idle_reason {
                                assert!(matches!(statent.player_state, PlayerState::Idle));
                                // Added the missing impl
                                assert_eq!(statent.player_state, PlayerState::Idle);
                                let Some(ref es) = statent.extended_status else {
                                    // Exit when at the end of the playlist
                                    return;
                                };
                                // At the moment the enum has just this element,
                                // but match so any additions must be handled.
                                match es.player_state {
                                    ExtendedPlayerState::Loading => (),
                                }
                            }

                            self.set_media_status(statent);
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
                    self.receiver
                        .connection
                        .disconnect(DEFAULT_DESTINATION_ID)
                        .await
                        .unwrap();
                    return;
                }
            }
        }
    }

    pub async fn next(&self) -> Result<(), rust_cast::errors::Error> {
        let statent = self
            .receiver
            .media
            .next(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(statent);
        Ok(())
    }

    pub async fn prev(&self) -> Result<(), rust_cast::errors::Error> {
        let statent = self
            .receiver
            .media
            .prev(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(statent);
        Ok(())
    }

    pub async fn play(&self) -> Result<(), rust_cast::errors::Error> {
        let statent = self
            .receiver
            .media
            .play(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(statent);
        Ok(())
    }

    pub async fn pause(&self) -> Result<(), rust_cast::errors::Error> {
        let statent = self
            .receiver
            .media
            .pause(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(statent);
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), rust_cast::errors::Error> {
        let statent = self
            .receiver
            .media
            .stop(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(statent);
        Ok(())
    }
}
