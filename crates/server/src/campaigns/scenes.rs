//! Scenes: a map plus its own settings (MAP-009).

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use holotable_core::{
    campaign::{
        CreateSceneRequest, SceneDetail, SceneSummary, UpdateSceneRequest, normalize_name,
        tile_layout,
    },
    uvtt::UvttMap,
};

use super::access::Member;
use crate::{
    AppState, db,
    error::{AppError, invalid, not_found},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/scenes", post(create)).route(
        "/scenes/{scene_id}",
        get(detail).patch(update).delete(remove),
    )
}

/// Grid size limits, in image pixels per square.
const MIN_PPG: f64 = 8.0;
const MAX_PPG: f64 = 1000.0;

/// `POST /api/campaigns/{id}/scenes` (GM): a new scene from an uploaded map.
async fn create(
    State(state): State<AppState>,
    member: Member,
    Json(req): Json<CreateSceneRequest>,
) -> Result<(StatusCode, Json<SceneSummary>), AppError> {
    member.require_gm()?;
    let name = normalize_name(&req.name).map_err(invalid)?;
    let kind: Option<String> =
        sqlx::query_scalar("SELECT kind FROM assets WHERE id = ? AND campaign_id = ?")
            .bind(req.asset_id)
            .bind(member.campaign_id)
            .fetch_optional(&state.db)
            .await?;
    if kind.as_deref() != Some("uvtt") {
        return Err(invalid(
            "Pick an uploaded .uvtt or .dd2vtt map for the scene.",
        ));
    }
    // The scene's grid starts as the map file's.
    let map = read_map(&state, member.campaign_id, req.asset_id).await?;
    let id = sqlx::query(
        "INSERT INTO scenes (campaign_id, asset_id, name, pixels_per_grid, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(member.campaign_id)
    .bind(req.asset_id)
    .bind(&name)
    .bind(f64::from(map.resolution.pixels_per_grid))
    .bind(db::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();
    Ok((
        StatusCode::CREATED,
        Json(SceneSummary {
            id,
            name,
            asset_id: req.asset_id,
        }),
    ))
}

/// `GET /api/campaigns/{id}/scenes/{scene_id}` (member): everything needed to draw it.
async fn detail(
    State(state): State<AppState>,
    member: Member,
    Path((_, scene_id)): Path<(i64, i64)>,
) -> Result<Json<SceneDetail>, AppError> {
    let row: Option<(i64, String, f64, f64, f64, u32, u32)> = sqlx::query_as(
        "SELECT s.asset_id, s.name, s.pixels_per_grid, s.grid_offset_x, s.grid_offset_y, a.width, a.height
         FROM scenes s JOIN assets a ON a.id = s.asset_id
         WHERE s.id = ? AND s.campaign_id = ?",
    )
    .bind(scene_id)
    .bind(member.campaign_id)
    .fetch_optional(&state.db)
    .await?;
    let (asset_id, name, pixels_per_grid, grid_offset_x, grid_offset_y, width, height) =
        row.ok_or_else(not_found)?;
    let map = read_map(&state, member.campaign_id, asset_id).await?;
    Ok(Json(SceneDetail {
        id: scene_id,
        name,
        asset_id,
        pixels_per_grid,
        grid_offset_x,
        grid_offset_y,
        image_width: width,
        image_height: height,
        tiles: tile_layout(width, height),
        map,
    }))
}

/// `PATCH /api/campaigns/{id}/scenes/{scene_id}` (GM): rename, or adjust the grid.
async fn update(
    State(state): State<AppState>,
    member: Member,
    Path((_, scene_id)): Path<(i64, i64)>,
    Json(req): Json<UpdateSceneRequest>,
) -> Result<Json<SceneDetail>, AppError> {
    member.require_gm()?;
    let name = req
        .name
        .as_deref()
        .map(normalize_name)
        .transpose()
        .map_err(invalid)?;
    if let Some(ppg) = req.pixels_per_grid
        && !(MIN_PPG..=MAX_PPG).contains(&ppg)
    {
        return Err(invalid("The grid size must be between 8 and 1000 pixels."));
    }
    for offset in [req.grid_offset_x, req.grid_offset_y].into_iter().flatten() {
        if !offset.is_finite() || offset.abs() > MAX_PPG {
            return Err(invalid(
                "The grid offset must be between -1000 and 1000 pixels.",
            ));
        }
    }
    // COALESCE(?, column) keeps the old value when the new one is NULL (left out).
    let changed = sqlx::query(
        "UPDATE scenes SET
            name = COALESCE(?, name),
            pixels_per_grid = COALESCE(?, pixels_per_grid),
            grid_offset_x = COALESCE(?, grid_offset_x),
            grid_offset_y = COALESCE(?, grid_offset_y)
         WHERE id = ? AND campaign_id = ?",
    )
    .bind(name)
    .bind(req.pixels_per_grid)
    .bind(req.grid_offset_x)
    .bind(req.grid_offset_y)
    .bind(scene_id)
    .bind(member.campaign_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(not_found());
    }
    // The room keeps its own copy of the grid for checking moves; refresh it,
    // and tell everyone in the scene to reload the scene's settings.
    state.hub.scene_changed(scene_id);
    detail(State(state), member, Path((0, scene_id))).await
}

/// `DELETE /api/campaigns/{id}/scenes/{scene_id}` (GM). The map upload stays.
async fn remove(
    State(state): State<AppState>,
    member: Member,
    Path((_, scene_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    member.require_gm()?;
    let removed = sqlx::query("DELETE FROM scenes WHERE id = ? AND campaign_id = ?")
        .bind(scene_id)
        .bind(member.campaign_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(not_found());
    }
    // Tokens go with it (ON DELETE CASCADE). Close its room, telling
    // everyone in it why.
    state
        .hub
        .scene_deleted(scene_id, "The GM deleted this scene.");
    Ok(StatusCode::NO_CONTENT)
}

/// Reads a UVTT asset's saved map data (walls, doors, lights; no picture).
async fn read_map(state: &AppState, campaign_id: i64, asset_id: i64) -> Result<UvttMap, AppError> {
    let json = tokio::fs::read(state.storage.map_path(campaign_id, asset_id)).await?;
    serde_json::from_slice(&json)
        .map_err(|e| AppError::Internal(format!("stored map data is invalid: {e}")))
}
