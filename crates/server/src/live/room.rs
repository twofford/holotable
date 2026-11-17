//! A scene's room: the one place that scene's live state is kept and changed.
//!
//! Each room is a tokio task that owns its state outright and receives
//! commands through a channel (the "actor" pattern). Commands are handled one
//! at a time, in order, so two players moving tokens at the same moment can't
//! interleave half-finished changes, and nothing here needs a lock.
//!
//! For a change, the room: checks it (`holotable_core::scene::plan`), saves it
//! to SQLite, applies it to the in-memory state (`apply`), gives it the next
//! sequence number, and sends it to everyone in the room, filtered per person.
//! Only then does the sender get its `Ack` (SYN-002, SYN-003, SYN-008).

use std::collections::{HashMap, VecDeque};

use holotable_core::{
    campaign::Role,
    protocol::{Resume, ServerMsg},
    scene::{
        Actor, Applied, Change, RejectCode, Rejection, SceneOp, SceneRules, Token, Tokens, apply,
        for_player, plan, visible_tokens,
    },
    uvtt::UvttMap,
};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use super::{ConnId, Outbox, Outgoing};
use crate::{db, error::AppError, storage::Storage};

/// How many recent events a room remembers, for clients catching up after a
/// short disconnect. Older than that, a client gets a full snapshot instead.
const EVENT_LOG_LEN: usize = 256;

/// A row of the `tokens` table. `FromRow` matches columns to fields by name,
/// which reads better than an eight-element tuple.
#[derive(sqlx::FromRow)]
struct TokenRow {
    id: i64,
    name: String,
    asset_id: Option<i64>,
    x: f64,
    y: f64,
    size: i64,
    owner_id: Option<i64>,
    hidden: bool,
}

impl From<TokenRow> for Token {
    fn from(r: TokenRow) -> Self {
        // The CHECK constraint keeps size in 1..=4; `clamp` makes the cast safe regardless.
        let size = r.size.clamp(1, 4) as u8;
        Token {
            id: r.id,
            name: r.name,
            asset_id: r.asset_id,
            x: r.x,
            y: r.y,
            size,
            owner_id: r.owner_id,
            hidden: r.hidden,
        }
    }
}

/// What a room can be asked to do.
pub enum RoomCmd {
    Join {
        conn: ConnId,
        user_id: i64,
        role: Role,
        outbox: Outbox,
        resume: Option<Resume>,
    },
    Leave {
        conn: ConnId,
    },
    Op {
        conn: ConnId,
        req_id: u32,
        op: SceneOp,
    },
    Drag {
        conn: ConnId,
        token_id: i64,
        x: f64,
        y: f64,
    },
    /// The scene's settings changed in the database (the grid): reload them.
    Reload,
    /// The scene is gone. Tell everyone, then stop.
    Shutdown {
        reason: String,
    },
}

/// Someone in the room.
struct Member {
    user_id: i64,
    role: Role,
    outbox: Outbox,
}

pub struct Room {
    scene_id: i64,
    campaign_id: i64,
    db: SqlitePool,
    storage: std::sync::Arc<Storage>,
    /// Identifies this run of the room; see `Resume`.
    epoch: String,
    seq: u64,
    tokens: Tokens,
    rules: SceneRules,
    members: HashMap<ConnId, Member>,
    /// Recent events with their sequence numbers, oldest first.
    log: VecDeque<(u64, Applied)>,
}

