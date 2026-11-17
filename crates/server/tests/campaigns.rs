//! Integration tests for campaigns, members, invites, uploads and scenes (M3).

mod common;

use axum::{Router, http::StatusCode};
use common::{Reply, TestApp, req, sign_up_as, test_app};
use serde_json::json;

const SMALL_MAP: &[u8] = include_bytes!("../../../fixtures/small-bodega.uvtt");
const LARGE_MAP: &[u8] = include_bytes!("../../../fixtures/large-arcology.uvtt");

/// A campaign with a GM and (optionally) a player who joined by invite.
struct Table {
    t: TestApp,
    gm: String,
    player: String,
    outsider: String,
    campaign: i64,
}

impl Table {
    fn app(&self) -> &Router {
        &self.t.app
    }

    fn url(&self, rest: &str) -> String {
        format!("/api/campaigns/{}{rest}", self.campaign)
    }
}

async fn create_campaign(app: &Router, cookie: &str, name: &str) -> i64 {
    let reply = req("POST", "/api/campaigns")
        .cookie(cookie)
        .json(json!({ "name": name }))
        .send(app)
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
    reply.id()
}

async fn invite(app: &Router, gm: &str, campaign: i64, body: serde_json::Value) -> Reply {
    req("POST", format!("/api/campaigns/{campaign}/invites"))
        .cookie(gm)
        .json(body)
        .send(app)
        .await
}

async fn accept(app: &Router, cookie: &str, token: &str) -> Reply {
    req("POST", format!("/api/invites/{token}/accept"))
        .cookie(cookie)
        .send(app)
        .await
}

/// GM creates "Neon Nights"; a player joins through an invite; an outsider
/// has an account but isn't in the campaign.
async fn table() -> Table {
    let t = test_app().await;
    let gm = sign_up_as(&t.app, "gm@example.com", "Ghost").await;
    let player = sign_up_as(&t.app, "player@example.com", "Vex").await;
    let outsider = sign_up_as(&t.app, "outsider@example.com", "Nobody").await;
    let campaign = create_campaign(&t.app, &gm, "Neon Nights").await;
    let created = invite(&t.app, &gm, campaign, json!({ "role": "player" })).await;
    let token = created.body["token"].as_str().unwrap().to_owned();
    assert_eq!(accept(&t.app, &player, &token).await.status, StatusCode::OK);
    Table {
        t,
        gm,
        player,
        outsider,
        campaign,
    }
}

async fn upload(table: &Table, name: &str, bytes: &[u8]) -> Reply {
    req("POST", table.url(&format!("/assets?name={name}")))
        .cookie(&table.gm)
        .bytes(bytes.to_vec())
        .send(table.app())
        .await
}

#[tokio::test]
async fn creating_and_listing_campaigns() {
    let tb = table().await;
    let gm_list = req("GET", "/api/campaigns")
        .cookie(&tb.gm)
        .send(tb.app())
        .await;
    assert_eq!(
        gm_list.body,
        json!([{ "id": tb.campaign, "name": "Neon Nights", "role": "gm" }])
    );

    let player_list = req("GET", "/api/campaigns")
        .cookie(&tb.player)
        .send(tb.app())
        .await;
    assert_eq!(player_list.body[0]["role"], "player");

    let outsider_list = req("GET", "/api/campaigns")
        .cookie(&tb.outsider)
        .send(tb.app())
        .await;
    assert_eq!(outsider_list.body, json!([]));

    let detail = req("GET", tb.url(""))
        .cookie(&tb.player)
        .send(tb.app())
        .await;
    assert_eq!(detail.status, StatusCode::OK);
    assert_eq!(detail.body["role"], "player");
    // GMs are listed first.
    assert_eq!(detail.body["members"][0]["displayName"], "Ghost");
    assert_eq!(detail.body["members"][1]["role"], "player");
}

