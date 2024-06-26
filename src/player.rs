use std::ops::Deref;
use std::sync::Arc;

use arc_swap::ArcSwap;
use mpris_server::{PlaybackStatus, Property};
use rust_cast::channels::connection::ConnectionResponse;
use rust_cast::channels::heartbeat::HeartbeatResponse;
use rust_cast::channels::media::Metadata::MusicTrack;
use rust_cast::channels::media::{
    ExtendedPlayerState, ExtendedStatus, MediaResponse, PlayerState, RepeatMode, StatusEntry,
};
use rust_cast::channels::receiver;
use rust_cast::{CastDevice, ChannelMessage};
use tokio::sync::Notify;

mod mpris;

// I'd like rust_cast to export those constants
pub const DEFAULT_DESTINATION_ID: &str = "receiver-0";

pub struct Player<'a> {
    pub receiver: CastDevice<'a>,
    pub transport_id: String,
    pub media_session_id: i32,
    media_status: ArcSwap<StatusEntry>,
    media_status_change: Notify,
    receiver_status: ArcSwap<receiver::Status>,
    receiver_status_change: Notify,
}

impl<'a> Player<'a> {
    pub fn from_status(
        receiver: CastDevice<'a>,
        transport_id: String,
        media_status: StatusEntry,
        receiver_status: receiver::Status,
    ) -> Self {
        Self {
            receiver,
            transport_id,
            media_session_id: media_status.media_session_id,
            media_status: ArcSwap::from_pointee(media_status),
            media_status_change: Notify::new(),
            receiver_status: ArcSwap::from_pointee(receiver_status),
            receiver_status_change: Notify::new(),
        }
    }

    fn media_status(&self) -> impl Deref<Target = Arc<StatusEntry>> {
        self.media_status.load()
    }

    fn set_media_status(&self, ms: StatusEntry) {
        // Make sure media and items (queue subset) are set;
        // many updates don't include them.
        // Mostly because there's a loading -> playing transition
        // and the second update is abbreviated.
        // TODO: add queue_data as well
        if ms.items.is_some() && ms.media.is_some() && ms.queue_data.is_some() {
            self.media_status.store(Arc::new(ms));
        } else {
            self.media_status.rcu(|prev| {
                let mut ms = ms.clone();
                if ms.items.is_none() {
                    ms.items = prev.items.clone();
                }
                if ms.media.is_none() {
                    ms.media = prev.media.clone();
                }
                if ms.queue_data.is_none() {
                    ms.queue_data = prev.queue_data.clone();
                }
                ms
            });
        }
        self.media_status_change.notify_one();
    }

    fn receiver_status(&self) -> impl Deref<Target = Arc<receiver::Status>> {
        self.receiver_status.load()
    }

    fn set_receiver_status(&self, rs: receiver::Status) {
        self.receiver_status.store(rs.into());
        self.receiver_status_change.notify_one();
    }

