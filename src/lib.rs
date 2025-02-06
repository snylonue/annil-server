pub mod provider;

use std::{
    num::NonZeroU8,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anni_provider::{AnniProvider, ProviderError, Range};
use annil::{
    extractor::track::TrackIdentifier,
    provider::AnnilProvider,
    state::{AnnilKeys, AnnilState},
};
use axum::{
    extract::Path,
    http::{
        header::{ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL},
        Method, StatusCode,
    },
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Router,
};
use provider::{AnniURLProvider, SeafileProvider};
use serde::Deserialize;
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::cors;

#[derive(Deserialize)]
struct CoverPath {
    album_id: String,
    disc_id: Option<NonZeroU8>,
}

#[derive(Debug)]
enum Error {
    AnniError(ProviderError),
}

impl From<ProviderError> for Error {
    fn from(error: ProviderError) -> Self {
        Self::AnniError(error)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::AnniError(error) => (
                StatusCode::NOT_FOUND,
                [(CACHE_CONTROL, "private")],
                error.to_string(),
            ),
        }
        .into_response()
    }
}

async fn audio_redirect<P: AnniURLProvider + Send>(
    track: TrackIdentifier,
    Extension(provider): Extension<Arc<AnnilProvider<P>>>,
) -> Response {
    let provider = provider.read().await;

    let uri = match provider
        .get_audio_link(
            &track.album_id.to_string(),
            track.disc_id,
            track.track_id,
            Range::FULL,
        )
        .await
    {
        Ok(Ok(uri)) => uri,
        Err(e) => return Error::from(dbg!(e)).into_response(),
        _ => return (StatusCode::NOT_FOUND, [(CACHE_CONTROL, "private")]).into_response(),
    };

    let info = match provider
        .get_audio_info(&track.album_id.to_string(), track.disc_id, track.track_id)
        .await
    {
        Ok(info) => info,
        Err(e) => return Error::from(dbg!(e)).into_response(),
    };
    let header = [(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        "X-Origin-Type, X-Origin-Size, X-Duration-Seconds, X-Audio-Quality".to_string(),
    )];
    let headers = [
        ("X-Origin-Type", format!("audio/{}", info.extension)),
        ("X-Origin-Size", format!("{}", info.size)),
        ("X-Duration-Seconds", format!("{}", info.duration)),
        ("X-Audio-Quality", String::from("lossless")),
    ];

    (header, headers, Redirect::temporary(&uri)).into_response()
}

async fn cover_redirect<P: AnniURLProvider + Send + Sync>(
    Path(CoverPath { album_id, disc_id }): Path<CoverPath>,
    Extension(provider): Extension<Arc<AnnilProvider<P>>>,
) -> Response {
    let provider = provider.read().await;

    let uri = match provider.get_cover_link(&album_id, disc_id).await {
        Ok(Ok(uri)) => uri,
        Err(e) => return Error::from(e).into_response(),
        _ => return (StatusCode::NOT_FOUND, [(CACHE_CONTROL, "private")]).into_response(),
    };
    Redirect::temporary(&uri).into_response()
}

pub async fn make_state<P: AnniProvider + Send + Sync>(
    version: String,
    provider: &AnnilProvider<P>,
) -> AnnilState {
    AnnilState {
        version,
        last_update: RwLock::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ),
        etag: RwLock::new(provider.compute_etag().await.unwrap()),
        metadata: None,
    }
}

pub fn make_app<P: AnniURLProvider + Send + Sync + 'static>(
    provider: Arc<AnnilProvider<P>>,
    initial_state: Arc<AnnilState>,
    key: Arc<AnnilKeys>,
) -> Router {
    let router = Router::new()
        .route("/info", get(annil::route::user::info))
        .route(
            "/albums",
            get(annil::route::user::albums::<SeafileProvider>),
        )
        .route("/:album_id/cover", get(cover_redirect::<P>))
        .route(
            "/:album_id/:disc_id/cover",
            get(cover_redirect::<P>),
        )
        .route(
            "/:album_id/:disc_id/:track_id",
            get(audio_redirect::<P>)
                .head(annil::route::user::audio_head::<P>),
        )
        .route(
            "/admin/reload",
            post(annil::route::admin::reload::<P>),
        )
        .route("/admin/sign", post(annil::route::admin::sign))
        .layer(
            cors::CorsLayer::new()
                .allow_methods([Method::GET, Method::OPTIONS, Method::POST])
                .allow_headers(cors::Any)
                .allow_origin(cors::Any),
        )
        .layer(ServiceBuilder::new().layer(Extension(initial_state)))
        .layer(Extension(provider))
        .layer(Extension(key));

    router
}
