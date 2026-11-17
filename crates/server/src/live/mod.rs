//! Live play: who is connected, and the scene rooms they're in (M4).
//!
//! The `Hub` keeps two registries: open connections (one per browser tab) and
//! running scene rooms. It's shared by every connection and every HTTP handler
//! through `AppState`, so it sits behind an `Arc`, and its maps behind
//! `std::sync::Mutex`. The locks are held only for quick map lookups and never
//! across an `.await`; all the slow work (the database, the rooms themselves)
//! happens outside them.

pub mod rate;
pub mod room;

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use holotable_core::{
    campaign::Role,
    protocol::{Resume, ServerMsg},
    scene::SceneOp,
};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use self::room::{Room, RoomCmd};
use crate::{error::AppError, storage::Storage};

/// Identifies one WebSocket connection.
pub type ConnId = u64;

/// What a connection's writer task should do next.
#[derive(Debug)]
pub enum Outgoing {
    /// Send this message.
    Msg(ServerMsg),
    /// Close the connection, with this reason.
    Close(String),
}

/// The sending end of a connection's queue of outgoing messages. Cheap to
/// clone; every clone feeds the same queue.
pub type Outbox = mpsc::Sender<Outgoing>;

/// Each connection may queue this many messages before it counts as stalled.
pub const OUTBOX_CAPACITY: usize = 256;
/// Each room may queue this many commands.
const ROOM_QUEUE: usize = 1024;

struct Conn {
    user_id: i64,
    /// Hash of the session the connection signed in with, so signing out can
    /// close exactly that session's sockets.
    session_hash: Vec<u8>,
    outbox: Outbox,
    /// The room this connection is in: (scene id, campaign id).
    room: Option<(i64, i64)>,
}

#[derive(Clone)]
struct RoomHandle {
    campaign_id: i64,
    commands: mpsc::Sender<RoomCmd>,
}

pub struct Hub {
    db: SqlitePool,
    storage: Arc<Storage>,
    next_conn: AtomicU64,
    conns: Mutex<HashMap<ConnId, Conn>>,
    rooms: Mutex<HashMap<i64, RoomHandle>>,
}

/// Why joining a scene failed.
pub enum JoinError {
    /// No such scene, or the user isn't in its campaign. Deliberately the same.
    NotFound,
    Internal(AppError),
}

impl Hub {
    pub fn new(db: SqlitePool, storage: Arc<Storage>) -> Self {
        Self {
            db,
            storage,
            next_conn: AtomicU64::new(1),
            conns: Mutex::new(HashMap::new()),
            rooms: Mutex::new(HashMap::new()),
        }
    }

    /// `lock()` only fails if another thread panicked while holding the lock.
    /// The maps are still consistent (every update is a single insert or
    /// remove), so carry on with them.
    fn conns(&self) -> std::sync::MutexGuard<'_, HashMap<ConnId, Conn>> {
        self.conns.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn rooms(&self) -> std::sync::MutexGuard<'_, HashMap<i64, RoomHandle>> {
        self.rooms.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a new connection and returns its id.
    pub fn register(&self, user_id: i64, session_hash: Vec<u8>, outbox: Outbox) -> ConnId {
        // An atomic counter hands out unique ids without a lock.
        let id = self.next_conn.fetch_add(1, Ordering::Relaxed);
        self.conns().insert(
            id,
            Conn {
                user_id,
                session_hash,
                outbox,
                room: None,
            },
        );
        id
    }

    /// Forgets a connection that closed, taking it out of its room.
    pub fn unregister(&self, conn: ConnId) {
        let removed = self.conns().remove(&conn);
        if let Some(Conn {
            room: Some((scene_id, _)),
            ..
        }) = removed
        {
            self.tell_room(scene_id, RoomCmd::Leave { conn });
        }
    }

    /// Puts a connection in a scene's room, after checking the user may see it.
    pub async fn join(
        &self,
        conn: ConnId,
        scene_id: i64,
        resume: Option<Resume>,
    ) -> Result<(), JoinError> {
        let Some((user_id, outbox)) = self
            .conns()
            .get(&conn)
            .map(|c| (c.user_id, c.outbox.clone()))
        else {
            return Ok(()); // The connection closed meanwhile.
        };
        let row: Option<(i64, String)> = sqlx::query_as(
            "SELECT m.campaign_id, m.role FROM scenes s
             JOIN memberships m ON m.campaign_id = s.campaign_id AND m.user_id = ?
             WHERE s.id = ?",
        )
        .bind(user_id)
        .bind(scene_id)
        .fetch_optional(&self.db)
        .await
        .map_err(|e| JoinError::Internal(e.into()))?;
        let (campaign_id, role) = row.ok_or(JoinError::NotFound)?;
        let role = Role::from_db(&role).ok_or(JoinError::NotFound)?;

        let room = self.room(scene_id).await?;
        self.leave(conn);
        if let Some(c) = self.conns().get_mut(&conn) {
            c.room = Some((scene_id, campaign_id));
        }
        let _ = room
            .commands
            .send(RoomCmd::Join {
                conn,
                user_id,
                role,
                outbox,
                resume,
            })
            .await;
        Ok(())
    }

    /// Takes a connection out of its room, if it's in one.
    pub fn leave(&self, conn: ConnId) {
        let previous = self.conns().get_mut(&conn).and_then(|c| c.room.take());
        if let Some((scene_id, _)) = previous {
            self.tell_room(scene_id, RoomCmd::Leave { conn });
        }
    }

