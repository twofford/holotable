//! Leaving, changing roles, removing members (CMP-007, CMP-008).
//!
//! A campaign must always keep at least one GM. Checking "is there another
//! GM?" and then changing the role in a second query could race: two GMs
//! demoting each other at the same moment would both see the other and both
//! succeed. So each change is a single SQL statement that includes the check.
//! SQLite runs one write statement at a time, which makes the check and the
//! change one indivisible step.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{patch, post},
};
use holotable_core::campaign::{ChangeRoleRequest, MemberInfo, Role};

use super::access::Member;
use crate::{
    AppState,
    error::{AppError, not_found},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/leave", post(leave))
        .route("/members/{user_id}", patch(change_role).delete(remove))
}

/// SQL that matches a membership row only if removing or demoting it leaves a
/// GM behind.
///
/// It's a macro, not a `const`, so that `concat!` can glue it into the
/// queries below at compile time. sqlx 0.9 only accepts SQL that's a
/// `&'static str` (a literal), and refuses strings built at runtime with
/// `format!`, because pasting values into SQL text is how SQL injection
/// happens. `concat!` of literals is itself a literal, so it's allowed.
macro_rules! keeps_a_gm {
    () => {
        "(role != 'gm' OR (SELECT COUNT(*) FROM memberships WHERE campaign_id = ?1 AND role = 'gm') > 1)"
    };
}

const LAST_GM: &str = "A campaign needs at least one GM. Make someone else a GM first.";

/// `POST /api/campaigns/{id}/leave`: leave the campaign.
async fn leave(State(state): State<AppState>, member: Member) -> Result<StatusCode, AppError> {
    remove_membership(&state, member.campaign_id, member.user.id).await?;
    state.hub.kick_from_campaign(
        member.campaign_id,
        member.user.id,
        "You left this campaign.",
        false,
    );
    tracing::info!(
        campaign_id = member.campaign_id,
        user_id = member.user.id,
        "left campaign"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/campaigns/{id}/members/{user_id}` (GM): remove a member.
/// They're taken out of the campaign's scene rooms at once (CMP-007).
async fn remove(
    State(state): State<AppState>,
    member: Member,
    // The nested path has two parameters; the first is the campaign id, which
    // `Member` already holds, so `_` ignores it here.
    Path((_, user_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    member.require_gm()?;
    remove_membership(&state, member.campaign_id, user_id).await?;
    state.hub.kick_from_campaign(
        member.campaign_id,
        user_id,
        "You were removed from this campaign.",
        false,
    );
    tracing::info!(
        campaign_id = member.campaign_id,
        removed_user_id = user_id,
        by = member.user.id,
        "member removed"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `PATCH /api/campaigns/{id}/members/{user_id}` (GM): make someone GM or player.
async fn change_role(
    State(state): State<AppState>,
    member: Member,
    Path((_, user_id)): Path<(i64, i64)>,
    Json(req): Json<ChangeRoleRequest>,
) -> Result<Json<MemberInfo>, AppError> {
    member.require_gm()?;
    // Promoting is always fine. Demoting must leave another GM.
    const SQL: &str = concat!(
        "UPDATE memberships SET role = ?3 WHERE campaign_id = ?1 AND user_id = ?2 AND (?3 = 'gm' OR ",
        keeps_a_gm!(),
        ")"
    );
    let changed = sqlx::query(SQL)
        .bind(member.campaign_id)
        .bind(user_id)
        .bind(req.role.as_str())
        .execute(&state.db)
        .await?
        .rows_affected();
    if changed == 0 {
        return Err(why_not(&state, member.campaign_id, user_id).await?);
    }
    // Their open scenes were joined with the old role. Send them out; the
    // client joins again and gets what the new role may see.
    state.hub.kick_from_campaign(
        member.campaign_id,
        user_id,
        "Your role in this campaign changed.",
        true,
    );
    let display_name: String = sqlx::query_scalar("SELECT display_name FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(MemberInfo {
        user_id,
        display_name,
        role: req.role,
    }))
}

async fn remove_membership(
    state: &AppState,
    campaign_id: i64,
    user_id: i64,
) -> Result<(), AppError> {
    const SQL: &str = concat!(
        "DELETE FROM memberships WHERE campaign_id = ?1 AND user_id = ?2 AND ",
        keeps_a_gm!()
    );
    let removed = sqlx::query(SQL)
        .bind(campaign_id)
        .bind(user_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if removed == 0 {
        Err(why_not(state, campaign_id, user_id).await?)
    } else {
        Ok(())
    }
}

/// A guarded statement changed nothing: either there's no such member, or the
/// change would have removed the last GM.
async fn why_not(state: &AppState, campaign_id: i64, user_id: i64) -> Result<AppError, AppError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE campaign_id = ? AND user_id = ?")
            .bind(campaign_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    Ok(match role.as_deref().and_then(Role::from_db) {
        None => not_found(),
        Some(_) => AppError::Conflict(LAST_GM.to_owned()),
    })
}