/// Every route under /api/campaigns/{id}: one entry per method and path.
fn campaign_routes(campaign: i64) -> Vec<(&'static str, String)> {
    let c = format!("/api/campaigns/{campaign}");
    vec![
        ("GET", c.clone()),
        ("PATCH", c.clone()),
        ("DELETE", c.clone()),
        ("POST", format!("{c}/leave")),
        ("PATCH", format!("{c}/members/1")),
        ("DELETE", format!("{c}/members/1")),
        ("GET", format!("{c}/invites")),
        ("POST", format!("{c}/invites")),
        ("DELETE", format!("{c}/invites/1")),
        ("GET", format!("{c}/assets")),
        ("POST", format!("{c}/assets?name=x")),
        ("DELETE", format!("{c}/assets/1")),
        ("GET", format!("{c}/assets/1/tiles/0")),
        ("POST", format!("{c}/scenes")),
        ("GET", format!("{c}/scenes/1")),
        ("PATCH", format!("{c}/scenes/1")),
        ("DELETE", format!("{c}/scenes/1")),
    ]
}

/// CMP-010 and CMP-011: every campaign route refuses outsiders with 404 (and
/// signed-out visitors with 401) before doing anything.
#[tokio::test]
async fn every_campaign_route_checks_membership() {
    let tb = table().await;
    for (method, path) in campaign_routes(tb.campaign) {
        let outsider = req(method, &path)
            .cookie(&tb.outsider)
            .json(json!({}))
            .send(tb.app())
            .await;
        assert_eq!(
            outsider.status,
            StatusCode::NOT_FOUND,
            "{method} {path} let an outsider in"
        );
        let anonymous = req(method, &path).json(json!({})).send(tb.app()).await;
        assert_eq!(
            anonymous.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} let a signed-out visitor in"
        );
    }
    // A campaign that doesn't exist looks the same as one you're not in.
    let missing = req("GET", "/api/campaigns/999999")
        .cookie(&tb.gm)
        .send(tb.app())
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn players_cannot_do_gm_things() {
    let tb = table().await;
    let gm_only = [
        req("PATCH", tb.url("")).json(json!({ "name": "Mine now" })),
        req("DELETE", tb.url("")).json(json!({ "confirmName": "Neon Nights" })),
        req("PATCH", tb.url("/members/1")).json(json!({ "role": "player" })),
        req("DELETE", tb.url("/members/1")),
        req("GET", tb.url("/invites")),
        req("POST", tb.url("/invites")).json(json!({ "role": "gm" })),
        req("GET", tb.url("/assets")),
        req("POST", tb.url("/assets?name=x")).bytes(SMALL_MAP.to_vec()),
        req("POST", tb.url("/scenes")).json(json!({ "name": "x", "assetId": 1 })),
    ];
    for r in gm_only {
        let reply = r.cookie(&tb.player).send(tb.app()).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{}", reply.body);
        assert_eq!(reply.error_code(), "forbidden");
    }
}

