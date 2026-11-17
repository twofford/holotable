//! A scene's room: the one place that scene's live state is kept and changed.
//!
//! Each room is a tokio task that owns its state outright and receives
//! commands through a channel (the "actor" pattern). Commands are handled one
//! at a time, in order, so two players moving tokens at the same moment can't
//! interleave half-finished changes, and nothing here needs a lock.
//!
//! For a change, the room: checks it (`holotable_core::scene::plan`), saves it
//! to SQLite, applies it to the in-memory state (`apply`), gives it the next
//! sequence number, and sends it to everyone in the room. Only then does the
//! sender get its `Ack` (SYN-002, SYN-003, SYN-008).
//!
//! GMs get every event. Each player is told only about the tokens their own
//! tokens can see (VIS-008): the room remembers which tokens each player knows
//! about, works out the new set after every change, and sends the difference.

use std::collections::{BTreeSet, HashMap, VecDeque};

use holotable_core::{
    campaign::Role,
    protocol::{Explored, Resume, ServerMsg},
    scene::{
        Actor, Change, DoorState, RejectCode, Rejection, SceneEvent, SceneOp, SceneRules, Token,
        Tokens, apply, explore, plan, player_events, visible_to,
    },
    uvtt::UvttMap,
    vision::ExploredLayout,
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
    vision: bool,
    vision_range: Option<f64>,
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
            vision: r.vision,
            vision_range: r.vision_range,
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
    /// For players: the tokens they currently know about. `None` for GMs,
    /// who know about everything.
    known: Option<BTreeSet<i64>>,
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
    /// Recent events with their sequence numbers, oldest first. Only GMs
    /// catch up from this; players get a fresh snapshot of their own view.
    log: VecDeque<(u64, SceneEvent)>,
    /// What each player in the room has explored, by user id, loaded when
    /// they first join and saved whenever it grows.
    explored: HashMap<i64, Vec<u8>>,
    layout: ExploredLayout,
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
        let mut rules = SceneRules::new(&map, width, height, ppg, ox, oy);
        let doors: Vec<(u32, bool, bool)> =
            sqlx::query_as("SELECT door, open, locked FROM scene_doors WHERE scene_id = ?")
                .bind(scene_id)
                .fetch_all(&db)
                .await?;
        for (door, open, locked) in doors {
            rules.set_door(door, DoorState { open, locked });
        }

        let rows: Vec<TokenRow> = sqlx::query_as(
            "SELECT id, name, asset_id, x, y, size, owner_id, hidden, vision, vision_range
             FROM tokens WHERE scene_id = ? ORDER BY id",
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
            layout: ExploredLayout::for_map(rules.width, rules.height),
            rules,
            members: HashMap::new(),
            log: VecDeque::new(),
            explored: HashMap::new(),
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
                } => self.join(conn, user_id, role, outbox, resume).await,
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

    async fn join(
        &mut self,
        conn: ConnId,
        user_id: i64,
        role: Role,
        outbox: Outbox,
        resume: Option<Resume>,
    ) {
        if role == Role::Gm {
            let member = Member {
                user_id,
                role,
                outbox,
                known: None,
            };
            // Catch up from where the client left off, if this room still
            // remembers everything since then; otherwise send a full snapshot.
            let oldest = self.log.front().map_or(self.seq + 1, |(seq, _)| *seq);
            match resume {
                Some(r) if r.epoch == self.epoch && r.seq <= self.seq && r.seq + 1 >= oldest => {
                    for (seq, event) in self.log.iter().filter(|(seq, _)| *seq > r.seq) {
                        send(
                            &member,
                            ServerMsg::Event {
                                seq: *seq,
                                event: event.clone(),
                            },
                        );
                    }
                }
                _ => {
                    let tokens = self.tokens.values().cloned().collect();
                    send(&member, self.snapshot(role, tokens, None));
                }
            }
            self.members.insert(conn, member);
            return;
        }

        // A player always gets a snapshot of what they can see right now.
        self.load_explored(user_id).await;
        self.explore(user_id).await;
        let known = visible_to(&self.tokens, &self.rules, user_id);
        let tokens = known
            .iter()
            .filter_map(|id| self.tokens.get(id))
            .cloned()
            .collect();
        let explored = self
            .explored
            .get(&user_id)
            .map(|bits| Explored::new(self.layout, bits));
        let member = Member {
            user_id,
            role,
            outbox,
            known: Some(known),
        };
        send(&member, self.snapshot(role, tokens, explored));
        self.members.insert(conn, member);
    }

    fn snapshot(&self, role: Role, tokens: Vec<Token>, explored: Option<Explored>) -> ServerMsg {
        ServerMsg::SceneSnapshot {
            scene_id: self.scene_id,
            epoch: self.epoch.clone(),
            seq: self.seq,
            role,
            tokens,
            doors: self.rules.doors().to_vec(),
            explored,
        }
    }

    /// Reads a player's explored fog from the database, once per room.
    async fn load_explored(&mut self, user_id: i64) {
        if self.explored.contains_key(&user_id) {
            return;
        }
        let saved: Option<Vec<u8>> = sqlx::query_scalar(
            "SELECT bits FROM scene_explored WHERE scene_id = ? AND user_id = ?",
        )
        .bind(self.scene_id)
        .bind(user_id)
        .fetch_optional(&self.db)
        .await
        .unwrap_or_else(|e| {
            tracing::error!(error = %e, "loading explored fog failed");
            None
        });
        let len = self.layout.byte_len();
        // A saved value of the wrong size (the picture changed?) starts afresh.
        let bits = saved
            .filter(|b| b.len() == len)
            .unwrap_or_else(|| vec![0; len]);
        self.explored.insert(user_id, bits);
    }

    /// Adds what a player's tokens see now to their explored fog, and saves
    /// it if it grew.
    async fn explore(&mut self, user_id: i64) {
        let Some(bits) = self.explored.get_mut(&user_id) else {
            return;
        };
        if !explore(&self.tokens, &self.rules, user_id, self.layout, bits) {
            return;
        }
        let result = sqlx::query(
            "INSERT INTO scene_explored (scene_id, user_id, bits) VALUES (?, ?, ?)
             ON CONFLICT (scene_id, user_id) DO UPDATE SET bits = excluded.bits",
        )
        .bind(self.scene_id)
        .bind(user_id)
        .bind(bits.as_slice())
        .execute(&self.db)
        .await;
        if let Err(e) = result {
            tracing::error!(error = %e, scene_id = self.scene_id, "saving explored fog failed");
        }
    }

    async fn op(&mut self, conn: ConnId, req_id: u32, op: SceneOp) {
        let Some(member) = self.members.get(&conn) else {
            return;
        };
        let actor = Actor {
            user_id: member.user_id,
            is_gm: member.role == Role::Gm,
        };
        // To a player, a token they can't see doesn't exist: the same answer
        // as for a made-up id, so probing ids reveals nothing.
        let token_id = match &op {
            SceneOp::MoveToken { token_id, .. }
            | SceneOp::UpdateToken { token_id, .. }
            | SceneOp::DeleteToken { token_id } => Some(*token_id),
            _ => None,
        };
        if let (Some(known), Some(id)) = (&member.known, token_id)
            && !known.contains(&id)
        {
            send(
                member,
                ServerMsg::Reject {
                    req_id,
                    code: RejectCode::NotFound,
                    message: "That token isn't on this scene.".into(),
                },
            );
            return;
        }
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
        let event = apply(&mut self.tokens, &mut self.rules, change);
        self.seq += 1;
        self.after_change(&event).await;
        self.broadcast(self.seq, &event);
        self.log.push_back((self.seq, event));
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

    /// Updates explored fog after a change: for players whose tokens moved or
    /// changed, or for everyone when a door changed or the GM reset it.
    async fn after_change(&mut self, event: &SceneEvent) {
        let players: BTreeSet<i64> = self
            .members
            .values()
            .filter(|m| m.role != Role::Gm)
            .map(|m| m.user_id)
            .collect();
        let affected: Vec<i64> = match event {
            SceneEvent::ExploredReset => {
                for bits in self.explored.values_mut() {
                    bits.fill(0);
                }
                players.into_iter().collect()
            }
            SceneEvent::DoorChanged { .. } => players.into_iter().collect(),
            SceneEvent::TokenCreated { token } | SceneEvent::TokenUpdated { token } => token
                .owner_id
                .into_iter()
                .filter(|o| players.contains(o))
                .collect(),
            SceneEvent::TokenMoved { token_id, .. } => self
                .tokens
                .get(token_id)
                .and_then(|t| t.owner_id)
                .into_iter()
                .filter(|o| players.contains(o))
                .collect(),
            SceneEvent::TokenRemoved { .. } => Vec::new(),
        };
        for user_id in affected {
            self.explore(user_id).await;
        }
    }

    /// Writes a change to the database. A new token gets its id here.
    async fn save(&self, change: Change) -> Result<Change, sqlx::Error> {
        Ok(match change {
            Change::Create(mut t) => {
                t.id = sqlx::query(
                    "INSERT INTO tokens (scene_id, name, asset_id, x, y, size, owner_id, hidden, vision, vision_range, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(self.scene_id)
                .bind(&t.name)
                .bind(t.asset_id)
                .bind(t.x)
                .bind(t.y)
                .bind(i64::from(t.size))
                .bind(t.owner_id)
                .bind(t.hidden)
                .bind(t.vision)
                .bind(t.vision_range)
                .bind(db::now())
                .execute(&self.db)
                .await?
                .last_insert_rowid();
                Change::Create(t)
            }
            Change::Update(t) => {
                sqlx::query("UPDATE tokens SET name = ?, asset_id = ?, size = ?, owner_id = ?, hidden = ?, vision = ?, vision_range = ? WHERE id = ? AND scene_id = ?")
                    .bind(&t.name)
                    .bind(t.asset_id)
                    .bind(i64::from(t.size))
                    .bind(t.owner_id)
                    .bind(t.hidden)
                    .bind(t.vision)
                    .bind(t.vision_range)
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
            Change::Door { door, state } => {
                sqlx::query(
                    "INSERT INTO scene_doors (scene_id, door, open, locked) VALUES (?, ?, ?, ?)
                     ON CONFLICT (scene_id, door) DO UPDATE SET open = excluded.open, locked = excluded.locked",
                )
                .bind(self.scene_id)
                .bind(door)
                .bind(state.open)
                .bind(state.locked)
                .execute(&self.db)
                .await?;
                Change::Door { door, state }
            }
            Change::ResetExplored => {
                sqlx::query("DELETE FROM scene_explored WHERE scene_id = ?")
                    .bind(self.scene_id)
                    .execute(&self.db)
                    .await?;
                Change::ResetExplored
            }
        })
    }

    /// Sends an event to everyone, as each person may see it.
    fn broadcast(&mut self, seq: u64, event: &SceneEvent) {
        let mut stalled = Vec::new();
        for (conn, member) in &mut self.members {
            let events = match &mut member.known {
                None => vec![event.clone()],
                Some(known) => {
                    let now = visible_to(&self.tokens, &self.rules, member.user_id);
                    let events = player_events(known, &now, &self.tokens, event);
                    *known = now;
                    events
                }
            };
            for event in events {
                if !send(member, ServerMsg::Event { seq, event }) {
                    stalled.push(*conn);
                    break;
                }
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
        // Players see the drag only if they know the token and could see it
        // where it is now.
        let at = Token {
            x,
            y,
            ..token.clone()
        };
        for (other, m) in &self.members {
            if *other == conn {
                continue;
            }
            let shown = match &m.known {
                None => true,
                Some(known) => {
                    known.contains(&token_id)
                        && (token.owner_id == Some(m.user_id) || self.player_sees(m.user_id, &at))
                }
            };
            if shown {
                send(m, msg.clone());
            }
        }
    }

    /// Can any of a player's seeing tokens see `target`?
    fn player_sees(&self, user_id: i64, target: &Token) -> bool {
        self.tokens
            .values()
            .filter(|t| t.owner_id == Some(user_id) && !t.hidden && t.vision)
            .any(|eye| self.rules.sees(eye, target))
    }

    /// Reloads the scene's grid from the database and tells everyone.
    async fn reload(&mut self) {
        match Room::load(self.db.clone(), self.storage.clone(), self.scene_id).await {
            Ok(fresh) => {
                self.rules = fresh.rules;
                // The grid moved, so tokens sit in different places on the
                // picture and players may see different things: send each
                // player a fresh view.
                let mut snapshots = Vec::new();
                for (conn, m) in &self.members {
                    if m.known.is_some() {
                        let known = visible_to(&self.tokens, &self.rules, m.user_id);
                        let tokens = known
                            .iter()
                            .filter_map(|id| self.tokens.get(id))
                            .cloned()
                            .collect();
                        let explored = self
                            .explored
                            .get(&m.user_id)
                            .map(|b| Explored::new(self.layout, b));
                        snapshots.push((*conn, known, self.snapshot(m.role, tokens, explored)));
                    }
                }
                for (conn, known, msg) in snapshots {
                    if let Some(m) = self.members.get_mut(&conn) {
                        m.known = Some(known);
                        send(m, msg);
                    }
                }
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
