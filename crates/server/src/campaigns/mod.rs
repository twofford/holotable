//! Campaigns (milestone M3).
//!
//! | Method | Path                                         | Who    | Purpose                    |
//! |--------|----------------------------------------------|--------|----------------------------|
//! | GET    | `/api/campaigns`                             | anyone | my campaigns and my roles  |
//! | POST   | `/api/campaigns`                             | anyone | create; creator is GM      |
//! | GET    | `/api/invites/{token}`                       | anyone | preview an invite          |
//! | POST   | `/api/invites/{token}/accept`                | anyone | join through an invite     |
//! | GET    | `/api/campaigns/{id}`                        | member | name, members, scenes      |
//! | PATCH  | `/api/campaigns/{id}`                        | GM     | rename                     |
//! | DELETE | `/api/campaigns/{id}`                        | GM     | delete (name must match)   |
//! | POST   | `/api/campaigns/{id}/leave`                  | member | leave                      |
//! | PATCH  | `/api/campaigns/{id}/members/{user}`         | GM     | change role                |
//! | DELETE | `/api/campaigns/{id}/members/{user}`         | GM     | remove member              |
//! | GET    | `/api/campaigns/{id}/invites`                | GM     | list open invites          |
//! | POST   | `/api/campaigns/{id}/invites`                | GM     | create invite              |
//! | DELETE | `/api/campaigns/{id}/invites/{invite}`       | GM     | revoke invite              |
//! | GET    | `/api/campaigns/{id}/assets`                 | GM     | list uploads               |
//! | POST   | `/api/campaigns/{id}/assets?name=…`          | GM     | upload (raw file body)     |
//! | DELETE | `/api/campaigns/{id}/assets/{asset}`         | GM     | delete upload              |
//! | GET    | `/api/campaigns/{id}/assets/{asset}/tiles/{n}` | member | one picture tile         |
//! | POST   | `/api/campaigns/{id}/scenes`                 | GM     | create scene from a map    |
//! | GET    | `/api/campaigns/{id}/scenes/{scene}`         | member | everything to draw a scene |
//! | PATCH  | `/api/campaigns/{id}/scenes/{scene}`         | GM     | rename, adjust grid        |
//! | DELETE | `/api/campaigns/{id}/scenes/{scene}`         | GM     | delete scene               |
//! | PUT    | `/api/campaigns/{id}/active-scene`           | GM     | put everyone on a scene    |
//!
//! "anyone" means any signed-in user. Every `/api/campaigns/{id}/...` route
//! goes through `access::require_member` first.

pub mod access;
mod assets;
mod invites;
mod members;
mod scenes;

use axum::{Json, Router, extract::State, http::StatusCode, middleware, routing::get};
use holotable_core::campaign::{
    CampaignDetail, CampaignNameRequest, CampaignSummary, DeleteCampaignRequest, MemberInfo, Role,
    SceneSummary, SetActiveSceneRequest, normalize_name,
};

use self::access::{Member, require_member};
use crate::{
    AppState,
    auth::extract::CurrentUser,
    db,
    error::{AppError, invalid},
};

pub fn routes(state: AppState) -> Router<AppState> {
    // Routes that act on one campaign. Paths here are relative to the nest
    // point below, so "/" is `/api/campaigns/{campaign_id}` itself.
    let one_campaign = Router::new()
        .route("/", get(detail).patch(rename).delete(delete_campaign))
        .route("/active-scene", axum::routing::put(set_active_scene))
        .merge(members::routes())
        .merge(invites::campaign_routes())
        .merge(assets::routes())
        .merge(scenes::routes())
        // `route_layer` runs the middleware only for requests that matched a
        // route above, after routing, so the `{campaign_id}` path parameter
        // is available to it.
        .route_layer(middleware::from_fn_with_state(state, require_member));

    Router::new()
        .route("/api/campaigns", get(list).post(create))
        .merge(invites::public_routes())
        .nest("/api/campaigns/{campaign_id}", one_campaign)
}

/// `GET /api/campaigns`: the campaigns I'm in, with my role (CMP-002).
async fn list(
    State(state): State<AppState>,
    current: CurrentUser,
) -> Result<Json<Vec<CampaignSummary>>, AppError> {
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT c.id, c.name, m.role FROM campaigns c
         JOIN memberships m ON m.campaign_id = c.id
         WHERE m.user_id = ? ORDER BY c.name COLLATE NOCASE",
    )
    .bind(current.user.id)
    .fetch_all(&state.db)
    .await?;
    // `filter_map` drops any row whose role doesn't parse (the CHECK
    // constraint means there shouldn't be any).
    let campaigns = rows
        .into_iter()
        .filter_map(|(id, name, role)| {
            Some(CampaignSummary {
                id,
                name,
                role: Role::from_db(&role)?,
            })
        })
        .collect();
    Ok(Json(campaigns))
}

