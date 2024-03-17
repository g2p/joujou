use mpris_server::async_trait;
use mpris_server::zbus;
use mpris_server::zbus::fdo;
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, RootInterface, Time,
    TrackId, Volume,
};
use rust_cast::channels::media::RepeatMode;

use super::Player;

fn errconvert(err: rust_cast::errors::Error) -> zbus::Error {
    zbus::Error::Failure(format!("rust_cast error {err}"))
}

fn mpris_time_to_seek_time(time: Time) -> f32 {
    // No from or tryfrom in this case (lossy); "as" casts are the only option
    // mpris Time is internally i64 microseconds
    ((time.as_micros() as f64) / 1_000_000.) as f32
}

pub fn cast_time_to_mpris_time(time: f64) -> Time {
    Time::from_micros((time * 1_000_000.) as i64)
}

/// https://specifications.freedesktop.org/mpris-spec/latest/Media_Player.html
#[async_trait]
impl<'a> RootInterface for Player<'a> {
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn quit(&self) -> fdo::Result<()> {
        todo!()
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    // XXX the trait uses a different error type, seems like a mistake
    async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("Joujou".to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        //Err(fdo::Error::NotSupported("No desktop entry".to_owned()))
        Ok(String::new())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        // We don't support https://specifications.freedesktop.org/mpris-spec/latest/Player_Interface.html#Method:OpenUri
        // so keep the list empty
        Ok(Vec::new())
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        // We don't support https://specifications.freedesktop.org/mpris-spec/latest/Player_Interface.html#Method:OpenUri
        // so keep the list empty
        Ok(Vec::new())
    }
}

/// https://specifications.freedesktop.org/mpris-spec/latest/Player_Interface.html
#[async_trait]
impl<'a> PlayerInterface for Player<'a> {
    async fn next(&self) -> fdo::Result<()> {
        self.next().await.map_err(errconvert)?;
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.prev().await.map_err(errconvert)?;
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.pause().await.map_err(errconvert)?;
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        match self.playback_status() {
            PlaybackStatus::Playing => self.pause().await,
            PlaybackStatus::Paused | PlaybackStatus::Stopped => self.play().await,
        }
        .map_err(errconvert)?;
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        // TODO: kill self.media_session_id, exit task
        self.stop().await.map_err(errconvert)?;
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.play().await.map_err(errconvert)?;
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.receiver
            .media
            .seek(
                &self.transport_id,
                self.media_session_id,
                None,
                Some(mpris_time_to_seek_time(offset)),
                None,
            )
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        // TODO check TrackId matches
        log::debug!("set_position TrackId {track_id}");
        self.receiver
            .media
            .seek(
                &self.transport_id,
                self.media_session_id,
                Some(mpris_time_to_seek_time(position)),
                None,
                None,
            )
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        // Is it possible to return something like NoSuchMethod?
        Err(fdo::Error::NotSupported(
            "Loading on the fly is not supported".to_owned(),
        ))
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self.playback_status())
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        Ok(self.loop_status())
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> zbus::Result<()> {
        let ms = self
            .receiver
            .media
            .update_queue(
                &self.transport_id,
                self.media_session_id,
                Some(match loop_status {
                    LoopStatus::None => RepeatMode::Off,
                    LoopStatus::Track => RepeatMode::Single,
                    LoopStatus::Playlist => RepeatMode::All,
                }),
                None,
            )
            .await
            .map_err(errconvert)?;
        self.set_media_status(ms);
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        // XXX
        Ok(1.)
    }

    async fn set_rate(&self, _rate: PlaybackRate) -> zbus::Result<()> {
        todo!()
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(self.shuffle_status())
    }

    async fn set_shuffle(&self, shuffle: bool) -> zbus::Result<()> {
        // Will setting this to false restore the original order?
        // Needs testing
        let ms = self
            .receiver
            .media
            .update_queue(
                &self.transport_id,
                self.media_session_id,
                None,
                Some(shuffle),
            )
            .await
            .map_err(errconvert)?;
        self.set_media_status(ms);
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.metadata())
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.volume())
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        // XXX channel::receiver::set_volume drops most of
        // the RECEIVER_STATUS reply to keep only part of
        // the volume struct.
        let _volume = self
            .receiver
            .receiver
            .set_volume(volume as f32)
            .await
            .map_err(errconvert)?;
        // So we follow up with a get_status call
        self.set_receiver_status(
            self.receiver
                .receiver
                .get_status()
                .await
                .map_err(errconvert)?,
        );
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        let ms = self.media_status();
        Ok(cast_time_to_mpris_time(
            ms.current_time.unwrap_or_default().into(),
        ))
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        // XXX
        Ok(1.)
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        // XXX
        Ok(1.)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(self.can_go_next())
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(self.can_go_previous())
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        // There is always a current track, we don't launch with an empty
        // tracklist
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        // No live streams, everything can be paused
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}