    /// Passes a scene change to the connection's room. Returns `false` if the
    /// connection isn't in a room.
    pub async fn op(&self, conn: ConnId, req_id: u32, op: SceneOp) -> bool {
        let Some(scene_id) = self.room_of(conn) else {
            return false;
        };
        let commands = self.rooms().get(&scene_id).map(|r| r.commands.clone());
        match commands {
            Some(tx) => tx.send(RoomCmd::Op { conn, req_id, op }).await.is_ok(),
            None => false,
        }
    }

    pub fn drag(&self, conn: ConnId, token_id: i64, x: f64, y: f64) {
        if let Some(scene_id) = self.room_of(conn) {
            self.tell_room(
                scene_id,
                RoomCmd::Drag {
                    conn,
                    token_id,
                    x,
                    y,
                },
            );
        }
    }

    fn room_of(&self, conn: ConnId) -> Option<i64> {
        self.conns()
            .get(&conn)
            .and_then(|c| c.room.map(|(scene_id, _)| scene_id))
    }

    /// The running room for a scene, starting it if needed.
    async fn room(&self, scene_id: i64) -> Result<RoomHandle, JoinError> {
        if let Some(room) = self.rooms().get(&scene_id) {
            return Ok(room.clone());
        }
        // Load outside the lock: it reads the database and a file.
        let loaded = Room::load(self.db.clone(), self.storage.clone(), scene_id)
            .await
            .map_err(|e| match e {
                AppError::NotFound(_) => JoinError::NotFound,
                other => JoinError::Internal(other),
            })?;
        // Two joins may have loaded the room at once; the first to get here
        // wins and the other copy is simply dropped.
        let mut rooms = self.rooms();
        let handle = rooms.entry(scene_id).or_insert_with(|| {
            let (tx, rx) = mpsc::channel(ROOM_QUEUE);
            let handle = RoomHandle {
                campaign_id: loaded.campaign_id(),
                commands: tx,
            };
            tokio::spawn(loaded.run(rx));
            handle
        });
        Ok(handle.clone())
    }

    /// Sends a command to a room without waiting. If its queue is full, the
    /// command is dropped: only joins and changes must never be lost, and
    /// those use the waiting `send` above.
    fn tell_room(&self, scene_id: i64, cmd: RoomCmd) {
        if let Some(room) = self.rooms().get(&scene_id) {
            let _ = room.commands.try_send(cmd);
        }
    }

    // --- Called from HTTP handlers when something changes outside the room ---

    /// A member was removed, left, or changed role: take them out of every
    /// room of that campaign at once (CMP-007). With `rejoin`, the client may
    /// join again (and will get its new role).
    pub fn kick_from_campaign(&self, campaign_id: i64, user_id: i64, reason: &str, rejoin: bool) {
        let mut conns = self.conns();
        for (id, c) in conns.iter_mut() {
            if c.user_id == user_id && c.room.is_some_and(|(_, cid)| cid == campaign_id) {
                let (scene_id, _) = c.room.take().expect("checked above");
                let _ = c.outbox.try_send(Outgoing::Msg(ServerMsg::Kicked {
                    reason: reason.to_owned(),
                    rejoin,
                }));
                if let Some(room) = self.rooms().get(&scene_id) {
                    let _ = room.commands.try_send(RoomCmd::Leave { conn: *id });
                }
            }
        }
    }

    /// The session was signed out: close its sockets.
    pub fn close_session(&self, session_hash: &[u8]) {
        for c in self.conns().values() {
            if c.session_hash == session_hash {
                let _ = c.outbox.try_send(Outgoing::Close("Signed out.".into()));
            }
        }
    }

    /// The password changed: close the user's sockets in every other session.
    pub fn close_other_sessions(&self, user_id: i64, keep_hash: &[u8]) {
        for c in self.conns().values() {
            if c.user_id == user_id && c.session_hash != keep_hash {
                let _ = c
                    .outbox
                    .try_send(Outgoing::Close("Your password was changed.".into()));
            }
        }
    }

    /// The scene's settings changed: the room reloads them and tells everyone.
    pub fn scene_changed(&self, scene_id: i64) {
        self.tell_room(scene_id, RoomCmd::Reload);
    }

    /// The scene was deleted: close its room.
    pub fn scene_deleted(&self, scene_id: i64, reason: &str) {
        let room = self.rooms().remove(&scene_id);
        if let Some(room) = room {
            let _ = room.commands.try_send(RoomCmd::Shutdown {
                reason: reason.to_owned(),
            });
        }
        for c in self.conns().values_mut() {
            if c.room.is_some_and(|(sid, _)| sid == scene_id) {
                c.room = None;
            }
        }
    }

    /// The campaign was deleted: close all its rooms.
    pub fn campaign_deleted(&self, campaign_id: i64) {
        let scenes: Vec<i64> = self
            .rooms()
            .iter()
            .filter(|(_, r)| r.campaign_id == campaign_id)
            .map(|(id, _)| *id)
            .collect();
        for scene_id in scenes {
            self.scene_deleted(scene_id, "The GM deleted this campaign.");
        }
    }

    /// Sends a message to every open connection of these users.
    pub fn notify_users(&self, user_ids: &[i64], msg: ServerMsg) {
        for c in self.conns().values() {
            if user_ids.contains(&c.user_id) {
                let _ = c.outbox.try_send(Outgoing::Msg(msg.clone()));
            }
        }
    }

    /// How many connections are open. For tests and logs.
    pub fn connection_count(&self) -> usize {
        self.conns().len()
    }
}
