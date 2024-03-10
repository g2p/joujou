use mpris_server::async_trait;
use mpris_server::zbus;
use mpris_server::zbus::fdo;
use mpris_server::{
    LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface, RootInterface, Time,
    TrackId, Volume,
};
use rust_cast::channels::media::RepeatMode;
use rust_cast::channels::media::{ExtendedPlayerState, ExtendedStatus, PlayerState};

fn errconvert(err: rust_cast::errors::Error) -> zbus::Error {
    zbus::Error::Failure(format!("rust_cast error {err}"))
}

pub struct Player<'a> {
    pub device: rust_cast::CastDevice<'a>,
    pub transport_id: String,
    pub media_session_id: i32,
}

fn mpris_time_to_seek_time(time: Time) -> f32 {
    // No from or tryfrom in this case (lossy); "as" casts are the only option
    // mpris Time is internally i64 microseconds
    ((time.as_micros() as f64) / 1_000_000.) as f32
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
        todo!()
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("Joujou".to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        todo!()
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
        self.device
            .media
            .next(&self.transport_id, self.media_session_id)
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.device
            .media
            .prev(&self.transport_id, self.media_session_id)
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn pause(&self) -> fdo::Result<()> {
        self.device
            .media
            .pause(&self.transport_id, self.media_session_id)
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        todo!()
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.device
            .media
            .stop(&self.transport_id, self.media_session_id)
            .await
            .map_err(errconvert)?;
        // TODO: kill self.media_session_id, exit task
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        self.device
            .media
            .play(&self.transport_id, self.media_session_id)
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        todo!()
        // https://developers.google.com/cast/docs/reference/web_receiver/cast.framework.messages.SeekRequestData
        // can also work with relativeTime, but it's not exposed at the moment
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        // TODO check TrackId matches
        self.device
            .media
            .seek(
                &self.transport_id,
                self.media_session_id,
                Some(mpris_time_to_seek_time(position)),
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
        // We could proactively cache all status messages (for our
        // media_session_id) as we receive them
        let status = self
            .device
            .media
            .get_status(&self.transport_id, Some(self.media_session_id))
            .await
            .map_err(errconvert)?;
        // Should have just the one we requested
        assert_eq!(status.entries.len(), 1);
        let sentry = &status.entries[0];
        assert_eq!(sentry.media_session_id, self.media_session_id);
        Ok(match sentry.player_state {
            PlayerState::Idle => match sentry.extended_status {
                Some(ExtendedStatus {
                    player_state: ExtendedPlayerState::Loading,
                    ..
                }) => PlaybackStatus::Playing,
                None => PlaybackStatus::Stopped,
            },
            PlayerState::Playing => PlaybackStatus::Playing,
            PlayerState::Buffering => PlaybackStatus::Playing,
            PlayerState::Paused => PlaybackStatus::Paused,
        })
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        todo!()
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> zbus::Result<()> {
        self.device
            .media
            .update_queue(
                &self.transport_id,
                self.media_session_id,
                Some(match loop_status {
                    LoopStatus::None => RepeatMode::Off,
                    LoopStatus::Track => RepeatMode::Single,
                    LoopStatus::Playlist => RepeatMode::All,
                }),
            )
            .await
            .map_err(errconvert)?;
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        todo!()
    }

    async fn set_rate(&self, rate: PlaybackRate) -> zbus::Result<()> {
        todo!()
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        todo!()
    }

    async fn set_shuffle(&self, shuffle: bool) -> zbus::Result<()> {
        todo!()
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        todo!()
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        let status = self
            .device
            .receiver
            .get_status()
            .await
            .map_err(errconvert)?;
        let vol = &status.volume;
        if vol.muted == Some(true) {
            return Ok(0.);
        }
        let vol = vol.level.unwrap();
        Ok(vol.into())
    }

    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        todo!()
    }

    async fn position(&self) -> fdo::Result<Time> {
        todo!()
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        todo!()
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        todo!()
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        todo!()
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        todo!()
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
        todo!()
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}
