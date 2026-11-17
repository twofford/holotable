//! Integration tests for live scenes (M4), over real WebSocket connections.
//!
//! Each test starts the app on a random local port and connects real clients.
//! HTTP calls (sign-up, uploads) still go through `oneshot` on the same app, so
//! both share one state: one database, one hub.

mod common;

use std::{net::SocketAddr, time::Duration};

use axum::http::{StatusCode, header};
use common::{TestApp, req, sign_up_as, test_app};
use futures_util::{SinkExt, StreamExt};
use holotable_core::{
    campaign::Role,
    protocol::{ClientMsg, Resume, ServerMsg},
    scene::{RejectCode, SceneEvent, SceneOp, Token, TokenSettings},
};
use serde_json::json;
use tokio::{net::TcpStream, time::timeout};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

const SMALL_MAP: &[u8] = include_bytes!("../../../fixtures/small-bodega.uvtt");

/// A served app with a campaign, a scene of the small map, a GM, a player
/// (Vex) and an outsider.
struct Table {
    t: TestApp,
    addr: SocketAddr,
    gm: String,
    vex: String,
    outsider: String,
    campaign: i64,
    scene: i64,
}

async fn table() -> Table {
    let t = test_app().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let service = t
        .app
        .clone()
        .into_make_service_with_connect_info::<SocketAddr>();
    // The server runs as its own task for the rest of the test.
    tokio::spawn(async move { axum::serve(listener, service).await.unwrap() });

    let gm = sign_up_as(&t.app, "gm@example.com", "Ghost").await;
    let vex = sign_up_as(&t.app, "vex@example.com", "Vex").await;
    let outsider = sign_up_as(&t.app, "outsider@example.com", "Nobody").await;
    let campaign = req("POST", "/api/campaigns")
        .cookie(&gm)
        .json(json!({ "name": "Neon Nights" }))
        .send(&t.app)
        .await
        .id();
    let invite = req("POST", format!("/api/campaigns/{campaign}/invites"))
        .cookie(&gm)
        .json(json!({ "role": "player" }))
        .send(&t.app)
        .await;
    let token = invite.body["token"].as_str().unwrap();
    req("POST", format!("/api/invites/{token}/accept"))
        .cookie(&vex)
        .send(&t.app)
        .await;
    let asset = req(
        "POST",
        format!("/api/campaigns/{campaign}/assets?name=bodega.uvtt"),
    )
    .cookie(&gm)
    .bytes(SMALL_MAP.to_vec())
    .send(&t.app)
    .await
    .id();
    let scene = req("POST", format!("/api/campaigns/{campaign}/scenes"))
        .cookie(&gm)
        .json(json!({ "name": "Bodega", "assetId": asset }))
        .send(&t.app)
        .await
        .id();
    Table {
        t,
        addr,
        gm,
        vex,
        outsider,
        campaign,
        scene,
    }
}

/// User ids in the order the accounts were created.
const GM_ID: i64 = 1;
const VEX_ID: i64 = 2;

/// A WebSocket client that speaks the protocol.
struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    next_req: u32,
}

impl Client {
    async fn connect(addr: SocketAddr, cookie: &str) -> Client {
        let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
        request
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        request
            .headers_mut()
            .insert(header::ORIGIN, format!("http://{addr}").parse().unwrap());
        let (ws, _) = connect_async(request).await.expect("WebSocket connects");
        let mut client = Client { ws, next_req: 1 };
        assert!(matches!(
            client.next().await,
            Some(ServerMsg::Welcome { .. })
        ));
        client
    }

    async fn send(&mut self, msg: &ClientMsg) {
        self.ws
            .send(Message::Text(serde_json::to_string(msg).unwrap().into()))
            .await
            .unwrap();
    }

    /// Sends an op and returns its request id.
    async fn op(&mut self, op: SceneOp) -> u32 {
        let req_id = self.next_req;
        self.next_req += 1;
        self.send(&ClientMsg::Op { req_id, op }).await;
        req_id
    }

    async fn join(&mut self, scene_id: i64, resume: Option<Resume>) {
        self.send(&ClientMsg::JoinScene { scene_id, resume }).await;
    }

