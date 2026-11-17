//! Uploaded maps and images (AST-001 to AST-008).
//!
//! An upload is the raw file as the request body: `POST /assets?name=<file name>`.
//! The server works out what it is from its first bytes, not from the name or
//! the browser's Content-Type, both of which the uploader controls (AST-003).

use std::io::Cursor;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State, rejection::BytesRejection},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{delete, get},
};
use holotable_core::{
    campaign::{
        AssetInfo, AssetKind, CAMPAIGN_QUOTA_BYTES, MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS,
        MAX_UVTT_BYTES, tile_layout,
    },
    uvtt::{ImageFormat, UvttMap, image_info},
};
use serde::Deserialize;

use super::access::Member;
use crate::{
    AppState, db,
    error::{AppError, invalid, not_found},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/assets",
            // The default JSON body limit is 2 MB; uploads get their own,
            // larger limit. Past it, axum stops reading and answers 413.
            get(list)
                .post(upload)
                .layer(DefaultBodyLimit::max(MAX_UVTT_BYTES)),
        )
        .route("/assets/{asset_id}", delete(remove))
        .route("/assets/{asset_id}/tiles/{index}", get(tile))
}

#[derive(Deserialize)]
struct UploadQuery {
    name: Option<String>,
}

/// What the image check found, and the tiles it made.
struct CheckedImage {
    format: ImageFormat,
    width: u32,
    height: u32,
    /// PNG-encoded tiles, in `tile_layout` order. Empty when the picture is small
    /// enough to serve whole.
    tiles: Vec<Vec<u8>>,
}

