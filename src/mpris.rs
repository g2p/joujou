use mpris_server::async_trait;
use mpris_server::zbus;
use mpris_server::zbus::fdo;
use mpris_server::LoopStatus;
use mpris_server::PlaybackRate;
use mpris_server::PlaybackStatus;
use mpris_server::TrackId;
use mpris_server::{
    LocalPlayerInterface, LocalRootInterface, Metadata, Property, Server, Signal, Time, Volume,
};

fn errconvert(err: rust_cast::errors::Error) -> zbus::Error {
    zbus::Error::Failure(format!("rust_cast error {err}"))
}

pub struct Player<'a> {
    device: rust_cast::CastDevice<'a>,
    transport_id: String,
    media_session_id: i32,
}

/// https://specifications.freedesktop.org/mpris-spec/latest/Media_Player.html
#[async_trait(?Send)]
impl<'a> LocalRootInterface for Player<'a> {
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
    async fn set_fullscreen(&self, fullscreen: bool) -> mpris_server::zbus::Result<()> {
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
        todo!()
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        todo!()
    }
}

/// https://specifications.freedesktop.org/mpris-spec/latest/Player_Interface.html
#[async_trait(?Send)]
impl<'a> LocalPlayerInterface for Player<'a> {
    async fn next(&self) -> fdo::Result<()> {
        todo!()
    }

    async fn previous(&self) -> fdo::Result<()> {
        todo!()
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
        todo!()
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
    }

    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        todo!()
    }

    async fn open_uri(&self, uri: String) -> fdo::Result<()> {
        todo!()
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        todo!()
    }

    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        todo!()
    }

    async fn set_loop_status(&self, loop_status: LoopStatus) -> zbus::Result<()> {
        todo!()
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
        todo!()
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
        todo!()
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        todo!()
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        todo!()
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        todo!()
    }
}