impl Room {
    /// Loads a scene from the database and its map file.
    pub async fn load(
        db: SqlitePool,
        storage: std::sync::Arc<Storage>,
        scene_id: i64,
    ) -> Result<Room, AppError> {
        let row: Option<(i64, i64, f64, f64, f64, u32, u32)> = sqlx::query_as(
            "SELECT s.campaign_id, s.asset_id, s.pixels_per_grid, s.grid_offset_x, s.grid_offset_y, a.width, a.height
             FROM scenes s JOIN assets a ON a.id = s.asset_id WHERE s.id = ?",
        )
        .bind(scene_id)
        .fetch_optional(&db)
        .await?;
        let (campaign_id, asset_id, ppg, ox, oy, width, height) =
            row.ok_or_else(crate::error::not_found)?;
        let map = read_map(&storage, campaign_id, asset_id).await?;
        let rules = SceneRules::new(&map, width, height, ppg, ox, oy);

        let rows: Vec<TokenRow> = sqlx::query_as(
            "SELECT id, name, asset_id, x, y, size, owner_id, hidden FROM tokens WHERE scene_id = ? ORDER BY id",
        )
        .bind(scene_id)
        .fetch_all(&db)
        .await?;
        let tokens = rows.into_iter().map(|r| (r.id, r.into())).collect();

        Ok(Room {
            scene_id,
            campaign_id,
            db,
            storage,
            epoch: new_epoch(),
            seq: 0,
            tokens,
            rules,
            members: HashMap::new(),
            log: VecDeque::new(),
        })
    }

    pub fn campaign_id(&self) -> i64 {
        self.campaign_id
    }

    /// The room's main loop. `self` is moved in: the task owns the room, and
    /// the room ends when the loop does.
    pub async fn run(mut self, mut commands: mpsc::Receiver<RoomCmd>) {
        tracing::debug!(scene_id = self.scene_id, "room started");
        // `recv` returns `None` once every sender is gone: nobody can reach
        // this room any more, so it can stop.
        while let Some(cmd) = commands.recv().await {
            match cmd {
                RoomCmd::Join {
                    conn,
                    user_id,
                    role,
                    outbox,
                    resume,
                } => self.join(conn, user_id, role, outbox, resume),
                RoomCmd::Leave { conn } => {
                    self.members.remove(&conn);
                }
                RoomCmd::Op { conn, req_id, op } => self.op(conn, req_id, op).await,
                RoomCmd::Drag {
                    conn,
                    token_id,
                    x,
                    y,
                } => self.drag(conn, token_id, x, y),
                RoomCmd::Reload => self.reload().await,
                RoomCmd::Shutdown { reason } => {
                    for m in self.members.values() {
                        let _ = m.outbox.try_send(Outgoing::Msg(ServerMsg::Kicked {
                            reason: reason.clone(),
                            rejoin: false,
                        }));
                    }
                    break;
                }
            }
        }
        tracing::debug!(scene_id = self.scene_id, "room stopped");
    }

    fn join(
        &mut self,
        conn: ConnId,
        user_id: i64,
        role: Role,
        outbox: Outbox,
        resume: Option<Resume>,
    ) {
        let member = Member {
            user_id,
            role,
            outbox,
        };
        // Catch up from where the client left off, if this room still
        // remembers everything since then; otherwise send a full snapshot.
        let oldest = self.log.front().map_or(self.seq + 1, |(seq, _)| *seq);
        match resume {
            Some(r) if r.epoch == self.epoch && r.seq <= self.seq && r.seq + 1 >= oldest => {
                for (seq, applied) in self.log.iter().filter(|(seq, _)| *seq > r.seq) {
                    if let Some(event) = view(&member, applied) {
                        send(&member, ServerMsg::Event { seq: *seq, event });
                    }
                }
            }
            _ => {
                send(
                    &member,
                    ServerMsg::SceneSnapshot {
                        scene_id: self.scene_id,
                        epoch: self.epoch.clone(),
                        seq: self.seq,
                        role,
                        tokens: visible_tokens(&self.tokens, role == Role::Gm),
                    },
                );
            }
        }
        self.members.insert(conn, member);
    }

    async fn op(&mut self, conn: ConnId, req_id: u32, op: SceneOp) {
        let Some(member) = self.members.get(&conn) else {
            return;
        };
        let actor = Actor {
            user_id: member.user_id,
            is_gm: member.role == Role::Gm,
        };
        match self.try_op(actor, &op).await {
            Ok((seq, token_id)) => {
                if let Some(m) = self.members.get(&conn) {
                    send(
                        m,
                        ServerMsg::Ack {
                            req_id,
                            seq,
                            token_id,
                        },
                    );
                }
            }
            Err(r) => {
                if let Some(m) = self.members.get(&conn) {
                    send(
                        m,
                        ServerMsg::Reject {
                            req_id,
                            code: r.code,
                            message: r.message,
                        },
                    );
                }
            }
        }
    }

