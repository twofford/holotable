//! JSON types for campaigns, members, invites, assets and scenes (M3), shared
//! with the client through ts-rs, plus the limits both sides use.
//!
//! Ids and times are `i64` in Rust. ts-rs would turn those into `bigint`,
//! which JSON can't carry, so each one is marked `#[ts(type = "number")]`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::uvtt::UvttMap;

const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

/// Largest `.uvtt` / `.dd2vtt` upload (spec AST-002).
pub const MAX_UVTT_BYTES: usize = 50 * 1024 * 1024;
/// Largest image upload (spec AST-002).
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// Total upload space per campaign (spec AST-002).
pub const CAMPAIGN_QUOTA_BYTES: i64 = 1024 * 1024 * 1024;
/// Largest picture accepted, in pixels. Decoding needs 4 bytes per pixel, so
/// this caps the server's memory use per upload at about 400 MB.
pub const MAX_IMAGE_PIXELS: u64 = 100_000_000;
/// Pictures wider or taller than this are cut into tiles (spec AST-005).
pub const TILE_ABOVE_PX: u32 = 4096;
/// Tile size, in pixels.
pub const TILE_SIZE_PX: u32 = 2048;
pub const MAX_CAMPAIGN_NAME_CHARS: usize = 80;
/// Longest an invite can last, and the default.
pub const MAX_INVITE_DAYS: u32 = 30;
pub const DEFAULT_INVITE_DAYS: u32 = 7;
pub const MAX_INVITE_USES: u32 = 100;

/// A member's role in one campaign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = EXPORT_DIR)]
pub enum Role {
    Gm,
    Player,
}

impl Role {
    /// The value stored in the database's `role` columns.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Gm => "gm",
            Role::Player => "player",
        }
    }

    pub fn from_db(s: &str) -> Option<Role> {
        match s {
            "gm" => Some(Role::Gm),
            "player" => Some(Role::Player),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct CampaignSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// The current user's role.
    pub role: Role,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct CampaignDetail {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// The current user's role.
    pub role: Role,
    pub members: Vec<MemberInfo>,
    pub scenes: Vec<SceneSummary>,
    /// The scene the GM has put everyone on, if any (MAP-010).
    #[ts(type = "number | null")]
    pub active_scene_id: Option<i64>,
}

/// Sets (or, with `null`, clears) the campaign's active scene.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct SetActiveSceneRequest {
    #[ts(type = "number | null")]
    pub scene_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct MemberInfo {
    #[ts(type = "number")]
    pub user_id: i64,
    pub display_name: String,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct CampaignNameRequest {
    pub name: String,
}

/// Deleting a campaign needs its name typed again, so it can't happen by accident.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct DeleteCampaignRequest {
    pub confirm_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct ChangeRoleRequest {
    pub role: Role,
}

/// `#[serde(default)]` lets the client leave a field out, and
/// `#[ts(optional)]` makes it `field?: number` in TypeScript to match.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct CreateInviteRequest {
    pub role: Role,
    /// Default 7, at most 30.
    #[serde(default)]
    #[ts(optional)]
    pub expires_in_days: Option<u32>,
    /// Default 1, at most 100.
    #[serde(default)]
    #[ts(optional)]
    pub max_uses: Option<u32>,
}

/// An invite as the GM sees it. The token isn't here: it's shown once, at creation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct InviteInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub role: Role,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub expires_at: i64,
    pub max_uses: u32,
    pub uses: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct CreatedInvite {
    pub invite: InviteInfo,
    /// The secret for the link `/invite/<token>`. The server keeps only its hash.
    pub token: String,
}

/// What someone opening an invite link sees before accepting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct InvitePreview {
    #[ts(type = "number")]
    pub campaign_id: i64,
    pub campaign_name: String,
    pub role: Role,
    pub already_member: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = EXPORT_DIR)]
