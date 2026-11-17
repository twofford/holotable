//! The one place campaign access is decided (spec CMP-010).
//!
//! Every route under `/api/campaigns/{campaign_id}/...` sits behind the
//! `require_member` middleware. It resolves `(signed-in user, campaign) → role`
//! once, refuses anyone who isn't a member, and hands the result to the handler
//! as a `Member` value. A handler can't forget the check: without it there's no
//! `Member` to extract, and the handler wouldn't compile against the router.

use std::collections::HashMap;

use axum::{
    extract::{FromRequestParts, Path, Request, State},
    http::request::Parts,
    middleware::Next,
    response::Response,
};
use holotable_core::{api::UserInfo, campaign::Role};

use crate::{
    AppState,
    auth::extract::CurrentUser,
    error::{AppError, gm_only, not_found},
};

/// The signed-in user's membership in the campaign named in the URL.
#[derive(Debug, Clone)]
pub struct Member {
    pub user: UserInfo,
    pub campaign_id: i64,
    pub role: Role,
}

impl Member {
    /// For GM-only actions: `member.require_gm()?;` at the top of the handler.
    pub fn require_gm(&self) -> Result<(), AppError> {
        if self.role == Role::Gm {
            Ok(())
        } else {
            Err(gm_only())
        }
    }
}

/// Middleware for every campaign route.
///
/// A middleware function can take extractors before the `Request`, like a
/// handler. `CurrentUser` returns 401 for anyone signed out. A user who isn't a
/// member gets 404, not 403, so they can't even tell the campaign exists
/// (spec CMP-011).
pub async fn require_member(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(params): Path<HashMap<String, String>>,
    mut req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let campaign_id: i64 = params
        .get("campaign_id")
        .and_then(|s| s.parse().ok())
        .ok_or_else(not_found)?;
    let role = role_in(&state, current.user.id, campaign_id)
        .await?
        .ok_or_else(not_found)?;
    // Extensions are a type-keyed map carried with the request; the
    // `Member` extractor below reads the value back out.
    req.extensions_mut().insert(Member {
        user: current.user,
        campaign_id,
        role,
    });
    Ok(next.run(req).await)
}

/// The user's role in a campaign, or `None` if they aren't a member.
pub async fn role_in(
    state: &AppState,
    user_id: i64,
    campaign_id: i64,
) -> Result<Option<Role>, AppError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM memberships WHERE campaign_id = ? AND user_id = ?")
            .bind(campaign_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;
    // `Option::map` + `and_then` turn Option<String> into Option<Role>.
    Ok(role.as_deref().and_then(Role::from_db))
}

/// Lets handlers take `member: Member` directly.
///
/// If a handler that asks for `Member` were ever mounted outside the
/// middleware, this returns a 500 rather than letting the request through.
impl<S: Send + Sync> FromRequestParts<S> for Member {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Member>().cloned().ok_or_else(|| {
            AppError::Internal("campaign route is missing require_member".to_owned())
        })
    }
}