    async fn next(&self) -> Result<(), rust_cast::errors::Error> {
        let ms = self
            .receiver
            .media
            .next(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(ms);
        Ok(())
    }

    async fn prev(&self) -> Result<(), rust_cast::errors::Error> {
        let ms = self
            .receiver
            .media
            .prev(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(ms);
        Ok(())
    }

    async fn play(&self) -> Result<(), rust_cast::errors::Error> {
        let ms = self
            .receiver
            .media
            .play(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(ms);
        Ok(())
    }

    async fn pause(&self) -> Result<(), rust_cast::errors::Error> {
        let ms = self
            .receiver
            .media
            .pause(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(ms);
        Ok(())
    }

    async fn stop(&self) -> Result<(), rust_cast::errors::Error> {
        let ms = self
            .receiver
            .media
            .stop(&self.transport_id, self.media_session_id)
            .await?;
        self.set_media_status(ms);
        Ok(())
    }

    fn playback_status(&self) -> PlaybackStatus {
        let ms = self.media_status();
        match ms.player_state {
            PlayerState::Idle => match ms.extended_status {
                Some(ExtendedStatus {
                    player_state: ExtendedPlayerState::Loading,
                    media_session_id,
                    ..
                }) => {
                    if media_session_id == Some(self.media_session_id) {
                        PlaybackStatus::Playing
                    } else {
                        PlaybackStatus::Stopped
                    }
                }
                None => PlaybackStatus::Stopped,
            },
            PlayerState::Playing => PlaybackStatus::Playing,
            PlayerState::Buffering => PlaybackStatus::Playing,
            PlayerState::Paused => PlaybackStatus::Paused,
        }
    }

    fn loop_status(&self) -> mpris_server::LoopStatus {
        let ms = self.media_status();
        // XXX should we look at ms.repeat_mode or ms.queue_data.repeat_mode?
        match ms.repeat_mode {
            Some(RepeatMode::Off) | None => mpris_server::LoopStatus::None,
            Some(RepeatMode::All) => mpris_server::LoopStatus::Playlist,
            Some(RepeatMode::Single) => mpris_server::LoopStatus::Track,
            // XXX no exact mapping
            Some(RepeatMode::AllAndShuffle) => mpris_server::LoopStatus::Playlist,
        }
    }

    fn shuffle_status(&self) -> bool {
        let ms = self.media_status();
        if let Some(ref queue_data) = ms.queue_data {
            return queue_data.shuffle;
        }
        false
    }

    fn volume(&self) -> mpris_server::Volume {
        let ms = self.receiver_status();
        let vol = ms.volume;
        if vol.muted == Some(true) {
            return 0.;
        }
        vol.level.unwrap().into()
    }

    fn metadata(&self) -> mpris_server::Metadata {
        // There is information loss going through the cast metadata format
        // For multi-valued tags, we would be better off
        // recognizing the URL and using metadata stored on this side.
        let ms = self.media_status();
        let mut md1 = mpris_server::Metadata::new();
        if let Some(ref media) = ms.media {
            if let Some(MusicTrack(ref md0)) = media.metadata {
                md1.set_album(md0.album_name.clone());
                md1.set_title(md0.title.clone());
                md1.set_album_artist(md0.album_artist.as_ref().map(|aa| vec![aa]));
                md1.set_artist(md0.artist.as_ref().map(|a| vec![a]));
                md1.set_composer(md0.composer.as_ref().map(|c| vec![c]));
                md1.set_track_number(md0.track_number.map(|n| n.try_into().unwrap()));
                md1.set_disc_number(md0.disc_number.map(|n| n.try_into().unwrap()));
                md1.set_art_url(md0.images.first().map(|img| img.url.clone()));
                md1.set_content_created(md0.release_date.clone());
            }
            md1.set_length(
                media
                    .duration
                    .map(|d| mpris::cast_time_to_mpris_time(d.into())),
            );
        }
        md1
    }

    fn can_go_next(&self) -> bool {
        let ms = self.media_status();
        if let Some(repeat) = ms.repeat_mode {
            if repeat != RepeatMode::Off {
                return true;
            }
        }
        let Some(ref items) = ms.items else {
            // For another app than the default player,
            // this might be inaccurate, there might
            // be a queue that isn't exposed to us.
            return false;
        };
        let Some(current_item_id) = ms.current_item_id else {
            return false;
        };
        let Some(pos) = items
            .iter()
            .position(|it| it.item_id == Some(current_item_id))
        else {
            // We might assert
            return false;
        };
        if pos + 1 < items.len() {
            return true;
        }
        false
    }

    fn can_go_previous(&self) -> bool {
        let ms = self.media_status();
        if let Some(repeat) = ms.repeat_mode {
            if repeat != RepeatMode::Off {
                return true;
            }
        }
        let Some(ref items) = ms.items else {
            // For another app than the default player,
            // this might be inaccurate, there might
            // be a queue that isn't exposed to us.
            return false;
        };
        let Some(current_item_id) = ms.current_item_id else {
            return false;
        };
        if let Some(first) = items.first() {
            if let Some(fid) = first.item_id {
                if fid != current_item_id && items.len() > 1 {
                    return true;
                }
            }
        }
        false
    }
}

/// Player main loop
///
/// Read device messages, act on media status changes, and update player state
/// until the receiver closes the connection or indicates it is done playing
pub async fn run_player(server: &mpris_server::Server<Player<'static>>) {
    let player = server.imp();
    let mut playback_status = player.playback_status();
    let mut loop_status = player.loop_status();
    let mut metadata = player.metadata();
    let mut can_go_next = player.can_go_next();
    let mut can_go_previous = player.can_go_previous();
    let mut volume = player.volume();
    let mut shuffle = player.shuffle_status();
    // Volume is receiver status and needs a different notification
    //let mut volume = player.volume().await;
    loop {
        tokio::select! {
            _ = player.receiver_status_change.notified() => {
                let p = player.volume();
                if volume != p {
                    volume = p;
                    server.properties_changed([Property::Volume(p)]).await.unwrap();
                }
            }
            _ = player.media_status_change.notified() => {
                let mut props = Vec::new();
                let p = player.playback_status();
                if playback_status != p {
                    playback_status = p;
                    props.push(Property::PlaybackStatus(p));
                }
                let p = player.loop_status();
                if loop_status != p {
                    loop_status = p;
                    props.push(Property::LoopStatus(p));
                }
                let p = player.metadata();
                if metadata != p {
                    metadata = p.clone();
                    props.push(Property::Metadata(p));
                }
                let p = player.can_go_next();
                if can_go_next != p {
                    can_go_next = p;
                    props.push(Property::CanGoNext(p));
                }
                let p = player.can_go_previous();
                if can_go_previous != p {
                    can_go_previous = p;
                    props.push(Property::CanGoPrevious(p));
                }
                let p = player.shuffle_status();
                if shuffle != p {
                    shuffle = p;
                    props.push(Property::Shuffle(p));
                }
                if !props.is_empty() {
                    server.properties_changed(props).await.unwrap();
                }
            }
            msg = player.receiver.receive() => {
                match msg {
                    Ok(ChannelMessage::Heartbeat(response)) => {
                        if matches!(response, HeartbeatResponse::Ping) {
                            player.receiver.heartbeat.pong().await.unwrap();
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
                            for ms in stat.entries {
                                if ms.media_session_id != player.media_session_id {
                                    continue;
                                }
                                // The player became idle, and not because it hasn't started yet
                                // Either it's Finished (ran out of playlist), or the user explicitly stopped it,
                                // or some fatal error happened.  Either way, time to exit.
                                if let Some(_reason) = ms.idle_reason {
                                    assert_eq!(ms.player_state, PlayerState::Idle);
                                    let Some(ref es) = ms.extended_status else {
                                        // Exit when at the end of the playlist
                                        return;
                                    };
                                    // At the moment the enum has just this element,
                                    // but match so any additions must be handled.
                                    match es.player_state {
                                        ExtendedPlayerState::Loading => (),
                                    }
                                }

                                player.set_media_status(ms);
                            }
                        }
                    }
                    Ok(ChannelMessage::Receiver(response)) => {
                        log::debug!("[Receiver] {:?}", response);
                        if let receiver::ReceiverResponse::Status(status) = response {
                            player.set_receiver_status(status);
                        }
                    },
                    Ok(ChannelMessage::Raw(response)) => log::debug!(
                        "Support for the following message type is not yet supported: {:?}",
                        response
                    ),
                    Err(error) => {
                        log::error!("Error occurred while receiving message {}", error);
                        player
                            .receiver
                            .connection
                            .disconnect(DEFAULT_DESTINATION_ID)
                            .await
                            .unwrap();
                        return;
                    }
                }
            }
        }
    }
}
