//! The WebSocket endpoint.
//!
//! In M0 each connection is on its own: the server greets the client, answers its
//! pings, and pings it every few seconds. M4 replaces this with scene rooms.

use std::time::Duration;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    response::Response,
};
use holotable_core::protocol::{ClientMsg, ServerMsg};
use tokio::time::{Instant, MissedTickBehavior, interval};

/// How often the server pings each client.
const PING_EVERY: Duration = Duration::from_secs(5);
/// Messages larger than this are refused. The M0 protocol needs far less.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// `GET /ws`: upgrades the HTTP request to a WebSocket.
///
/// The handler returns at once with the 101 response; `on_upgrade` runs
/// `handle_socket` later, on its own task, once the upgrade completes.
pub async fn upgrade(ws: WebSocketUpgrade) -> Response {
    // TODO(M2/M4): check the session cookie and the Origin header before upgrading.
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(handle_socket)
}

/// Runs one connection until the client leaves.
///
/// `socket` is moved into this function, so this task owns it: nothing else can
/// read or write it, and it is closed when the function returns and `socket` is dropped.
async fn handle_socket(mut socket: WebSocket) {
    tracing::info!("client connected");
    let welcome = ServerMsg::Welcome {
        server_version: env!("CARGO_PKG_VERSION").into(),
    };
    if send(&mut socket, &welcome).await.is_err() {
        return;
    }

    let mut ticker = interval(PING_EVERY);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // The first tick fires at once; skip it.

    let mut next_nonce: u32 = 1;
    // The ping we are waiting on, and when we sent it.
    let mut awaiting: Option<(u32, Instant)> = None;

    loop {
        // `select!` waits on both futures and runs the branch of whichever finishes
        // first. The other future is dropped, which is safe for both of these.
        tokio::select! {
            _ = ticker.tick() => {
                let nonce = next_nonce;
                next_nonce = next_nonce.wrapping_add(1);
                awaiting = Some((nonce, Instant::now()));
                if send(&mut socket, &ServerMsg::Ping { nonce }).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { break };
                match message {
                    Message::Text(text) => {
                        let reply = handle_text(text.as_str(), &mut awaiting);
                        if let Some(reply) = reply
                            && send(&mut socket, &reply).await.is_err()
                        {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    // Binary frames are not part of the protocol. Ping/Pong frames
                    // are WebSocket-level keepalives, answered by the library.
                    _ => {}
                }
            }
        }
    }
    tracing::info!("client disconnected");
}

/// Handles one text message and returns the reply to send, if any.
///
/// This is a plain function with no I/O, so it is easy to test.
fn handle_text(text: &str, awaiting: &mut Option<(u32, Instant)>) -> Option<ServerMsg> {
    match serde_json::from_str::<ClientMsg>(text) {
        Ok(ClientMsg::Ping { nonce }) => Some(ServerMsg::Pong { nonce }),
        Ok(ClientMsg::Pong { nonce }) => {
            // `take()` moves the value out and leaves `None` behind.
            match awaiting.take() {
                Some((expected, sent_at)) if expected == nonce => {
                    tracing::info!(
                        nonce,
                        rtt_ms = sent_at.elapsed().as_millis() as u64,
                        "client answered ping"
                    );
                }
                other => {
                    tracing::debug!(nonce, "pong for a ping we are not waiting on");
                    *awaiting = other;
                }
            }
            None
        }
        Err(err) => Some(ServerMsg::Error {
            message: format!("bad message: {err}"),
        }),
    }
}

/// Serializes `msg` to JSON and sends it as a text frame.
async fn send(socket: &mut WebSocket, msg: &ServerMsg) -> Result<(), axum::Error> {
    let json = serde_json::to_string(msg).expect("ServerMsg always serializes");
    socket.send(Message::Text(json.into())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_gets_pong_with_same_nonce() {
        let mut awaiting = None;
        let reply = handle_text(r#"{"type":"ping","nonce":42}"#, &mut awaiting);
        assert_eq!(reply, Some(ServerMsg::Pong { nonce: 42 }));
    }

    #[test]
    fn matching_pong_clears_awaiting() {
        let mut awaiting = Some((3, Instant::now()));
        assert_eq!(
            handle_text(r#"{"type":"pong","nonce":3}"#, &mut awaiting),
            None
        );
        assert!(awaiting.is_none());
    }

    #[test]
    fn stale_pong_keeps_awaiting() {
        let mut awaiting = Some((3, Instant::now()));
        handle_text(r#"{"type":"pong","nonce":2}"#, &mut awaiting);
        assert!(matches!(awaiting, Some((3, _))));
    }

    #[test]
    fn garbage_gets_error() {
        let mut awaiting = None;
        assert!(matches!(
            handle_text("nope", &mut awaiting),
            Some(ServerMsg::Error { .. })
        ));
    }
}
