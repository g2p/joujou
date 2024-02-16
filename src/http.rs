use std::path::PathBuf;
use std::sync::Arc;

use axum::extract;
use axum::http::header;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::headers::Range;
use axum_extra::TypedHeader;
use axum_range::{KnownSize, Ranged};

#[derive(Debug)]
enum ServedData {
    FileSystem(PathBuf),
    Memory(Box<[u8]>),
}

impl ServedData {
    async fn make_response(&self, range: Option<Range>) -> Result<Response, StatusCode> {
        match self {
            Self::FileSystem(ref path) => {
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|_| StatusCode::NOT_FOUND)?;
                let body = KnownSize::file(file)
                    .await
                    .map_err(|_| StatusCode::NOT_FOUND)?;
                Ok(Ranged::new(range, body).into_response())
            }
            Self::Memory(_) => todo!(),
        }
    }
}

#[derive(Debug)]
struct ServedItem {
    mime_type: &'static str,
    contents: ServedData,
}

impl ServedItem {
    async fn make_response(&self, range: Option<Range>) -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, self.mime_type)],
            self.contents.make_response(range).await,
        )
    }
}

#[derive(Debug, Default)]
struct AppState {
    tracks: Vec<ServedItem>,
    visuals: Vec<ServedItem>,
}

async fn serve_one_track(
    extract::Path(track_id): extract::Path<u16>,
    range: Option<TypedHeader<Range>>,
    extract::State(state): extract::State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    let item = state
        .tracks
        .get(usize::from(track_id))
        .ok_or(StatusCode::NOT_FOUND)?;
    let range = range.map(|TypedHeader(range)| range);
    Ok(item.make_response(range).await)
}

pub fn make_app(entries: &[crate::audio::AudioFile]) -> axum::routing::Router {
    let mut state = AppState::default();
    for ent in entries.iter() {
        state.tracks.push(ServedItem {
            mime_type: ent.mime_type,
            contents: ServedData::FileSystem(ent.path.clone()),
        })
    }
    axum::Router::new()
        .route("/track/:track_id", axum::routing::get(serve_one_track))
        .with_state(Arc::new(state))
}
