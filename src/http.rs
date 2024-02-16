use std::path::PathBuf;
use std::sync::Arc;

use axum::extract;
use axum::extract::State;
use axum::http::StatusCode;
use axum_extra::headers::Range;
use axum_extra::TypedHeader;
use axum_range::{KnownSize, Ranged};

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

pub fn make_app(entries: &[crate::audio::AudioFile]) -> axum::routing::Router {
    let state = AppState {
        served_files: entries.iter().map(|de| de.path.clone()).collect(),
    };
    axum::Router::new()
        .route("/:track_id", axum::routing::get(serve_one_track))
        .with_state(Arc::new(state))
}
