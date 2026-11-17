//! WebSocket protocol shared by the server and the client.
//!
//! Rust is the source of truth. `cargo test` regenerates the TypeScript types in
//! `client/src/protocol/generated/` through `ts-rs`, so a change here that breaks
//! the client is caught by the TypeScript compiler.
//!
//! Each enum is "internally tagged": `#[serde(tag = "type")]` puts the variant name
//! in a `type` field, so `ClientMsg::Ping { nonce: 7 }` is sent as
//! `{"type":"ping","nonce":7}`. Field names become camelCase (`server_version` →
//! `serverVersion`), as is usual in JavaScript. In TypeScript this becomes a discriminated union,
//! and `switch (msg.type)` narrows the type in each branch.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    campaign::Role,
    scene::{RejectCode, SceneEvent, SceneOp, Token},
};

/// Where `cargo test` writes the generated TypeScript. ts-rs resolves this
/// against `crates/core/bindings/`, so it climbs three levels to the repo root.
const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

/// Where a client left off in a scene, so it can catch up after a reconnect
/// instead of reloading everything (spec SYN-007).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct Resume {
    /// Identifies one run of the scene's room. Sequence numbers restart when
    /// the server restarts, so a resume only works within the same epoch.
    pub epoch: String,
    /// The last event sequence number the client saw.
    #[ts(type = "number")]
    pub seq: u64,
}

/// Messages a client sends to the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = EXPORT_DIR)]
pub enum ClientMsg {
    /// The client checks that the server is alive. The server answers with `Pong`.
    Ping { nonce: u32 },
    /// The client's answer to a server `Ping`.
    Pong { nonce: u32 },
    /// Enter a scene's room (and leave any other). The server answers with a
    /// `SceneSnapshot`, or with the missed events when `resume` still works.
    JoinScene {
        #[ts(type = "number")]
        scene_id: i64,
        #[serde(default)]
        #[ts(optional)]
        resume: Option<Resume>,
    },
    /// Leave the current scene's room.
    LeaveScene,
    /// A change to the scene. `req_id` is chosen by the client and comes back
    /// in the `Ack` or `Reject`, so the client knows which change it was.
    Op { req_id: u32, op: SceneOp },
    /// A token is being dragged: show it moving to the others. Not saved, not
    /// acknowledged. Clients send at most 20 a second (SYN-006).
    Drag {
        #[ts(type = "number")]
        token_id: i64,
        x: f64,
        y: f64,
    },
}

/// Messages the server sends to a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = EXPORT_DIR)]
pub enum ServerMsg {
    /// Sent once, right after the connection opens.
    Welcome {
        server_version: String,
        /// The signed-in user's display name.
        display_name: String,
    },
    /// The server checks that the client is alive. The client answers with `Pong`.
    Ping { nonce: u32 },
    /// The server's answer to a client `Ping`.
    Pong { nonce: u32 },
    /// The server could not use a message from the client.
    Error { message: String },
    /// Everything about the scene the client may see, as of `seq`.
    SceneSnapshot {
        #[ts(type = "number")]
        scene_id: i64,
        epoch: String,
        #[ts(type = "number")]
        seq: u64,
        /// Your role in the scene's campaign.
        role: Role,
        tokens: Vec<Token>,
    },
    /// Something changed. Sequence numbers increase; a client may see gaps,
    /// because events it isn't allowed to see are left out.
    Event {
        #[ts(type = "number")]
        seq: u64,
        event: SceneEvent,
    },
    /// Your request was applied. For a new token, `token_id` is its id.
    Ack {
        req_id: u32,
        #[ts(type = "number")]
        seq: u64,
        #[serde(default)]
        #[ts(type = "number | null")]
        token_id: Option<i64>,
    },
    /// Your request was refused; nothing changed (SYN-003).
    Reject {
        req_id: u32,
        code: RejectCode,
        message: String,
    },
    /// Someone is dragging a token (see `ClientMsg::Drag`).
    TokenDragging {
        #[ts(type = "number")]
        token_id: i64,
        x: f64,
        y: f64,
        #[ts(type = "number")]
        user_id: i64,
    },
    /// You were taken out of the scene: removed from the campaign, the scene
    /// was deleted, or your role changed. `rejoin` says whether joining again
    /// may work (it does after a role change).
    Kicked { reason: String, rejoin: bool },
    /// The scene's settings changed (the grid, say): fetch them again.
    SceneChanged {
        #[ts(type = "number")]
        scene_id: i64,
    },
    /// The GM picked the scene everyone should be on (MAP-010).
    ActiveScene {
        #[ts(type = "number")]
        campaign_id: i64,
        #[ts(type = "number | null")]
        scene_id: Option<i64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_ping_uses_type_tag() {
        let json = serde_json::to_string(&ClientMsg::Ping { nonce: 7 }).unwrap();
        assert_eq!(json, r#"{"type":"ping","nonce":7}"#);
    }

    #[test]
    fn server_welcome_round_trips() {
        let msg = ServerMsg::Welcome {
            server_version: "0.1.0".into(),
            display_name: "Vex".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"welcome","serverVersion":"0.1.0","displayName":"Vex"}"#
        );
        let back: ServerMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn join_scene_resume_is_optional() {
        let msg: ClientMsg = serde_json::from_str(r#"{"type":"joinScene","sceneId":3}"#).unwrap();
        assert_eq!(
            msg,
            ClientMsg::JoinScene {
                scene_id: 3,
                resume: None
            }
        );
    }

    #[test]
    fn ops_nest_inside_client_messages() {
        let msg: ClientMsg = serde_json::from_str(
            r#"{"type":"op","reqId":4,"op":{"type":"deleteToken","tokenId":9}}"#,
        )
        .unwrap();
        assert_eq!(
            msg,
            ClientMsg::Op {
                req_id: 4,
                op: SceneOp::DeleteToken { token_id: 9 }
            }
        );
    }

    #[test]
    fn unknown_type_is_rejected() {
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"hack"}"#).is_err());
    }
}
