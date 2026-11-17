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

/// Where `cargo test` writes the generated TypeScript. ts-rs resolves this
/// against `crates/core/bindings/`, so it climbs three levels to the repo root.
const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

/// Messages a client sends to the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
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
}

/// Messages the server sends to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = EXPORT_DIR)]
pub enum ServerMsg {
    /// Sent once, right after the connection opens.
    Welcome { server_version: String },
    /// The server checks that the client is alive. The client answers with `Pong`.
    Ping { nonce: u32 },
    /// The server's answer to a client `Ping`.
    Pong { nonce: u32 },
    /// The server could not use a message from the client.
    Error { message: String },
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
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"welcome","serverVersion":"0.1.0"}"#);
        let back: ServerMsg = serde_json::from_str(&json).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn unknown_type_is_rejected() {
        assert!(serde_json::from_str::<ClientMsg>(r#"{"type":"hack"}"#).is_err());
    }
}
