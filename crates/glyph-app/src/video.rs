//! Music video search and stream resolution commands for desktop and mobile.

use crate::controller::Core;
use glyph_protocol::{Track, VideoResolveResponse, VideoSearchResponse};
use std::sync::Arc;

pub async fn search(
    core: &Arc<Core>,
    track: Track,
    query: Option<String>,
) -> Result<VideoSearchResponse, String> {
    let backend = core.backend_or_err()?;
    backend
        .video_search(&track, query)
        .await
        .map_err(|e| e.to_string())
}

pub async fn resolve(core: &Arc<Core>, id: String) -> Result<VideoResolveResponse, String> {
    let backend = core.backend_or_err()?;
    backend.video_resolve(&id).await.map_err(|e| e.to_string())
}
