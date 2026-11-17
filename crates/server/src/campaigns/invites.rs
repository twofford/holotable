//! Invite links (CMP-003 to CMP-006).
//!
//! A GM creates an invite with a role, an expiry and a number of uses. The
//! link holds a secret token; the database holds only its hash. Anyone signed
//! in who has the link can preview it and accept it.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use holotable_core::campaign::{
    CampaignSummary, CreateInviteRequest, CreatedInvite, DEFAULT_INVITE_DAYS, InviteInfo,
    InvitePreview, MAX_INVITE_DAYS, MAX_INVITE_USES, Role,
};

use super::access::{Member, role_in};
use crate::{
    AppState,
    auth::extract::CurrentUser,
    db,
    error::{AppError, invalid, not_found},
    tokens::SecretToken,
};

const DAY_SECS: i64 = 24 * 60 * 60;
const INVALID_INVITE: &str =
    "This invite link has expired, been used up, or been revoked. Ask your GM for a new one.";

/// Routes inside `/api/campaigns/{campaign_id}` (behind `require_member`).
pub fn campaign_routes() -> Router<AppState> {
    Router::new()
        .route("/invites", get(list).post(create))
        .route("/invites/{invite_id}", delete(revoke))
}

/// Routes for people who aren't members yet.
pub fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/invites/{token}", get(preview))
        .route("/api/invites/{token}/accept", post(accept))
}

/// The columns of an invite row, in `InviteInfo` order.
type InviteRow = (i64, String, i64, i64, i64, i64);

fn to_info((id, role, created_at, expires_at, max_uses, uses): InviteRow) -> Option<InviteInfo> {
    Some(InviteInfo {
        id,
        role: Role::from_db(&role)?,
        created_at,
        expires_at,
        max_uses: max_uses.try_into().ok()?,
        uses: uses.try_into().ok()?,
    })
}

/// `GET /api/campaigns/{id}/invites` (GM): invites that can still be used.
async fn list(
    State(state): State<AppState>,
    member: Member,
) -> Result<Json<Vec<InviteInfo>>, AppError> {
    member.require_gm()?;
    let rows: Vec<InviteRow> = sqlx::query_as(
        "SELECT id, role, created_at, expires_at, max_uses, uses FROM invites
         WHERE campaign_id = ? AND revoked_at IS NULL AND expires_at > ? AND uses < max_uses
         ORDER BY created_at DESC",
    )
    .bind(member.campaign_id)
    .bind(db::now())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.into_iter().filter_map(to_info).collect()))
}