/// `POST /api/campaigns/{id}/assets?name=…` (GM): upload a map or an image.
///
/// `Result<Bytes, BytesRejection>` lets the handler see a body-too-large
/// rejection and answer with our own JSON error instead of axum's plain text.
async fn upload(
    State(state): State<AppState>,
    member: Member,
    Query(query): Query<UploadQuery>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<AssetInfo>), AppError> {
    member.require_gm()?;
    let body = body.map_err(|e| {
        if e.status() == StatusCode::PAYLOAD_TOO_LARGE {
            AppError::TooLarge("Maps can be up to 50 MB.".to_owned())
        } else {
            invalid("The upload couldn't be read.")
        }
    })?;
    let name = display_name(query.name.as_deref());

    // Decide what was uploaded from its content (AST-003).
    let (kind, picture, map) = if looks_like_json(&body) {
        let text =
            std::str::from_utf8(&body).map_err(|_| invalid("This map file isn't valid text."))?;
        // The Rust UVTT parser does the schema check (AST-004).
        let map = UvttMap::parse(text).map_err(|e| AppError::InvalidInput(e.to_string()))?;
        let picture = map
            .image_bytes()
            .map_err(|e| AppError::InvalidInput(e.to_string()))?;
        (AssetKind::Uvtt, picture, Some(map))
    } else if image_info(&body).is_ok() {
        if body.len() > MAX_IMAGE_BYTES {
            return Err(AppError::TooLarge("Images can be up to 20 MB.".to_owned()));
        }
        (AssetKind::Image, body.to_vec(), None)
    } else {
        return Err(invalid(
            "Upload a .uvtt or .dd2vtt map, or a PNG, JPEG or WebP image.",
        ));
    };

    // Decoding and tiling a large picture takes real CPU time, so it runs on
    // the blocking pool. `move` hands `picture` to that thread; the closure
    // gives it back in the result, so it can still be written to disk below.
    let (picture, checked) = tokio::task::spawn_blocking(move || {
        let checked = check_and_tile(&picture);
        (picture, checked)
    })
    .await?;
    let checked = checked?;

    // Everything this asset will occupy on disk, for the quota (AST-002).
    let map_json = match &map {
        // `..map.clone()` copies every other field; the picture is stored separately.
        Some(m) => Some(
            serde_json::to_vec(&UvttMap {
                image: String::new(),
                ..m.clone()
            })
            .map_err(|e| AppError::Internal(e.to_string()))?,
        ),
        None => None,
    };
    let size_bytes = (picture.len()
        + map_json.as_ref().map_or(0, Vec::len)
        + checked.tiles.iter().map(Vec::len).sum::<usize>()) as i64;
    let used: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes), 0) FROM assets WHERE campaign_id = ?")
            .bind(member.campaign_id)
            .fetch_one(&state.db)
            .await?;
    if used + size_bytes > CAMPAIGN_QUOTA_BYTES {
        return Err(AppError::TooLarge(
            "This campaign is out of upload space (1 GB). Delete some uploads first.".to_owned(),
        ));
    }

    let tile_count = checked.tiles.len() as i64;
    let id = sqlx::query(
        "INSERT INTO assets (campaign_id, kind, original_name, size_bytes, image_format, width, height, tile_count, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(member.campaign_id)
    .bind(kind_str(kind))
    .bind(&name)
    .bind(size_bytes)
    .bind(extension(checked.format))
    .bind(checked.width)
    .bind(checked.height)
    .bind(tile_count)
    .bind(member.user.id)
    .bind(db::now())
    .execute(&state.db)
    .await?
    .last_insert_rowid();

    // Now the files. If any write fails, undo the row and whatever was written,
    // so the database never points at files that aren't there.
    let written = write_files(
        &state,
        member.campaign_id,
        id,
        &checked,
        &picture,
        map_json.as_deref(),
    )
    .await;
    if let Err(e) = written {
        let _ = sqlx::query("DELETE FROM assets WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await;
        let _ = state.storage.remove_asset(member.campaign_id, id).await;
        return Err(e.into());
    }

    tracing::info!(
        campaign_id = member.campaign_id,
        asset_id = id,
        size_bytes,
        tiles = tile_count,
        "asset uploaded"
    );
    let info = AssetInfo {
        id,
        kind,
        name,
        size_bytes,
        width: checked.width,
        height: checked.height,
        created_at: db::now(),
        used_by: Vec::new(),
    };
    Ok((StatusCode::CREATED, Json(info)))
}

async fn write_files(
    state: &AppState,
    campaign_id: i64,
    asset_id: i64,
    checked: &CheckedImage,
    picture: &[u8],
    map_json: Option<&[u8]>,
) -> std::io::Result<()> {
    let storage = &state.storage;
    storage
        .write(
            &storage.image_path(campaign_id, asset_id, extension(checked.format)),
            picture,
        )
        .await?;
    if let Some(json) = map_json {
        storage
            .write(&storage.map_path(campaign_id, asset_id), json)
            .await?;
    }
    for (index, tile) in checked.tiles.iter().enumerate() {
        storage
            .write(
                &storage.tile_path(campaign_id, asset_id, index as u32),
                tile,
            )
            .await?;
    }
    Ok(())
}

/// Decodes the picture to prove it's a real image, and cuts it into tiles if
/// it's large. Runs on a blocking thread.
fn check_and_tile(bytes: &[u8]) -> Result<CheckedImage, AppError> {
    let unreadable =
        |e: image::ImageError| AppError::InvalidInput(format!("The image can't be read: {e}"));
    let (format, _) =
        image_info(bytes).map_err(|_| invalid("The map's image isn't a PNG, JPEG or WebP."))?;
    let decoder_format = match format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
        ImageFormat::WebP => image::ImageFormat::WebP,
    };
    // A closure that builds a fresh reader each time: `into_dimensions` and
    // `decode` each consume theirs.
    let reader = || {
        let mut reader = image::ImageReader::with_format(Cursor::new(bytes), decoder_format);
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(512 * 1024 * 1024);
        reader.limits(limits);
        reader
    };

    // Read the size from the header first. A small file can claim to be a
    // gigapixel image (a "decompression bomb"); refuse that before decoding.
    let (width, height) = reader().into_dimensions().map_err(unreadable)?;
    if width == 0 || height == 0 {
        return Err(invalid("The image is empty."));
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(AppError::TooLarge(
            "Images can be up to 100 megapixels.".to_owned(),
        ));
    }

    let image = reader().decode().map_err(unreadable)?;
    let layout = tile_layout(width, height);
    if layout.len() == 1 {
        return Ok(CheckedImage {
            format,
            width,
            height,
            tiles: Vec::new(),
        });
    }
    let mut tiles = Vec::with_capacity(layout.len());
    for t in &layout {
        let mut png = Cursor::new(Vec::new());
        image
            .crop_imm(t.x, t.y, t.width, t.height)
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|e| AppError::Internal(format!("tile encoding failed: {e}")))?;
        tiles.push(png.into_inner());
    }
    Ok(CheckedImage {
        format,
        width,
        height,
        tiles,
    })
}

/// `GET /api/campaigns/{id}/assets` (GM): uploads, with the scenes using each.
async fn list(
    State(state): State<AppState>,
    member: Member,
) -> Result<Json<Vec<AssetInfo>>, AppError> {
    member.require_gm()?;
    let rows: Vec<(i64, String, String, i64, u32, u32, i64)> = sqlx::query_as(
        "SELECT id, kind, original_name, size_bytes, width, height, created_at FROM assets
         WHERE campaign_id = ? ORDER BY created_at DESC, id DESC",
    )
    .bind(member.campaign_id)
    .fetch_all(&state.db)
    .await?;
    let uses: Vec<(i64, String)> = sqlx::query_as(
        "SELECT asset_id, name FROM scenes WHERE campaign_id = ? ORDER BY name COLLATE NOCASE",
    )
    .bind(member.campaign_id)
    .fetch_all(&state.db)
    .await?;

    let assets = rows
        .into_iter()
        .map(
            |(id, kind, name, size_bytes, width, height, created_at)| AssetInfo {
                id,
                kind: if kind == "uvtt" {
                    AssetKind::Uvtt
                } else {
                    AssetKind::Image
                },
                name,
                size_bytes,
                width,
                height,
                created_at,
                used_by: uses
                    .iter()
                    .filter(|(asset_id, _)| *asset_id == id)
                    .map(|(_, n)| n.clone())
                    .collect(),
            },
        )
        .collect();
    Ok(Json(assets))
}