#[tokio::test]
async fn invite_links_work_once_and_can_be_revoked() {
    let tb = table().await;
    let app = tb.app();
    let created = invite(
        app,
        &tb.gm,
        tb.campaign,
        json!({ "role": "gm", "maxUses": 1, "expiresInDays": 2 }),
    )
    .await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["invite"]["role"], "gm");
    let token = created.body["token"].as_str().unwrap().to_owned();
    let expires = created.body["invite"]["expiresAt"].as_i64().unwrap()
        - created.body["invite"]["createdAt"].as_i64().unwrap();
    assert_eq!(expires, 2 * 86_400);

    // The token is stored only as a hash.
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM invites")
        .fetch_all(&tb.t.db)
        .await
        .unwrap();
    assert!(
        stored
            .iter()
            .all(|h| h.len() == 32 && h != token.as_bytes())
    );

    let preview = req("GET", format!("/api/invites/{token}"))
        .cookie(&tb.outsider)
        .send(app)
        .await;
    assert_eq!(preview.body["campaignName"], "Neon Nights");
    assert_eq!(preview.body["alreadyMember"], false);

    let joined = accept(app, &tb.outsider, &token).await;
    assert_eq!(joined.body["role"], "gm");
    // Accepting again changes nothing and doesn't fail.
    assert_eq!(
        accept(app, &tb.outsider, &token).await.status,
        StatusCode::OK
    );

    // Used up: nobody else can use it.
    let late = sign_up_as(app, "late@example.com", "Late").await;
    let refused = accept(app, &late, &token).await;
    assert_eq!(refused.status, StatusCode::NOT_FOUND);
    assert!(
        refused.body["message"]
            .as_str()
            .unwrap()
            .contains("expired, been used up, or been revoked")
    );

    // Revoked and expired invites don't work either.
    let revocable = invite(app, &tb.gm, tb.campaign, json!({ "role": "player" })).await;
    let id = revocable.body["invite"]["id"].as_i64().unwrap();
    let open = req("GET", tb.url("/invites"))
        .cookie(&tb.gm)
        .send(app)
        .await;
    assert_eq!(
        open.body.as_array().unwrap().len(),
        1,
        "only the unused invite is open"
    );
    assert_eq!(
        req("DELETE", tb.url(&format!("/invites/{id}")))
            .cookie(&tb.gm)
            .send(app)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        accept(app, &late, revocable.body["token"].as_str().unwrap())
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    let expiring = invite(app, &tb.gm, tb.campaign, json!({ "role": "player" })).await;
    sqlx::query("UPDATE invites SET expires_at = 0")
        .execute(&tb.t.db)
        .await
        .unwrap();
    assert_eq!(
        accept(app, &late, expiring.body["token"].as_str().unwrap())
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // Bad settings and junk tokens.
    assert_eq!(
        invite(
            app,
            &tb.gm,
            tb.campaign,
            json!({ "role": "player", "maxUses": 0 })
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        invite(
            app,
            &tb.gm,
            tb.campaign,
            json!({ "role": "player", "expiresInDays": 31 })
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        accept(app, &late, "not-a-token").await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_campaign_always_keeps_a_gm() {
    let tb = table().await;
    let app = tb.app();
    let gm_id = 1; // First account created.
    let player_id = 2;

    // The only GM can't leave, demote themselves, or be removed.
    for r in [
        req("POST", tb.url("/leave")),
        req("PATCH", tb.url(&format!("/members/{gm_id}"))).json(json!({ "role": "player" })),
        req("DELETE", tb.url(&format!("/members/{gm_id}"))),
    ] {
        let reply = r.cookie(&tb.gm).send(app).await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{}", reply.body);
        assert_eq!(reply.error_code(), "conflict");
    }

    // With a second GM, the first can step down.
    let promoted = req("PATCH", tb.url(&format!("/members/{player_id}")))
        .cookie(&tb.gm)
        .json(json!({ "role": "gm" }))
        .send(app)
        .await;
    assert_eq!(
        promoted.body,
        json!({ "userId": player_id, "displayName": "Vex", "role": "gm" })
    );
    let stepped_down = req("PATCH", tb.url(&format!("/members/{gm_id}")))
        .cookie(&tb.gm)
        .json(json!({ "role": "player" }))
        .send(app)
        .await;
    assert_eq!(stepped_down.status, StatusCode::OK);

    // The new GM removes the old one, who then can't see the campaign at all.
    assert_eq!(
        req("DELETE", tb.url(&format!("/members/{gm_id}")))
            .cookie(&tb.player)
            .send(app)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        req("GET", tb.url("")).cookie(&tb.gm).send(app).await.status,
        StatusCode::NOT_FOUND
    );

    // Changing someone who isn't a member is a 404.
    let missing = req("PATCH", tb.url("/members/999"))
        .cookie(&tb.player)
        .json(json!({ "role": "gm" }))
        .send(app)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn players_can_leave() {
    let tb = table().await;
    assert_eq!(
        req("POST", tb.url("/leave"))
            .cookie(&tb.player)
            .send(tb.app())
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        req("GET", tb.url(""))
            .cookie(&tb.player)
            .send(tb.app())
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn uploading_a_map_and_making_a_scene() {
    let tb = table().await;
    let app = tb.app();
    let asset = upload(&tb, "small-bodega.uvtt", SMALL_MAP).await;
    assert_eq!(asset.status, StatusCode::CREATED, "{}", asset.body);
    assert_eq!(asset.body["kind"], "uvtt");
    assert_eq!(asset.body["name"], "small-bodega.uvtt");
    assert_eq!(
        (asset.body["width"].as_u64(), asset.body["height"].as_u64()),
        (Some(840), Some(700))
    );
    let asset_id = asset.id();

    // The files are on disk, under names the server chose.
    let dir =
        tb.t.data_dir
            .path()
            .join(format!("campaigns/{}/assets/{asset_id}", tb.campaign));
    assert!(dir.join("image.png").exists() && dir.join("map.json").exists());

    let scene = req("POST", tb.url("/scenes"))
        .cookie(&tb.gm)
        .json(json!({ "name": "The Bodega", "assetId": asset_id }))
        .send(app)
        .await;
    assert_eq!(scene.status, StatusCode::CREATED);
    let scene_id = scene.id();

    // Players can load the scene and its picture.
    let detail = req("GET", tb.url(&format!("/scenes/{scene_id}")))
        .cookie(&tb.player)
        .send(app)
        .await;
    assert_eq!(detail.body["pixelsPerGrid"], 70.0);
    assert_eq!(detail.body["tiles"].as_array().unwrap().len(), 1);
    assert_eq!(
        detail.body["map"]["line_of_sight"]
            .as_array()
            .unwrap()
            .len(),
        28
    );
    assert_eq!(
        detail.body["map"]["image"], "",
        "the picture comes as tiles, not inside the JSON"
    );

    let tile = req("GET", tb.url(&format!("/assets/{asset_id}/tiles/0")))
        .cookie(&tb.player)
        .send(app)
        .await;
    assert_eq!(tile.status, StatusCode::OK);
    assert_eq!(tile.content_type.as_deref(), Some("image/png"));
    assert_eq!(image::load_from_memory(&tile.bytes).unwrap().width(), 840);
    assert_eq!(
        req("GET", tb.url(&format!("/assets/{asset_id}/tiles/1")))
            .cookie(&tb.player)
            .send(app)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // The GM adjusts the grid; nonsense values are refused; players can't.
    let adjusted = req("PATCH", tb.url(&format!("/scenes/{scene_id}")))
        .cookie(&tb.gm)
        .json(json!({ "pixelsPerGrid": 72.5, "gridOffsetX": 3 }))
        .send(app)
        .await;
    assert_eq!(adjusted.body["pixelsPerGrid"], 72.5);
    assert_eq!(adjusted.body["gridOffsetX"], 3.0);
    assert_eq!(adjusted.body["name"], "The Bodega");
    let silly = req("PATCH", tb.url(&format!("/scenes/{scene_id}")))
        .cookie(&tb.gm)
        .json(json!({ "pixelsPerGrid": 2 }))
        .send(app)
        .await;
    assert_eq!(silly.status, StatusCode::BAD_REQUEST);
    let player_edit = req("PATCH", tb.url(&format!("/scenes/{scene_id}")))
        .cookie(&tb.player)
        .json(json!({ "name": "x" }))
        .send(app)
        .await;
    assert_eq!(player_edit.status, StatusCode::FORBIDDEN);

    // The asset can't be deleted while the scene uses it (AST-008).
    let in_use = req("DELETE", tb.url(&format!("/assets/{asset_id}")))
        .cookie(&tb.gm)
        .send(app)
        .await;
    assert_eq!(in_use.status, StatusCode::CONFLICT);
    assert!(
        in_use.body["message"]
            .as_str()
            .unwrap()
            .contains("The Bodega")
    );
    let listed = req("GET", tb.url("/assets")).cookie(&tb.gm).send(app).await;
    assert_eq!(listed.body[0]["usedBy"], json!(["The Bodega"]));

    assert_eq!(
        req("DELETE", tb.url(&format!("/scenes/{scene_id}")))
            .cookie(&tb.gm)
            .send(app)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        req("DELETE", tb.url(&format!("/assets/{asset_id}")))
            .cookie(&tb.gm)
            .send(app)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(!dir.exists(), "files are removed with the asset");
}

#[tokio::test]
async fn large_maps_are_tiled() {
    let tb = table().await;
    let asset = upload(&tb, "large.uvtt", LARGE_MAP).await;
    assert_eq!(asset.status, StatusCode::CREATED, "{}", asset.body);
    let asset_id = asset.id();
    let scene = req("POST", tb.url("/scenes"))
        .cookie(&tb.gm)
        .json(json!({ "name": "Plaza", "assetId": asset_id }))
        .send(tb.app())
        .await;
    let detail = req("GET", tb.url(&format!("/scenes/{}", scene.id())))
        .cookie(&tb.player)
        .send(tb.app())
        .await;
    assert_eq!(detail.body["tiles"].as_array().unwrap().len(), 9);

    // The last tile is the bottom-right corner: 1504 × 104 pixels.
    let last = req("GET", tb.url(&format!("/assets/{asset_id}/tiles/8")))
        .cookie(&tb.player)
        .send(tb.app())
        .await;
    let img = image::load_from_memory(&last.bytes).unwrap();
    assert_eq!((img.width(), img.height()), (1504, 104));
    assert!(last.content_type.as_deref() == Some("image/png"));
}

#[tokio::test]
async fn uploads_are_checked() {
    let tb = table().await;
    let app = tb.app();

    // A PNG image is accepted as an image asset.
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(64, 32)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let image_asset = upload(&tb, "token.png", &png.into_inner()).await;
    assert_eq!(image_asset.status, StatusCode::CREATED);
    assert_eq!(image_asset.body["kind"], "image");

    // Neither a map nor a supported image.
    for (name, bytes) in [
        ("notes.txt", b"just some text".to_vec()),
        ("anim.gif", b"GIF89a\x01\x00\x01\x00".to_vec()),
        (
            "fake.png",
            b"\x89PNG\r\n\x1a\nthis is not really a png".to_vec(),
        ),
        ("broken.uvtt", br#"{"format": 0.3}"#.to_vec()),
    ] {
        let reply = upload(&tb, name, &bytes).await;
        assert_eq!(
            reply.status,
            StatusCode::BAD_REQUEST,
            "{name}: {}",
            reply.body
        );
        assert_eq!(reply.error_code(), "invalid_input");
    }

    // Over 50 MB is refused before it's all read.
    let huge = upload(&tb, "huge.uvtt", &vec![b' '; 51 * 1024 * 1024]).await;
    assert_eq!(huge.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(huge.error_code(), "too_large");

    // The campaign's 1 GB quota.
    sqlx::query("UPDATE assets SET size_bytes = 1073741824")
        .execute(&tb.t.db)
        .await
        .unwrap();
    let over_quota = upload(&tb, "small.uvtt", SMALL_MAP).await;
    assert_eq!(over_quota.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(
        over_quota.body["message"]
            .as_str()
            .unwrap()
            .contains("out of upload space")
    );

    // Scenes need a map, not an image.
    let not_a_map = req("POST", tb.url("/scenes"))
        .cookie(&tb.gm)
        .json(json!({ "name": "x", "assetId": image_asset.id() }))
        .send(app)
        .await;
    assert_eq!(not_a_map.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deleting_a_campaign_removes_everything() {
    let tb = table().await;
    let app = tb.app();
    let asset = upload(&tb, "small.uvtt", SMALL_MAP).await;
    req("POST", tb.url("/scenes"))
        .cookie(&tb.gm)
        .json(json!({ "name": "Bodega", "assetId": asset.id() }))
        .send(app)
        .await;
    let dir =
        tb.t.data_dir
            .path()
            .join(format!("campaigns/{}", tb.campaign));
    assert!(dir.exists());

    let wrong = req("DELETE", tb.url(""))
        .cookie(&tb.gm)
        .json(json!({ "confirmName": "neon nights" }))
        .send(app)
        .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);

    let renamed = req("PATCH", tb.url(""))
        .cookie(&tb.gm)
        .json(json!({ "name": "  Chrome & Rain " }))
        .send(app)
        .await;
    assert_eq!(renamed.body["name"], "Chrome & Rain");

    let deleted = req("DELETE", tb.url(""))
        .cookie(&tb.gm)
        .json(json!({ "confirmName": "Chrome & Rain" }))
        .send(app)
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.body);
    assert!(!dir.exists());
    for table_name in ["campaigns", "memberships", "invites", "assets", "scenes"] {
        let sql = format!("SELECT COUNT(*) FROM {table_name}");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&tb.t.db)
            .await
            .unwrap();
        assert_eq!(count, 0, "{table_name} should be empty");
    }
    assert_eq!(
        req("GET", "/api/campaigns")
            .cookie(&tb.player)
            .send(app)
            .await
            .body,
        json!([])
    );
}