/// `POST /api/campaigns/{id}/invites` (GM): create an invite (CMP-003, CMP-004).
/// The response is the only time the token is ever shown.
async fn create(
    State(state): State<AppState>,
    member: Member,
    Json(req): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<CreatedInvite>), AppError> {
    member.require_gm()?;
    let days = req.expires_in_days.unwrap_or(DEFAULT_INVITE_DAYS);
    let max_uses = req.max_uses.unwrap_or(1);
    if !(1..=MAX_INVITE_DAYS).contains(&days) {
        return Err(invalid("An invite can last from 1 to 30 days."));
    }
    if !(1..=MAX_INVITE_USES).contains(&max_uses) {
        return Err(invalid("An invite can be used from 1 to 100 times."));
    }

    let token = SecretToken::generate();
    let now = db::now();
    let expires_at = now + i64::from(days) * DAY_SECS;
    let id = sqlx::query(
        "INSERT INTO invites (campaign_id, token_hash, role, created_by, created_at, expires_at, max_uses)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(member.campaign_id)
    .bind(token.hash())
    .bind(req.role.as_str())
    .bind(member.user.id)
    .bind(now)
    .bind(expires_at)
    .bind(max_uses)
    .execute(&state.db)
    .await?
    .last_insert_rowid();

    let invite = InviteInfo {
        id,
        role: req.role,
        created_at: now,
        expires_at,
        max_uses,
        uses: 0,
    };
    Ok((
        StatusCode::CREATED,
        Json(CreatedInvite {
            invite,
            token: token.expose().to_owned(),
        }),
    ))
}

/// `DELETE /api/campaigns/{id}/invites/{invite_id}` (GM): revoke.
async fn revoke(
    State(state): State<AppState>,
    member: Member,
    Path((_, invite_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    member.require_gm()?;
    let changed = sqlx::query(
        "UPDATE invites SET revoked_at = ? WHERE id = ? AND campaign_id = ? AND revoked_at IS NULL",
    )
    .bind(db::now())
    .bind(invite_id)
    .bind(member.campaign_id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if changed == 0 {
        Err(not_found())
    } else {
        Ok(StatusCode::NO_CONTENT)
    }
}

/// An invite looked up by its token: (invite id, campaign id, campaign name,
/// role, still usable?).
type FoundInvite = (i64, i64, String, String, bool);

/// The invite a token belongs to, usable or not. Someone who already joined
/// through a single-use link should still get a friendly answer from it, so
/// "usable" is reported rather than filtered out here.
async fn find(state: &AppState, token: &str) -> Result<(i64, i64, String, Role, bool), AppError> {
    let invalid_invite = || AppError::NotFound(INVALID_INVITE.to_owned());
    let token = SecretToken::parse(token).ok_or_else(invalid_invite)?;
    let row: Option<FoundInvite> = sqlx::query_as(
        "SELECT i.id, c.id, c.name, i.role,
                i.revoked_at IS NULL AND i.expires_at > ? AND i.uses < i.max_uses
         FROM invites i JOIN campaigns c ON c.id = i.campaign_id
         WHERE i.token_hash = ?",
    )
    .bind(db::now())
    .bind(token.hash())
    .fetch_optional(&state.db)
    .await?;
    let (invite_id, campaign_id, name, role, usable) = row.ok_or_else(invalid_invite)?;
    let role = Role::from_db(&role).ok_or_else(invalid_invite)?;
    Ok((invite_id, campaign_id, name, role, usable))
}

/// `GET /api/invites/{token}`: what the invite is for, before accepting.
async fn preview(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(token): Path<String>,
) -> Result<Json<InvitePreview>, AppError> {
    let (_, campaign_id, campaign_name, role, usable) = find(&state, &token).await?;
    let already_member = role_in(&state, current.user.id, campaign_id)
        .await?
        .is_some();
    if !usable && !already_member {
        return Err(AppError::NotFound(INVALID_INVITE.to_owned()));
    }
    Ok(Json(InvitePreview {
        campaign_id,
        campaign_name,
        role,
        already_member,
    }))
}

/// `POST /api/invites/{token}/accept`: join the campaign (CMP-005, CMP-006).
///
/// Accepting when already a member changes nothing and doesn't use up the
/// invite. Otherwise, counting the use and adding the membership happen in one
/// transaction, so a failure can't leave a used-up invite and no membership.
async fn accept(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(token): Path<String>,
) -> Result<Json<CampaignSummary>, AppError> {
    let (invite_id, campaign_id, name, role, usable) = find(&state, &token).await?;
    if let Some(existing) = role_in(&state, current.user.id, campaign_id).await? {
        return Ok(Json(CampaignSummary {
            id: campaign_id,
            name,
            role: existing,
        }));
    }
    if !usable {
        return Err(AppError::NotFound(INVALID_INVITE.to_owned()));
    }

    let now = db::now();
    let mut tx = state.db.begin().await?;
    // Count the use only if the invite is still valid at this exact moment.
    // Another accept could have used the last slot since `find`.
    let counted = sqlx::query(
        "UPDATE invites SET uses = uses + 1
         WHERE id = ? AND revoked_at IS NULL AND expires_at > ? AND uses < max_uses",
    )
    .bind(invite_id)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if counted == 0 {
        return Err(AppError::NotFound(INVALID_INVITE.to_owned()));
    }
    sqlx::query(
        "INSERT INTO memberships (campaign_id, user_id, role, joined_at) VALUES (?, ?, ?, ?)",
    )
    .bind(campaign_id)
    .bind(current.user.id)
    .bind(role.as_str())
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        campaign_id,
        user_id = current.user.id,
        role = role.as_str(),
        "joined through invite"
    );
    Ok(Json(CampaignSummary {
        id: campaign_id,
        name,
        role,
    }))
}