/// `DELETE /api/campaigns/{id}/assets/{asset_id}` (GM). Refused while a scene
/// uses the asset (AST-008).
async fn remove(
    State(state): State<AppState>,
    member: Member,
    Path((_, asset_id)): Path<(i64, i64)>,
) -> Result<StatusCode, AppError> {
    member.require_gm()?;
    let users: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM scenes WHERE asset_id = ? AND campaign_id = ? ORDER BY name",
    )
    .bind(asset_id)
    .bind(member.campaign_id)
    .fetch_all(&state.db)
    .await?;
    if !users.is_empty() {
        return Err(AppError::Conflict(format!(
            "This map is used by: {}. Delete those scenes first.",
            users.join(", ")
        )));
    }
    // An image may be a token's picture.
    let token_scenes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT s.name FROM tokens t JOIN scenes s ON s.id = t.scene_id
         WHERE t.asset_id = ? AND s.campaign_id = ? ORDER BY s.name",
    )
    .bind(asset_id)
    .bind(member.campaign_id)
    .fetch_all(&state.db)
    .await?;
    if !token_scenes.is_empty() {
        return Err(AppError::Conflict(format!(
            "Tokens use this image in: {}. Change or delete those tokens first.",
            token_scenes.join(", ")
        )));
    }
    let removed = sqlx::query("DELETE FROM assets WHERE id = ? AND campaign_id = ?")
        .bind(asset_id)
        .bind(member.campaign_id)
        .execute(&state.db)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(not_found());
    }
    state
        .storage
        .remove_asset(member.campaign_id, asset_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /api/campaigns/{id}/assets/{asset_id}/tiles/{index}` (member): one
/// piece of the picture. A picture that wasn't cut up has a single tile, 0,
/// which is the original file (AST-007: members only, via `require_member`).
async fn tile(
    State(state): State<AppState>,
    member: Member,
    Path((_, asset_id, index)): Path<(i64, i64, u32)>,
) -> Result<impl IntoResponse, AppError> {
    let row: Option<(String, i64)> = sqlx::query_as(
        "SELECT image_format, tile_count FROM assets WHERE id = ? AND campaign_id = ?",
    )
    .bind(asset_id)
    .bind(member.campaign_id)
    .fetch_optional(&state.db)
    .await?;
    let (format, tile_count) = row.ok_or_else(not_found)?;
    let storage = &state.storage;
    let (path, content_type) = if tile_count == 0 && index == 0 {
        let content_type = match format.as_str() {
            "png" => "image/png",
            "jpeg" => "image/jpeg",
            _ => "image/webp",
        };
        (
            storage.image_path(member.campaign_id, asset_id, &format),
            content_type,
        )
    } else if i64::from(index) < tile_count {
        (
            storage.tile_path(member.campaign_id, asset_id, index),
            "image/png",
        )
    } else {
        return Err(not_found());
    };

    let bytes = tokio::fs::read(&path).await?;
    let headers = [
        (header::CONTENT_TYPE, content_type),
        // Uploads never change, so the browser can keep them for a year.
        // `private`: a shared cache (a proxy) must not keep a copy.
        (
            header::CACHE_CONTROL,
            "private, max-age=31536000, immutable",
        ),
    ];
    Ok((headers, bytes))
}

/// The uploader's file name, cleaned for display (AST-006): path stripped,
/// control characters removed, length capped.
fn display_name(raw: Option<&str>) -> String {
    let raw = raw.unwrap_or("");
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control()).take(200).collect();
    let clean = clean.trim();
    if clean.is_empty() {
        "upload".to_owned()
    } else {
        clean.to_owned()
    }
}

/// UVTT files are JSON objects: the first non-blank byte is `{`.
fn looks_like_json(bytes: &[u8]) -> bool {
    bytes.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'{')
}

fn kind_str(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Uvtt => "uvtt",
        AssetKind::Image => "image",
    }
}

/// The file extension, which is also what the `image_format` column stores.
fn extension(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::WebP => "webp",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_are_cleaned() {
        assert_eq!(display_name(Some("../../etc/passwd")), "passwd");
        assert_eq!(
            display_name(Some("C:\\maps\\Safehouse.uvtt")),
            "Safehouse.uvtt"
        );
        assert_eq!(display_name(Some("bad\u{0}name.png")), "badname.png");
        assert_eq!(display_name(None), "upload");
        assert_eq!(display_name(Some("   ")), "upload");
    }

    #[test]
    fn json_is_recognised_by_its_first_byte() {
        assert!(looks_like_json(b"  \n{\"format\":0.3}"));
        assert!(!looks_like_json(b"\x89PNG"));
        assert!(!looks_like_json(b""));
    }
}
