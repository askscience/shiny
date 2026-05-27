use axum::extract::{Extension, Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::api::AppState;
use crate::errors::AppError;
use crate::models::Traveler;
use crate::services::artifacts::{self, Artifact, ArtifactSummary, ArtifactUpdate};

#[derive(Serialize)]
pub struct ArtifactListResponse {
    pub success: bool,
    pub data: Vec<ArtifactSummary>,
}

#[derive(Serialize)]
pub struct ArtifactResponse {
    pub success: bool,
    pub data: Artifact,
}

#[derive(Deserialize)]
pub struct UpsertArtifactRequest {
    pub artifact: Artifact,
    pub trip_id: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
) -> Result<Json<ArtifactListResponse>, AppError> {
    let rows = artifacts::list_summaries(&state.pool, &traveler.id).await?;
    Ok(Json(ArtifactListResponse {
        success: true,
        data: rows,
    }))
}

pub async fn get_one(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
) -> Result<Json<ArtifactResponse>, AppError> {
    let artifact = artifacts::load_artifact(&state.pool, &traveler.id, &id).await?;
    Ok(Json(ArtifactResponse {
        success: true,
        data: artifact,
    }))
}

pub async fn create(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Json(body): Json<UpsertArtifactRequest>,
) -> Result<Json<ArtifactResponse>, AppError> {
    let artifact =
        artifacts::save_artifact(&state.pool, &traveler.id, body.trip_id.as_deref(), &body.artifact)
            .await?;
    Ok(Json(ArtifactResponse {
        success: true,
        data: artifact,
    }))
}

pub async fn update(
    State(state): State<AppState>,
    Extension(traveler): Extension<Traveler>,
    Path(id): Path<String>,
    Json(body): Json<ArtifactUpdate>,
) -> Result<Json<ArtifactResponse>, AppError> {
    let artifact = artifacts::merge_update(&state.pool, &traveler.id, &id, body).await?;
    Ok(Json(ArtifactResponse {
        success: true,
        data: artifact,
    }))
}