pub enum AssetKind {
    Uvtt,
    Image,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct AssetInfo {
    #[ts(type = "number")]
    pub id: i64,
    pub kind: AssetKind,
    /// The uploaded file's name. Display only.
    pub name: String,
    #[ts(type = "number")]
    pub size_bytes: i64,
    pub width: u32,
    pub height: u32,
    #[ts(type = "number")]
    pub created_at: i64,
    /// Names of the scenes that use this asset. It can't be deleted while any do.
    pub used_by: Vec<String>,
}

/// One piece of a scene's picture, in image pixels.
/// Fetch it from `/api/campaigns/<id>/assets/<asset id>/tiles/<index>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct TileInfo {
    pub index: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct SceneSummary {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number")]
    pub asset_id: i64,
}

/// Everything the client needs to draw a scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct SceneDetail {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    #[ts(type = "number")]
    pub asset_id: i64,
    /// The scene's grid, in image pixels. May differ from the UVTT file's.
    pub pixels_per_grid: f64,
    pub grid_offset_x: f64,
    pub grid_offset_y: f64,
    pub image_width: u32,
    pub image_height: u32,
    /// The picture, in one or more tiles.
    pub tiles: Vec<TileInfo>,
    /// Walls, doors and lights. `image` is empty: the picture comes as tiles.
    pub map: UvttMap,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct CreateSceneRequest {
    pub name: String,
    #[ts(type = "number")]
    pub asset_id: i64,
}

/// Changes to a scene. Fields left out stay as they are.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct UpdateSceneRequest {
    #[serde(default)]
    #[ts(optional)]
    pub name: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub pixels_per_grid: Option<f64>,
    #[serde(default)]
    #[ts(optional)]
    pub grid_offset_x: Option<f64>,
    #[serde(default)]
    #[ts(optional)]
    pub grid_offset_y: Option<f64>,
}

/// Trims a campaign or scene name and checks its length.
pub fn normalize_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        Err("Enter a name.")
    } else if name.chars().count() > MAX_CAMPAIGN_NAME_CHARS {
        Err("Use 80 characters or fewer for the name.")
    } else if name.chars().any(char::is_control) {
        Err("The name can't contain control characters.")
    } else {
        Ok(name.to_owned())
    }
}

/// The tiles for a picture of `width` × `height` pixels: one tile covering the
/// whole picture if it's small enough, otherwise a grid of TILE_SIZE_PX tiles,
/// row by row. Server and client both use this, so they always agree.
pub fn tile_layout(width: u32, height: u32) -> Vec<TileInfo> {
    if width <= TILE_ABOVE_PX && height <= TILE_ABOVE_PX {
        return vec![TileInfo {
            index: 0,
            x: 0,
            y: 0,
            width,
            height,
        }];
    }
    let mut tiles = Vec::new();
    // `step_by` walks 0, 2048, 4096, ... up to the edge.
    for y in (0..height).step_by(TILE_SIZE_PX as usize) {
        for x in (0..width).step_by(TILE_SIZE_PX as usize) {
            tiles.push(TileInfo {
                index: tiles.len() as u32,
                x,
                y,
                width: TILE_SIZE_PX.min(width - x),
                height: TILE_SIZE_PX.min(height - y),
            });
        }
    }
    tiles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_round_trips_through_the_database_form() {
        for role in [Role::Gm, Role::Player] {
            assert_eq!(Role::from_db(role.as_str()), Some(role));
        }
        assert_eq!(Role::from_db("admin"), None);
        assert_eq!(serde_json::to_string(&Role::Gm).unwrap(), r#""gm""#);
    }

    #[test]
    fn small_pictures_are_one_tile() {
        assert_eq!(
            tile_layout(4096, 100),
            vec![TileInfo {
                index: 0,
                x: 0,
                y: 0,
                width: 4096,
                height: 100
            }]
        );
    }

    #[test]
    fn large_pictures_are_cut_into_a_grid() {
        let tiles = tile_layout(5600, 4200);
        assert_eq!(tiles.len(), 9); // 3 columns (2048, 2048, 1504) × 3 rows (2048, 2048, 104)
        assert_eq!(
            tiles[2],
            TileInfo {
                index: 2,
                x: 4096,
                y: 0,
                width: 1504,
                height: 2048
            }
        );
        assert_eq!(
            tiles[8],
            TileInfo {
                index: 8,
                x: 4096,
                y: 4096,
                width: 1504,
                height: 104
            }
        );
        // Tiles cover every pixel exactly once.
        let area: u64 = tiles
            .iter()
            .map(|t| u64::from(t.width) * u64::from(t.height))
            .sum();
        assert_eq!(area, 5600 * 4200);
    }

    #[test]
    fn invite_request_fields_are_optional() {
        let req: CreateInviteRequest = serde_json::from_str(r#"{"role":"player"}"#).unwrap();
        assert_eq!(req.role, Role::Player);
        assert_eq!(req.expires_in_days, None);
    }

    #[test]
    fn names_are_checked() {
        assert_eq!(normalize_name("  Neon Nights ").unwrap(), "Neon Nights");
        assert!(normalize_name("").is_err());
        assert!(normalize_name(&"x".repeat(81)).is_err());
    }
}
