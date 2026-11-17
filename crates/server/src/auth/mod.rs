//! Accounts: sign-up, sign-in, sign-out, and account settings (milestone M2).
//!
//! | Method | Path                    | Body                    | Success          |
//! |--------|-------------------------|-------------------------|------------------|
//! | POST   | `/api/auth/signup`      | `SignUpRequest`         | 201 + `UserInfo` |
//! | POST   | `/api/auth/signin`      | `SignInRequest`         | 200 + `UserInfo` |
//! | POST   | `/api/auth/signout`     | none                    | 204              |
//! | GET    | `/api/auth/me`          | none                    | 200 + `UserInfo` |
//! | PATCH  | `/api/account`          | `UpdateProfileRequest`  | 200 + `UserInfo` |
//! | POST   | `/api/account/password` | `ChangePasswordRequest` | 204              |
//!
//! Errors are `ApiError` JSON (see `error.rs`).

pub mod extract;
pub mod password;
pub mod rate_limit;
pub mod session;

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, patch, post},
};
use axum_extra::extract::CookieJar;
use holotable_core::api::{
    ChangePasswordRequest, SignInRequest, SignUpRequest, UpdateProfileRequest, UserInfo,
    check_new_password, normalize_display_name, normalize_email,
};

use self::extract::{ClientIp, CurrentUser};
use crate::{
    AppState, db,
    error::{AppError, invalid},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/auth/signup", post(sign_up))
        .route("/api/auth/signin", post(sign_in))
        .route("/api/auth/signout", post(sign_out))
        .route("/api/auth/me", get(me))
        .route("/api/account", patch(update_profile))
        .route("/api/account/password", post(change_password))
}

/// `POST /api/auth/signup`: creates an account and signs it in (ACC-001).
///
/// Handler arguments are extractors, run in order. `Json<T>` must be last,
/// because it consumes the request body.
async fn sign_up(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    Json(req): Json<SignUpRequest>,
) -> Result<impl IntoResponse, AppError> {
    limit(&state, &format!("signup-ip:{ip:?}"))?;
    let email = normalize_email(&req.email).map_err(invalid)?;
    let display_name = normalize_display_name(&req.display_name).map_err(invalid)?;
    check_new_password(&req.password).map_err(invalid)?;

    let password_hash = password::hash(req.password).await?;
    let result = sqlx::query(
        "INSERT INTO users (email, password_hash, display_name, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&email)
    .bind(&password_hash)
    .bind(&display_name)
    .bind(db::now())
    .execute(&state.db)
    .await;

    // The UNIQUE constraint on `email` is the real duplicate check: checking
    // first and inserting second could race with another sign-up.
    let id = match result {
        Ok(done) => done.last_insert_rowid(),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => {
            return Err(AppError::EmailTaken);
        }
        Err(e) => return Err(e.into()),
    };
    tracing::info!(user_id = id, "account created");

    let token = session::create(&state.db, id).await?;
    let jar = jar.add(session::cookie(&token, state.config.secure_cookies));
    Ok((
        StatusCode::CREATED,
        jar,
        Json(UserInfo {
            id,
            email,
            display_name,
        }),
    ))
}

/// `POST /api/auth/signin` (ACC-003, ACC-004).
async fn sign_in(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    jar: CookieJar,
    Json(req): Json<SignInRequest>,
) -> Result<impl IntoResponse, AppError> {
    // A badly formed email can't match an account, but it still goes through
    // the same steps, so the response gives nothing away.
    let email = normalize_email(&req.email).unwrap_or_default();
    limit(&state, &format!("signin-ip:{ip:?}"))?;
    limit(&state, &format!("signin-email:{email}"))?;

    let row: Option<(i64, String, String)> =
        sqlx::query_as("SELECT id, password_hash, display_name FROM users WHERE email = ?")
            .bind(&email)
            .fetch_optional(&state.db)
            .await?;

    // Split the Option into two: the stored hash goes to `verify` (which uses a
    // dummy hash when there's none), and the id and name are kept for later.
    let (account, stored_hash) = match row {
        Some((id, hash, name)) => (Some((id, name)), Some(hash)),
        None => (None, None),
    };
    let ok = password::verify(req.password, stored_hash).await?;
    let Some((id, display_name)) = account.filter(|_| ok) else {
        return Err(AppError::BadCredentials);
    };

    let token = session::create(&state.db, id).await?;
    tracing::info!(user_id = id, "signed in");
    let jar = jar.add(session::cookie(&token, state.config.secure_cookies));
    Ok((
        jar,
        Json(UserInfo {
            id,
            email,
            display_name,
        }),
    ))
}

/// `POST /api/auth/signout` (ACC-006). Ends this session only; always clears the cookie.
async fn sign_out(
    State(state): State<AppState>,
    jar: CookieJar,
) -> Result<impl IntoResponse, AppError> {
    if let Some(token) = extract::session_token(&jar) {
        session::delete(&state.db, &token).await?;
        // Close this session's live connections too.
        state.hub.close_session(&token.hash());
    }
    let jar = jar.add(session::removal_cookie(state.config.secure_cookies));
    Ok((StatusCode::NO_CONTENT, jar))
}

/// `GET /api/auth/me`: who am I? 401 if not signed in.
async fn me(current: CurrentUser) -> Json<UserInfo> {
    Json(current.user)
}

/// `PATCH /api/account`: change the display name (ACC-008).
async fn update_profile(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<UserInfo>, AppError> {
    let display_name = normalize_display_name(&req.display_name).map_err(invalid)?;
    sqlx::query("UPDATE users SET display_name = ? WHERE id = ?")
        .bind(&display_name)
        .bind(current.user.id)
        .execute(&state.db)
        .await?;
    Ok(Json(UserInfo {
        display_name,
        ..current.user
    }))
}

/// `POST /api/account/password` (ACC-008). Signs out every other session, so
/// a changed password also locks out anyone who had it.
async fn change_password(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AppError> {
    limit(&state, &format!("password-user:{}", current.user.id))?;
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = ?")
        .bind(current.user.id)
        .fetch_one(&state.db)
        .await?;
    if !password::verify(req.current_password, Some(stored)).await? {
        return Err(AppError::BadCredentials);
    }
    check_new_password(&req.new_password).map_err(invalid)?;

    let new_hash = password::hash(req.new_password).await?;
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
        .bind(new_hash)
        .bind(current.user.id)
        .execute(&state.db)
        .await?;
    let ended = session::delete_others(&state.db, current.user.id, &current.token).await?;
    state
        .hub
        .close_other_sessions(current.user.id, &current.token.hash());
    tracing::info!(
        user_id = current.user.id,
        other_sessions_ended = ended,
        "password changed"
    );
    Ok(StatusCode::NO_CONTENT)
}

fn limit(state: &AppState, key: &str) -> Result<(), AppError> {
    if state.auth_limiter.allow(key) {
        Ok(())
    } else {
        Err(AppError::RateLimited)
    }
}
