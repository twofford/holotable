//! Uploaded files on disk.
//!
//! Layout, under the data folder:
//!
//! ```text
//! campaigns/<campaign id>/assets/<asset id>/
//!     image.png | image.jpg | image.webp   the picture, as uploaded or as embedded in the UVTT
//!     map.json                             UVTT data without the picture (UVTT assets only)
//!     tile-0.png, tile-1.png, ...          tiles, when the picture is large
//! ```
//!
//! Names on disk come only from database ids, never from the uploader's file
//! name, so an upload can't choose where it's written (spec AST-006).

use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Storage {
    root: PathBuf,
}

impl Storage {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            root: data_dir.join("campaigns"),
        }
    }

    fn campaign_dir(&self, campaign_id: i64) -> PathBuf {
        self.root.join(campaign_id.to_string())
    }

    pub fn asset_dir(&self, campaign_id: i64, asset_id: i64) -> PathBuf {
        self.campaign_dir(campaign_id)
            .join("assets")
            .join(asset_id.to_string())
    }

    pub fn image_path(&self, campaign_id: i64, asset_id: i64, extension: &str) -> PathBuf {
        self.asset_dir(campaign_id, asset_id)
            .join(format!("image.{extension}"))
    }

    pub fn map_path(&self, campaign_id: i64, asset_id: i64) -> PathBuf {
        self.asset_dir(campaign_id, asset_id).join("map.json")
    }

    pub fn tile_path(&self, campaign_id: i64, asset_id: i64, index: u32) -> PathBuf {
        self.asset_dir(campaign_id, asset_id)
            .join(format!("tile-{index}.png"))
    }

    /// Writes a file in full, or not at all: write to a temporary name first,
    /// then rename. A crash halfway leaves a stray `.tmp`, never half a file
    /// under the real name.
    pub async fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await?;
        }
        let tmp = path.with_extension("tmp");
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }

    pub async fn remove_asset(&self, campaign_id: i64, asset_id: i64) -> io::Result<()> {
        remove_dir_if_present(&self.asset_dir(campaign_id, asset_id)).await
    }

    pub async fn remove_campaign(&self, campaign_id: i64) -> io::Result<()> {
        remove_dir_if_present(&self.campaign_dir(campaign_id)).await
    }
}

/// Deleting something that's already gone counts as success.
async fn remove_dir_if_present(dir: &Path) -> io::Result<()> {
    match tokio::fs::remove_dir_all(dir).await {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
