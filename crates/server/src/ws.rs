//! The WebSocket endpoint: one connection per browser tab.
//!
//! A socket can be read and written at the same time, so it's split in two:
//!
//! - a **writer task** owns the sending half. Everything to send goes through
//!   its queue (the connection's `Outbox`), from this file, from scene rooms,
//!   and from HTTP handlers (a kick, say). No two parts of the program ever
//!   write to the socket at once, because only the writer can.
//! - the **reader loop** below owns the receiving half: it decodes messages,
//!   checks the rate limit, and passes scene requests on to the hub.

use std::time::Duration;

use axum::{
    extract::{
        State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use holotable_core::{
    api::UserInfo,
    protocol::{ClientMsg, ServerMsg},
    scene::RejectCode,
};
use tokio::{
    sync::mpsc,
    time::{Instant, MissedTickBehavior, interval},
};

use crate::{
    AppState,
    auth::extract::CurrentUser,
    live::{
        ConnId, JoinError, OUTBOX_CAPACITY, Outbox, Outgoing,
        rate::{MessageRate, Verdict},
    },
};

/// How often the server pings each client.
const PING_EVERY: Duration = Duration::from_secs(5);
/// Messages larger than this are refused. The protocol so far needs far less.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;

/// `GET /ws`: upgrades the HTTP request to a WebSocket.
///
/// `CurrentUser` comes first, so a request without a valid session gets a 401
/// before any upgrade happens. The Origin check runs earlier still, in
/// `origin.rs`. The closure is `move`, so it owns `state` and `current` when
/// it runs later, on its own task, once the upgrade completes.
pub async fn upgrade(
    State(state): State<AppState>,
    current: CurrentUser,
    ws: WebSocketUpgrade,
) -> Response {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state, current))
}

async fn handle_socket(socket: WebSocket, state: AppState, current: CurrentUser) {
    let user = current.user;
    // `split` turns one socket into a sending half and a receiving half that
    // can be moved to different tasks.
    let (sink, mut stream) = socket.split();
    let (outbox, queue) = mpsc::channel::<Outgoing>(OUTBOX_CAPACITY);
    let writer = tokio::spawn(write_loop(sink, queue));

    let conn = state
        .hub
        .register(user.id, current.token.hash(), outbox.clone());
    tracing::info!(user_id = user.id, conn, "client connected");
    let _ = outbox
        .send(Outgoing::Msg(ServerMsg::Welcome {
            server_version: env!("CARGO_PKG_VERSION").into(),
            display_name: user.display_name.clone(),
        }))
        .await;

    read_loop(&state, conn, &user, &outbox, &mut stream).await;

    state.hub.unregister(conn);
    // Dropping our last `outbox` lets the writer's queue end. The writer then
    // finishes sending what's queued and stops. (The hub dropped its copy in
    // `unregister`.)
    drop(outbox);
    let _ = writer.await;
    tracing::info!(user_id = user.id, conn, "client disconnected");
}

/// Sends everything that arrives in `queue`, until it's closed or a `Close` comes.
async fn write_loop(
    mut sink: futures_util::stream::SplitSink<WebSocket, Message>,
    mut queue: mpsc::Receiver<Outgoing>,
) {
    while let Some(out) = queue.recv().await {
        match out {
            Outgoing::Msg(msg) => {
                let json = serde_json::to_string(&msg).expect("ServerMsg always serializes");
                if sink.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Outgoing::Close(reason) => {
                // 4000-4999 are close codes for applications to define.
                let frame = CloseFrame {
                    code: 4000,
                    reason: reason.into(),
                };
                let _ = sink.send(Message::Close(Some(frame))).await;
                break;
            }
        }
    }
}

async fn read_loop(
    state: &AppState,
    conn: ConnId,
    user: &UserInfo,
    outbox: &Outbox,
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
) {
    let mut ticker = interval(PING_EVERY);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker.tick().await; // The first tick fires at once; skip it.
    let mut next_nonce: u32 = 1;
    // The ping we are waiting on, and when we sent it.
    let mut awaiting: Option<(u32, Instant)> = None;
    let mut rate = MessageRate::new(std::time::Instant::now());

    loop {
        // `select!` waits on both futures and runs the branch of whichever
        // finishes first. Both are safe to cancel.
        tokio::select! {
            _ = ticker.tick() => {
                let nonce = next_nonce;
                next_nonce = next_nonce.wrapping_add(1);
                awaiting = Some((nonce, Instant::now()));
                if outbox.send(Outgoing::Msg(ServerMsg::Ping { nonce })).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => {
                let Some(Ok(message)) = incoming else { break };
                let Message::Text(text) = message else {
                    if matches!(message, Message::Close(_)) { break }
                    continue; // Binary frames aren't part of the protocol.
                };
                match rate.check(std::time::Instant::now()) {
                    Verdict::Allow => {}
                    Verdict::Drop => continue,
                    Verdict::Disconnect => {
                        tracing::warn!(user_id = user.id, conn, "too many messages; disconnecting");
                        let _ = outbox.send(Outgoing::Close("Too many messages.".into())).await;
                        break;
                    }
                }
                handle_text(state, conn, outbox, text.as_str(), &mut awaiting).await;
            }
        }
    }
}

/// Handles one text message from the client.
async fn handle_text(
    state: &AppState,
    conn: ConnId,
    outbox: &Outbox,
    text: &str,
    awaiting: &mut Option<(u32, Instant)>,
) {
    // A full outbox means the client is far behind; dropping a reply is fine
    // (the writer will close it soon), so the result is ignored.
    let reply = |msg| {
        let _ = outbox.try_send(Outgoing::Msg(msg));
    };
    let msg = match serde_json::from_str::<ClientMsg>(text) {
        Ok(msg) => msg,
        Err(err) => {
            reply(ServerMsg::Error {
                message: format!("bad message: {err}"),
            });
            return;
        }
    };
    match msg {
        ClientMsg::Ping { nonce } => {
            reply(ServerMsg::Pong { nonce });
        }
        ClientMsg::Pong { nonce } => {
            // `take()` moves the value out and leaves `None` behind.
            match awaiting.take() {
                Some((expected, sent_at)) if expected == nonce => {
                    tracing::debug!(
                        nonce,
                        rtt_ms = sent_at.elapsed().as_millis() as u64,
                        "client answered ping"
                    );
                }
                other => *awaiting = other,
            }
        }
        ClientMsg::JoinScene { scene_id, resume } => {
            match state.hub.join(conn, scene_id, resume).await {
                Ok(()) => {}
                Err(JoinError::NotFound) => {
                    reply(ServerMsg::Kicked {
                        reason: "That scene doesn't exist, or isn't yours to see.".into(),
                        rejoin: false,
                    });
                }
                Err(JoinError::Internal(e)) => {
                    tracing::error!(error = %e, scene_id, "joining a scene failed");
                    reply(ServerMsg::Error {
                        message: "The server couldn't open that scene.".into(),
                    });
                }
            }
        }
        ClientMsg::LeaveScene => state.hub.leave(conn),
        ClientMsg::Op { req_id, op } => {
            if !state.hub.op(conn, req_id, op).await {
                reply(ServerMsg::Reject {
                    req_id,
                    code: RejectCode::NotFound,
                    message: "You're not in a scene.".into(),
                });
            }
        }
        ClientMsg::Drag { token_id, x, y } => state.hub.drag(conn, token_id, x, y),
    }
}
