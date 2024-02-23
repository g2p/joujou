use std::borrow::Cow;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract;
use axum::http::header;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum_extra::headers::Range;
use axum_extra::TypedHeader;
use axum_range::{KnownSize, Ranged};
use uuid::Uuid;

#[derive(Debug)]
enum ServedData {
    FileSystem(PathBuf),
    Memory(Arc<[u8]>),
}

pub fn base_with_path(base: &url::Url, path: &str) -> url::Url {
    let mut r = base.clone();
    r.set_path(path);
    r
}

impl ServedData {
    async fn make_response(&self, range: Option<Range>) -> Result<Response, StatusCode> {
        match self {
            Self::FileSystem(path) => {
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|_| StatusCode::NOT_FOUND)?;
                let body = KnownSize::file(file)
                    .await
                    .map_err(|_| StatusCode::NOT_FOUND)?;
                Ok(Ranged::new(range, body).into_response())
            }
            Self::Memory(data) => {
                let body = Cursor::new(Arc::clone(data));
                let body = KnownSize::sized(body, data.len().try_into().unwrap_or(u64::MAX));
                Ok(Ranged::new(range, body).into_response())
            }
        }
    }
}

#[derive(Debug)]
struct ServedItem {
    mime_type: Cow<'static, str>,
    contents: ServedData,
}

impl ServedItem {
    async fn make_response(&self, range: Option<Range>) -> impl IntoResponse {
        (
            [(header::CONTENT_TYPE, self.mime_type.to_string())],
            self.contents.make_response(range).await,
        )
    }
}

#[derive(Debug)]
struct AppState {
    tracks: Vec<ServedItem>,
    visuals: Vec<ServedItem>,
    uuid: Uuid,
}

impl AppState {
    fn new(uuid: Uuid) -> Self {
        Self {
            tracks: Vec::new(),
            visuals: Vec::new(),
            uuid,
        }
    }
}

// Uuid must implement serde::Deserialize for Path extraction to compile
//#[axum::debug_handler]
async fn serve_one_track(
    extract::Path((uuid, track_id)): extract::Path<(Uuid, u16)>,
    range: Option<TypedHeader<Range>>,
    extract::State(state): extract::State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    if uuid != state.uuid {
        return Err(StatusCode::NOT_FOUND);
    }
    let item = state
        .tracks
        .get(usize::from(track_id))
        .ok_or(StatusCode::NOT_FOUND)?;
    let range = range.map(|TypedHeader(range)| range);
    Ok(item.make_response(range).await)
}

async fn serve_one_visual(
    extract::Path((uuid, id)): extract::Path<(Uuid, u16)>,
    range: Option<TypedHeader<Range>>,
    extract::State(state): extract::State<Arc<AppState>>,
) -> Result<impl IntoResponse, StatusCode> {
    if uuid != state.uuid {
        return Err(StatusCode::NOT_FOUND);
    }
    let item = state
        .visuals
        .get(usize::from(id))
        .ok_or(StatusCode::NOT_FOUND)?;
    let range = range.map(|TypedHeader(range)| range);
    Ok(item.make_response(range).await)
}

pub fn make_app(
    uuid: Uuid,
    playlist: &mut crate::scan::Playlist,
    base: &url::Url,
) -> axum::routing::Router {
    let mut state = AppState::new(uuid);
    let mut default_visual = None;
    for ent in playlist.entries.iter_mut() {
        state.tracks.push(ServedItem {
            mime_type: ent.mime_type.into(),
            contents: ServedData::FileSystem(ent.path.clone()),
        });
        if let Some(ref mut meta) = ent.metadata {
            if let Some(visual) = meta.visual.take() {
                let i = state.visuals.len();
                state.visuals.push(ServedItem {
                    mime_type: visual.media_type.into(),
                    contents: ServedData::Memory(visual.data.into()),
                });
                let mut url = base.clone();
                url.set_path(&format!("/{uuid}/visual/{i}"));
                meta.cast_metadata.images =
                    vec![rust_cast::channels::media::Image::new(url.into())];
            } else if let Some(ref cover) = playlist.cover {
                let default_visual = default_visual.get_or_insert_with(|| {
                    log::info!("No embedded cover, using {}", cover.display());
                    let i = state.visuals.len();
                    state.visuals.push(ServedItem {
                        mime_type: "image/jpeg".into(), // XXX
                        contents: ServedData::FileSystem(cover.clone()),
                    });
                    let mut url = base.clone();
                    url.set_path(&format!("/{uuid}/visual/{i}"));
                    rust_cast::channels::media::Image::new(url.into())
                });
                meta.cast_metadata.images = vec![default_visual.clone()]
            }
        }
    }
    axum::Router::new()
        .route(
            "/:uuid/track/:track_id",
            axum::routing::get(serve_one_track),
        )
        .route(
            "/:uuid/visual/:track_id",
            axum::routing::get(serve_one_visual),
        )
        .with_state(Arc::new(state))
}