    /// Check, save, apply, broadcast. Returns the new sequence number and,
    /// for a new token, its id.
    async fn try_op(
        &mut self,
        actor: Actor,
        op: &SceneOp,
    ) -> Result<(u64, Option<i64>), Rejection> {
        // Checks that need the database: the picture and the owner must
        // belong to this campaign.
        if let SceneOp::CreateToken { settings, .. } | SceneOp::UpdateToken { settings, .. } = op {
            self.check_references(settings.asset_id, settings.owner_id)
                .await?;
        }
        let change = plan(&self.tokens, &self.rules, actor, op)?;
        let change = self.save(change).await.map_err(|e| {
            tracing::error!(error = %e, scene_id = self.scene_id, "saving a scene change failed");
            Rejection {
                code: RejectCode::Internal,
                message: "The server couldn't save that.".into(),
            }
        })?;
        let created_id = match &change {
            Change::Create(t) => Some(t.id),
            _ => None,
        };
        let applied = apply(&mut self.tokens, change);
        self.seq += 1;
        self.broadcast(self.seq, &applied);
        self.log.push_back((self.seq, applied));
        if self.log.len() > EVENT_LOG_LEN {
            self.log.pop_front();
        }
        Ok((self.seq, created_id))
    }

    async fn check_references(
        &self,
        asset_id: Option<i64>,
        owner_id: Option<i64>,
    ) -> Result<(), Rejection> {
        let invalid = |m: &str| Rejection {
            code: RejectCode::Invalid,
            message: m.to_owned(),
        };
        let internal = |_| Rejection {
            code: RejectCode::Internal,
            message: "The server couldn't check that.".into(),
        };
        if let Some(asset_id) = asset_id {
            let kind: Option<String> =
                sqlx::query_scalar("SELECT kind FROM assets WHERE id = ? AND campaign_id = ?")
                    .bind(asset_id)
                    .bind(self.campaign_id)
                    .fetch_optional(&self.db)
                    .await
                    .map_err(internal)?;
            if kind.as_deref() != Some("image") {
                return Err(invalid("Pick an uploaded image for the token."));
            }
        }
        if let Some(owner_id) = owner_id {
            let member: Option<i64> = sqlx::query_scalar(
                "SELECT user_id FROM memberships WHERE campaign_id = ? AND user_id = ?",
            )
            .bind(self.campaign_id)
            .bind(owner_id)
            .fetch_optional(&self.db)
            .await
            .map_err(internal)?;
            if member.is_none() {
                return Err(invalid("A token's owner must be a member of the campaign."));
            }
        }
        Ok(())
    }