    /// The next protocol message, skipping pings. `None` if the connection
    /// closed or nothing arrived within two seconds.
    async fn next(&mut self) -> Option<ServerMsg> {
        loop {
            let frame = timeout(Duration::from_secs(2), self.ws.next())
                .await
                .ok()??
                .ok()?;
            match frame {
                Message::Text(text) => match serde_json::from_str(text.as_str()).unwrap() {
                    ServerMsg::Ping { .. } => continue,
                    msg => return Some(msg),
                },
                Message::Close(_) => return None,
                _ => continue,
            }
        }
    }

    /// Asserts that nothing arrives for a short while.
    async fn expect_silence(&mut self) {
        if let Ok(Some(Ok(Message::Text(text)))) =
            timeout(Duration::from_millis(300), self.ws.next()).await
        {
            let msg: ServerMsg = serde_json::from_str(text.as_str()).unwrap();
            assert!(
                matches!(msg, ServerMsg::Ping { .. }),
                "expected nothing, got {msg:?}"
            );
        }
    }

    async fn snapshot(&mut self) -> (String, u64, Role, Vec<Token>) {
        match self.next().await {
            Some(ServerMsg::SceneSnapshot {
                epoch,
                seq,
                role,
                tokens,
                ..
            }) => (epoch, seq, role, tokens),
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    async fn event(&mut self) -> (u64, SceneEvent) {
        match self.next().await {
            Some(ServerMsg::Event { seq, event }) => (seq, event),
            other => panic!("expected an event, got {other:?}"),
        }
    }

    /// Waits for the Ack of `req_id`, returning the new token id if any.
    async fn ack(&mut self, req_id: u32) -> Option<i64> {
        loop {
            match self.next().await {
                Some(ServerMsg::Ack {
                    req_id: r,
                    token_id,
                    ..
                }) if r == req_id => return token_id,
                Some(ServerMsg::Event { .. }) => continue, // Our own change comes back as an event first.
                other => panic!("expected ack {req_id}, got {other:?}"),
            }
        }
    }

    async fn reject(&mut self, req_id: u32) -> RejectCode {
        match self.next().await {
            Some(ServerMsg::Reject {
                req_id: r, code, ..
            }) if r == req_id => code,
            other => panic!("expected reject {req_id}, got {other:?}"),
        }
    }
}

fn settings(name: &str, owner: Option<i64>, hidden: bool) -> TokenSettings {
    TokenSettings {
        name: name.into(),
        asset_id: None,
        size: 1,
        owner_id: owner,
        hidden,
    }
}

/// GM and Vex in the scene, with Vex's token at (2, 2) in the shop.
async fn table_with_token() -> (Table, Client, Client, i64) {
    let tb = table().await;
    let mut gm = Client::connect(tb.addr, &tb.gm).await;
    let mut vex = Client::connect(tb.addr, &tb.vex).await;
    gm.join(tb.scene, None).await;
    assert_eq!(gm.snapshot().await.2, Role::Gm);
    vex.join(tb.scene, None).await;
    assert_eq!(vex.snapshot().await.2, Role::Player);

    let req = gm
        .op(SceneOp::CreateToken {
            settings: settings("Vex", Some(VEX_ID), false),
            x: 2.0,
            y: 2.0,
        })
        .await;
    let token_id = gm.ack(req).await.expect("new token id");
    match vex.event().await.1 {
        SceneEvent::TokenCreated { token } => assert_eq!(token.id, token_id),
        other => panic!("unexpected {other:?}"),
    }
    (tb, gm, vex, token_id)
}

#[tokio::test]
async fn moves_reach_everyone_quickly_and_are_saved() {
    let (tb, mut gm, mut vex, token_id) = table_with_token().await;

    let started = std::time::Instant::now();
    let req = vex
        .op(SceneOp::MoveToken {
            token_id,
            x: 4.0,
            y: 3.0,
        })
        .await;
    let (seq, event) = gm.event().await;
    let elapsed = started.elapsed();
    assert_eq!(
        event,
        SceneEvent::TokenMoved {
            token_id,
            x: 4.0,
            y: 3.0
        }
    );
    assert!(
        elapsed < Duration::from_millis(200),
        "took {elapsed:?} (SYN-005)"
    );
    vex.ack(req).await;

    // Saved before the Ack (SYN-008).
    let (x, y): (f64, f64) = sqlx::query_as("SELECT x, y FROM tokens WHERE id = ?")
        .bind(token_id)
        .fetch_one(&tb.t.db)
        .await
        .unwrap();
    assert_eq!((x, y), (4.0, 3.0));
    assert!(seq >= 2);

    // A newcomer's snapshot has the token where it is now.
    let mut late = Client::connect(tb.addr, &tb.vex).await;
    late.join(tb.scene, None).await;
    let (_, _, _, tokens) = late.snapshot().await;
    assert_eq!((tokens[0].x, tokens[0].y), (4.0, 3.0));
}

#[tokio::test]
async fn permissions_are_checked_on_every_message() {
    let (tb, mut gm, mut vex, token_id) = table_with_token().await;

    // Players can't create, update or delete tokens.
    let r = vex
        .op(SceneOp::CreateToken {
            settings: settings("Mine", None, false),
            x: 1.0,
            y: 1.0,
        })
        .await;
    assert_eq!(vex.reject(r).await, RejectCode::Forbidden);
    let r = vex.op(SceneOp::DeleteToken { token_id }).await;
    assert_eq!(vex.reject(r).await, RejectCode::Forbidden);

    // Players can't move tokens they don't own.
    let r = gm
        .op(SceneOp::CreateToken {
            settings: settings("Thug", None, false),
            x: 5.0,
            y: 5.0,
        })
        .await;
    let thug = gm.ack(r).await.unwrap();
    vex.event().await;
    let r = vex
        .op(SceneOp::MoveToken {
            token_id: thug,
            x: 4.0,
            y: 5.0,
        })
        .await;
    assert_eq!(vex.reject(r).await, RejectCode::Forbidden);

    // From the shop to the back room crosses a wall or the closed door
    // (TOK-004); the GM may do it anyway.
    let r = vex
        .op(SceneOp::MoveToken {
            token_id,
            x: 9.0,
            y: 2.0,
        })
        .await;
    assert_eq!(vex.reject(r).await, RejectCode::Blocked);
    let r = gm
        .op(SceneOp::MoveToken {
            token_id,
            x: 9.0,
            y: 2.0,
        })
        .await;
    gm.ack(r).await;

    // Owners must be members; images must be the campaign's images.
    let r = gm
        .op(SceneOp::CreateToken {
            settings: settings("Ghost owner", Some(3), false),
            x: 1.0,
            y: 1.0,
        })
        .await;
    assert_eq!(gm.reject(r).await, RejectCode::Invalid);
    let map_asset = TokenSettings {
        asset_id: Some(1),
        ..settings("Map as token", None, false)
    };
    let r = gm
        .op(SceneOp::CreateToken {
            settings: map_asset,
            x: 1.0,
            y: 1.0,
        })
        .await;
    assert_eq!(gm.reject(r).await, RejectCode::Invalid);

    // Outsiders can't join; the reply looks the same as for a missing scene.
    let mut outsider = Client::connect(tb.addr, &tb.outsider).await;
    outsider.join(tb.scene, None).await;
    assert!(matches!(
        outsider.next().await,
        Some(ServerMsg::Kicked { rejoin: false, .. })
    ));
    let r = outsider.op(SceneOp::DeleteToken { token_id }).await;
    assert_eq!(outsider.reject(r).await, RejectCode::NotFound);

    // Garbage gets an error, not a crash.
    outsider
        .ws
        .send(Message::Text("{\"type\":\"nope\"}".into()))
        .await
        .unwrap();
    assert!(matches!(
        outsider.next().await,
        Some(ServerMsg::Error { .. })
    ));
}

#[tokio::test]
async fn players_never_receive_hidden_tokens() {
    let (tb, mut gm, mut vex, _) = table_with_token().await;
    let r = gm
        .op(SceneOp::CreateToken {
            settings: settings("Ambusher", None, true),
            x: 6.0,
            y: 4.0,
        })
        .await;
    let ambusher = gm.ack(r).await.unwrap();
    let r = gm
        .op(SceneOp::MoveToken {
            token_id: ambusher,
            x: 5.0,
            y: 4.0,
        })
        .await;
    gm.ack(r).await;
    vex.expect_silence().await;

    // Not in a fresh snapshot either.
    let mut fresh = Client::connect(tb.addr, &tb.vex).await;
    fresh.join(tb.scene, None).await;
    assert_eq!(fresh.snapshot().await.3.len(), 1);

    // Revealed: players see it appear where it is now.
    let r = gm
        .op(SceneOp::UpdateToken {
            token_id: ambusher,
            settings: settings("Ambusher", None, false),
        })
        .await;
    gm.ack(r).await;
    match vex.event().await.1 {
        SceneEvent::TokenCreated { token } => assert_eq!((token.id, token.x), (ambusher, 5.0)),
        other => panic!("unexpected {other:?}"),
    }
    // Hidden again: it disappears for players.
    let r = gm
        .op(SceneOp::UpdateToken {
            token_id: ambusher,
            settings: settings("Ambusher", None, true),
        })
        .await;
    gm.ack(r).await;
    assert_eq!(
        vex.event().await.1,
        SceneEvent::TokenRemoved { token_id: ambusher }
    );
}

#[tokio::test]
async fn reconnecting_clients_catch_up() {
    let (tb, mut gm, vex, token_id) = table_with_token().await;
    let mut vex2 = Client::connect(tb.addr, &tb.vex).await;
    vex2.join(tb.scene, None).await;
    let (epoch, seq, _, _) = vex2.snapshot().await;
    drop(vex2);
    drop(vex);

    let r = gm
        .op(SceneOp::MoveToken {
            token_id,
            x: 3.0,
            y: 3.0,
        })
        .await;
    gm.ack(r).await;

    // Same epoch and a recent seq: only the missed event comes back.
    let mut back = Client::connect(tb.addr, &tb.vex).await;
    back.join(
        tb.scene,
        Some(Resume {
            epoch: epoch.clone(),
            seq,
        }),
    )
    .await;
    assert_eq!(
        back.event().await.1,
        SceneEvent::TokenMoved {
            token_id,
            x: 3.0,
            y: 3.0
        }
    );

    // An unknown epoch (say, after a server restart) gets a full snapshot.
    let mut other = Client::connect(tb.addr, &tb.vex).await;
    other
        .join(
            tb.scene,
            Some(Resume {
                epoch: "stale".into(),
                seq,
            }),
        )
        .await;
    let (_, _, _, tokens) = other.snapshot().await;
    assert_eq!(tokens[0].x, 3.0);
}

#[tokio::test]
async fn dragging_shows_to_others_but_only_for_your_own_tokens() {
    let (_tb, mut gm, mut vex, token_id) = table_with_token().await;
    vex.send(&ClientMsg::Drag {
        token_id,
        x: 2.5,
        y: 2.2,
    })
    .await;
    match gm.next().await {
        Some(ServerMsg::TokenDragging {
            token_id: t,
            user_id,
            ..
        }) => assert_eq!((t, user_id), (token_id, VEX_ID)),
        other => panic!("unexpected {other:?}"),
    }
    let r = gm
        .op(SceneOp::CreateToken {
            settings: settings("Thug", None, false),
            x: 5.0,
            y: 5.0,
        })
        .await;
    let thug = gm.ack(r).await.unwrap();
    vex.event().await;
    vex.send(&ClientMsg::Drag {
        token_id: thug,
        x: 4.0,
        y: 4.0,
    })
    .await;
    gm.expect_silence().await;
}

#[tokio::test]
async fn removed_members_are_kicked_at_once() {
    let (tb, _gm, mut vex, token_id) = table_with_token().await;
    let removed = req(
        "DELETE",
        format!("/api/campaigns/{}/members/{VEX_ID}", tb.campaign),
    )
    .cookie(&tb.gm)
    .send(&tb.t.app)
    .await;
    assert_eq!(removed.status, StatusCode::NO_CONTENT);
    assert!(matches!(
        vex.next().await,
        Some(ServerMsg::Kicked { rejoin: false, .. })
    ));
    let r = vex
        .op(SceneOp::MoveToken {
            token_id,
            x: 3.0,
            y: 2.0,
        })
        .await;
    assert_eq!(vex.reject(r).await, RejectCode::NotFound);
    vex.join(tb.scene, None).await;
    assert!(matches!(
        vex.next().await,
        Some(ServerMsg::Kicked { rejoin: false, .. })
    ));
}

#[tokio::test]
async fn role_changes_send_members_out_to_rejoin() {
    let (tb, _gm, mut vex, _) = table_with_token().await;
    req(
        "PATCH",
        format!("/api/campaigns/{}/members/{VEX_ID}", tb.campaign),
    )
    .cookie(&tb.gm)
    .json(json!({ "role": "gm" }))
    .send(&tb.t.app)
    .await;
    assert!(matches!(
        vex.next().await,
        Some(ServerMsg::Kicked { rejoin: true, .. })
    ));
    vex.join(tb.scene, None).await;
    assert_eq!(vex.snapshot().await.2, Role::Gm);
}

#[tokio::test]
async fn signing_out_closes_the_session_sockets() {
    let (tb, mut gm, mut vex, _) = table_with_token().await;
    req("POST", "/api/auth/signout")
        .cookie(&tb.vex)
        .send(&tb.t.app)
        .await;
    assert!(vex.next().await.is_none(), "socket should close");
    // Other users stay connected.
    gm.send(&ClientMsg::Ping { nonce: 9 }).await;
    assert!(matches!(
        gm.next().await,
        Some(ServerMsg::Pong { nonce: 9 })
    ));
}

#[tokio::test]
async fn scene_changes_reach_the_room() {
    let (tb, mut gm, mut vex, _) = table_with_token().await;
    let url = format!("/api/campaigns/{}", tb.campaign);

    req("PATCH", format!("{url}/scenes/{}", tb.scene))
        .cookie(&tb.gm)
        .json(json!({ "pixelsPerGrid": 35 }))
        .send(&tb.t.app)
        .await;
    assert!(matches!(
        vex.next().await,
        Some(ServerMsg::SceneChanged { .. })
    ));
    assert!(matches!(
        gm.next().await,
        Some(ServerMsg::SceneChanged { .. })
    ));

    let put = req("PUT", format!("{url}/active-scene"))
        .cookie(&tb.gm)
        .json(json!({ "sceneId": tb.scene }))
        .send(&tb.t.app)
        .await;
    assert_eq!(put.status, StatusCode::NO_CONTENT);
    match vex.next().await {
        Some(ServerMsg::ActiveScene { scene_id, .. }) => assert_eq!(scene_id, Some(tb.scene)),
        other => panic!("unexpected {other:?}"),
    }
    let detail = req("GET", url.clone())
        .cookie(&tb.vex)
        .send(&tb.t.app)
        .await;
    assert_eq!(detail.body["activeSceneId"], tb.scene);
    gm.next().await; // The GM is told too.

    req("DELETE", format!("{url}/scenes/{}", tb.scene))
        .cookie(&tb.gm)
        .send(&tb.t.app)
        .await;
    assert!(matches!(
        vex.next().await,
        Some(ServerMsg::Kicked { rejoin: false, .. })
    ));
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tokens")
        .fetch_one(&tb.t.db)
        .await
        .unwrap();
    assert_eq!(count, 0, "tokens go with their scene");
    let detail = req("GET", url).cookie(&tb.vex).send(&tb.t.app).await;
    assert!(detail.body["activeSceneId"].is_null());
}

#[tokio::test]
async fn flooding_clients_are_disconnected() {
    let (_tb, _gm, mut vex, token_id) = table_with_token().await;
    for second in 0..3 {
        for _ in 0..60 {
            vex.send(&ClientMsg::Drag {
                token_id,
                x: 2.0,
                y: 2.0,
            })
            .await;
        }
        if second < 2 {
            tokio::time::sleep(Duration::from_millis(1050)).await;
        }
    }
    // Drain until the server closes the connection.
    let closed = timeout(Duration::from_secs(3), async {
        while vex.next().await.is_some() {}
    })
    .await;
    assert!(closed.is_ok(), "connection should be closed (SYN-009)");
}

#[tokio::test]
async fn images_used_by_tokens_cannot_be_deleted() {
    let (tb, mut gm, _vex, token_id) = table_with_token().await;
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(64, 64)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let image = req(
        "POST",
        format!("/api/campaigns/{}/assets?name=vex.png", tb.campaign),
    )
    .cookie(&tb.gm)
    .bytes(png.into_inner())
    .send(&tb.t.app)
    .await
    .id();
    let with_image = TokenSettings {
        asset_id: Some(image),
        ..settings("Vex", Some(VEX_ID), false)
    };
    let r = gm
        .op(SceneOp::UpdateToken {
            token_id,
            settings: with_image,
        })
        .await;
    gm.ack(r).await;
    let refused = req(
        "DELETE",
        format!("/api/campaigns/{}/assets/{image}", tb.campaign),
    )
    .cookie(&tb.gm)
    .send(&tb.t.app)
    .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert!(refused.body["message"].as_str().unwrap().contains("Bodega"));
    let _ = GM_ID;
}