/// `POST /api/campaigns`: create a campaign; its creator becomes GM (CMP-001).
///
/// The two inserts run in a transaction: either both happen or neither does,
/// so there's never a campaign without a GM.
async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<CampaignNameRequest>,
) -> Result<(StatusCode, Json<CampaignSummary>), AppError> {
    let name = normalize_name(&req.name).map_err(invalid)?;
    let now = db::now();
    let mut tx = state.db.begin().await?;
    let id = sqlx::query("INSERT INTO campaigns (name, created_by, created_at) VALUES (?, ?, ?)")
        .bind(&name)
        .bind(current.user.id)
        .bind(now)
        // `&mut *tx` borrows the transaction's connection for this query.
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
    sqlx::query(
        "INSERT INTO memberships (campaign_id, user_id, role, joined_at) VALUES (?, ?, 'gm', ?)",
    )
    .bind(id)
    .bind(current.user.id)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    // Without `commit`, dropping `tx` rolls everything back.
    tx.commit().await?;
    tracing::info!(
        campaign_id = id,
        user_id = current.user.id,
        "campaign created"
    );
    Ok((
        StatusCode::CREATED,
        Json(CampaignSummary {
            id,
            name,
            role: Role::Gm,
        }),
    ))
}

/// `GET /api/campaigns/{id}`: name, members and scenes.
async fn detail(
    State(state): State<AppState>,
    member: Member,
) -> Result<Json<CampaignDetail>, AppError> {
    let (name, active_scene_id): (String, Option<i64>) =
        sqlx::query_as("SELECT name, active_scene_id FROM campaigns WHERE id = ?")
            .bind(member.campaign_id)
            .fetch_one(&state.db)
            .await?;

    let member_rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT u.id, u.display_name, m.role FROM memberships m
         JOIN users u ON u.id = m.user_id
         WHERE m.campaign_id = ?
         ORDER BY m.role = 'player', u.display_name COLLATE NOCASE",
    )
    .bind(member.campaign_id)
    .fetch_all(&state.db)
    .await?;
    let members = member_rows
        .into_iter()
        .filter_map(|(user_id, display_name, role)| {
            Some(MemberInfo {
                user_id,
                display_name,
                role: Role::from_db(&role)?,
            })
        })
        .collect();

    let scene_rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, name, asset_id FROM scenes WHERE campaign_id = ? ORDER BY name COLLATE NOCASE",
    )
    .bind(member.campaign_id)
    .fetch_all(&state.db)
    .await?;
    let scenes = scene_rows
        .into_iter()
        .map(|(id, name, asset_id)| SceneSummary { id, name, asset_id })
        .collect();

    Ok(Json(CampaignDetail {
        id: member.campaign_id,
        name,
        role: member.role,
        members,
        scenes,
        active_scene_id,
    }))
}

/// `PUT /api/campaigns/{id}/active-scene` (GM): put everyone on a scene
/// (MAP-010). Players who are connected are told at once and move there.
async fn set_active_scene(
    State(state): State<AppState>,
    member: Member,
    Json(req): Json<SetActiveSceneRequest>,
) -> Result<StatusCode, AppError> {
    member.require_gm()?;
    if let Some(scene_id) = req.scene_id {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT id FROM scenes WHERE id = ? AND campaign_id = ?")
                .bind(scene_id)
                .bind(member.campaign_id)
                .fetch_optional(&state.db)
                .await?;
        found.ok_or_else(crate::error::not_found)?;
    }
    sqlx::query("UPDATE campaigns SET active_scene_id = ? WHERE id = ?")
        .bind(req.scene_id)
        .bind(member.campaign_id)
        .execute(&state.db)
        .await?;
    let members: Vec<i64> =
        sqlx::query_scalar("SELECT user_id FROM memberships WHERE campaign_id = ?")
            .bind(member.campaign_id)
            .fetch_all(&state.db)
            .await?;
    state.hub.notify_users(
        &members,
        holotable_core::protocol::ServerMsg::ActiveScene {
            campaign_id: member.campaign_id,
            scene_id: req.scene_id,
        },
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/campaigns/{id}` (GM): rename (CMP-009).
async fn rename(
    State(state): State<AppState>,
    member: Member,
    Json(req): Json<CampaignNameRequest>,
) -> Result<Json<CampaignSummary>, AppError> {
    member.require_gm()?;
    let name = normalize_name(&req.name).map_err(invalid)?;
    sqlx::query("UPDATE campaigns SET name = ? WHERE id = ?")
        .bind(&name)
        .bind(member.campaign_id)
        .execute(&state.db)
        .await?;
    Ok(Json(CampaignSummary {
        id: member.campaign_id,
        name,
        role: member.role,
    }))
}

/// `DELETE /api/campaigns/{id}` (GM): delete the campaign and its files (CMP-009).
/// The body must repeat the campaign's name.
async fn delete_campaign(
    State(state): State<AppState>,
    member: Member,
    Json(req): Json<DeleteCampaignRequest>,
) -> Result<StatusCode, AppError> {
    member.require_gm()?;
    let name: String = sqlx::query_scalar("SELECT name FROM campaigns WHERE id = ?")
        .bind(member.campaign_id)
        .fetch_one(&state.db)
        .await?;
    if req.confirm_name.trim() != name {
        return Err(invalid("Type the campaign's name exactly to delete it."));
    }
    // ON DELETE CASCADE removes memberships, invites, assets and scenes.
    sqlx::query("DELETE FROM campaigns WHERE id = ?")
        .bind(member.campaign_id)
        .execute(&state.db)
        .await?;
    state.storage.remove_campaign(member.campaign_id).await?;
    state.hub.campaign_deleted(member.campaign_id);
    tracing::info!(
        campaign_id = member.campaign_id,
        user_id = member.user.id,
        "campaign deleted"
    );
    Ok(StatusCode::NO_CONTENT)
}