    /// Writes a change to the database. A new token gets its id here.
    async fn save(&self, change: Change) -> Result<Change, sqlx::Error> {
        Ok(match change {
            Change::Create(mut t) => {
                t.id = sqlx::query(
                    "INSERT INTO tokens (scene_id, name, asset_id, x, y, size, owner_id, hidden, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(self.scene_id)
                .bind(&t.name)
                .bind(t.asset_id)
                .bind(t.x)
                .bind(t.y)
                .bind(i64::from(t.size))
                .bind(t.owner_id)
                .bind(t.hidden)
                .bind(db::now())
                .execute(&self.db)
                .await?
                .last_insert_rowid();
                Change::Create(t)
            }
            Change::Update(t) => {
                sqlx::query("UPDATE tokens SET name = ?, asset_id = ?, size = ?, owner_id = ?, hidden = ? WHERE id = ? AND scene_id = ?")
                    .bind(&t.name)
                    .bind(t.asset_id)
                    .bind(i64::from(t.size))
                    .bind(t.owner_id)
                    .bind(t.hidden)
                    .bind(t.id)
                    .bind(self.scene_id)
                    .execute(&self.db)
                    .await?;
                Change::Update(t)
            }
            Change::Move { token_id, x, y } => {
                sqlx::query("UPDATE tokens SET x = ?, y = ? WHERE id = ? AND scene_id = ?")
                    .bind(x)
                    .bind(y)
                    .bind(token_id)
                    .bind(self.scene_id)
                    .execute(&self.db)
                    .await?;
                Change::Move { token_id, x, y }
            }
            Change::Remove { token_id } => {
                sqlx::query("DELETE FROM tokens WHERE id = ? AND scene_id = ?")
                    .bind(token_id)
                    .bind(self.scene_id)
                    .execute(&self.db)
                    .await?;
                Change::Remove { token_id }
            }
        })
    }

    /// Sends an event to everyone, as each person may see it.
    fn broadcast(&mut self, seq: u64, applied: &Applied) {
        let mut stalled = Vec::new();
        for (conn, member) in &self.members {
            if let Some(event) = view(member, applied)
                && !send(member, ServerMsg::Event { seq, event })
            {
                stalled.push(*conn);
            }
        }
        // A client that can't keep up has missed an event and is out of date.
        // Drop it from the room; when it notices, it joins again and gets a
        // fresh snapshot.
        for conn in stalled {
            tracing::warn!(
                scene_id = self.scene_id,
                conn,
                "client too slow; removed from room"
            );
            if let Some(m) = self.members.remove(&conn) {
                let _ = m
                    .outbox
                    .try_send(Outgoing::Close("Your connection fell behind.".into()));
            }
        }
    }

    /// Shows a token moving to everyone else in the room while it's dragged.
    fn drag(&self, conn: ConnId, token_id: i64, x: f64, y: f64) {
        let Some(member) = self.members.get(&conn) else {
            return;
        };
        let Some(token) = self.tokens.get(&token_id) else {
            return;
        };
        let is_gm = member.role == Role::Gm;
        // Same permission as a move: the GM, or the token's owner.
        if !is_gm && (token.hidden || token.owner_id != Some(member.user_id)) {
            return;
        }
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let msg = ServerMsg::TokenDragging {
            token_id,
            x,
            y,
            user_id: member.user_id,
        };
        for (other, m) in &self.members {
            if *other != conn && (m.role == Role::Gm || !token.hidden) {
                send(m, msg.clone());
            }
        }
    }

    /// Reloads the scene's grid from the database and tells everyone.
    async fn reload(&mut self) {
        match Room::load(self.db.clone(), self.storage.clone(), self.scene_id).await {
            Ok(fresh) => {
                self.rules = fresh.rules;
                for m in self.members.values() {
                    send(
                        m,
                        ServerMsg::SceneChanged {
                            scene_id: self.scene_id,
                        },
                    );
                }
            }
            Err(e) => tracing::error!(error = %e, scene_id = self.scene_id, "room reload failed"),
        }
    }
}

/// What one member sees of an event.
fn view(member: &Member, applied: &Applied) -> Option<holotable_core::scene::SceneEvent> {
    if member.role == Role::Gm {
        Some(applied.event.clone())
    } else {
        for_player(applied)
    }
}

/// Queues a message for a member. `try_send` never waits: if the member's
/// queue is full, it returns `false` rather than stalling the whole room.
fn send(member: &Member, msg: ServerMsg) -> bool {
    member.outbox.try_send(Outgoing::Msg(msg)).is_ok()
}

fn new_epoch() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("the OS random number generator works");
    hex::encode(bytes)
}

async fn read_map(storage: &Storage, campaign_id: i64, asset_id: i64) -> Result<UvttMap, AppError> {
    let json = tokio::fs::read(storage.map_path(campaign_id, asset_id)).await?;
    serde_json::from_slice(&json)
        .map_err(|e| AppError::Internal(format!("stored map data is invalid: {e}")))
}
