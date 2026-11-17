//! The rules for changing a scene (tokens in M4; doors and vision in M5), as
//! pure functions.
//!
//! The server's scene room calls `plan` to check a request and work out the
//! change, saves it to the database, then calls `apply` to update its copy of
//! the state and get the event to broadcast. Keeping the rules here, with no
//! I/O, means every rule is tested with plain function calls, and the client
//! could run the same checks later through WASM.
//!
//! Token positions are in **scene grid squares**: `x = 3` is the left edge of
//! the fourth column of the scene's grid (which the GM can adjust; see
//! `SceneDetail::pixels_per_grid`). A token of `size` 2 covers 2 × 2 squares.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    geometry::{Point, Segment, crosses_any, segments_intersect},
    uvtt::UvttMap,
    vision::{ExploredLayout, rasterize, square_visible, visibility_polygon},
};

const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

pub const MAX_TOKEN_NAME_CHARS: usize = 40;
/// Token sizes, in squares per side (spec TOK-001: 1×1 to 4×4).
pub const MIN_TOKEN_SIZE: u8 = 1;
pub const MAX_TOKEN_SIZE: u8 = 4;
/// Most tokens one scene can hold. Keeps a buggy or hostile client from
/// filling the database.
pub const MAX_TOKENS_PER_SCENE: usize = 500;
/// Feet per grid square, for vision ranges.
pub const FEET_PER_SQUARE: f64 = 5.0;
/// Longest vision range a token can have, in feet.
pub const MAX_VISION_RANGE_FT: f64 = 1000.0;

fn yes() -> bool {
    true
}

/// A token on a scene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct Token {
    #[ts(type = "number")]
    pub id: i64,
    pub name: String,
    /// An uploaded image to draw; `None` draws a plain disc with the initial.
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
    /// Top-left corner, in scene grid squares.
    pub x: f64,
    pub y: f64,
    /// Squares per side, 1 to 4.
    pub size: u8,
    /// The player who may move it. GMs can move every token.
    #[ts(type = "number | null")]
    pub owner_id: Option<i64>,
    /// Hidden tokens exist only for GMs: players are never sent them.
    pub hidden: bool,
    /// Whether the token can see (VIS-003). Its owner sees what it sees.
    #[serde(default = "yes")]
    pub vision: bool,
    /// How far it sees, in feet; `None` is unlimited.
    #[serde(default)]
    pub vision_range: Option<f64>,
}

/// The settings of a token that a GM can edit (everything except position).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct TokenSettings {
    pub name: String,
    #[ts(type = "number | null")]
    pub asset_id: Option<i64>,
    pub size: u8,
    #[ts(type = "number | null")]
    pub owner_id: Option<i64>,
    pub hidden: bool,
    #[serde(default = "yes")]
    pub vision: bool,
    #[serde(default)]
    pub vision_range: Option<f64>,
}

/// A door's state. Doors are numbered by their place in the map file's
/// `portals` list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = EXPORT_DIR)]
pub struct DoorState {
    pub open: bool,
    /// Locked doors can't be opened or closed by players (VIS-010).
    pub locked: bool,
}

/// A request to change the scene. The server answers each with `Ack` or `Reject`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = EXPORT_DIR)]
pub enum SceneOp {
    /// GM only (TOK-001).
    CreateToken {
        settings: TokenSettings,
        x: f64,
        y: f64,
    },
    /// GM: any token. Player: only tokens they own (TOK-002).
    MoveToken {
        #[ts(type = "number")]
        token_id: i64,
        x: f64,
        y: f64,
    },
    /// GM only (TOK-005).
    UpdateToken {
        #[ts(type = "number")]
        token_id: i64,
        settings: TokenSettings,
    },
    /// GM only (TOK-005).
    DeleteToken {
        #[ts(type = "number")]
        token_id: i64,
    },
    /// GM: any unlocked or locked door. Player: an unlocked door within one
    /// square of a token they own (VIS-007).
    SetDoor { door: u32, open: bool },
    /// GM only (VIS-010).
    LockDoor { door: u32, locked: bool },
    /// GM only: everyone's explored fog on this scene goes dark again (VIS-005).
    ResetExplored,
}

/// Something that happened to the scene. Sent to everyone in it, filtered per
/// person (players never see hidden tokens).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export, export_to = EXPORT_DIR)]
pub enum SceneEvent {
    TokenCreated {
        token: Token,
    },
    TokenUpdated {
        token: Token,
    },
    TokenMoved {
        #[ts(type = "number")]
        token_id: i64,
        x: f64,
        y: f64,
    },
    TokenRemoved {
        #[ts(type = "number")]
        token_id: i64,
    },
    DoorChanged {
        door: u32,
        open: bool,
        locked: bool,
    },
    ExploredReset,
}

/// Why a request was refused. The client rolls back its optimistic change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = EXPORT_DIR)]
pub enum RejectCode {
    Forbidden,
    NotFound,
    Invalid,
    /// The move would pass through a wall or a closed door (TOK-004).
    Blocked,
    RateLimited,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejection {
    pub code: RejectCode,
    pub message: String,
}

fn reject<T>(code: RejectCode, message: &str) -> Result<T, Rejection> {
    Err(Rejection {
        code,
        message: message.to_owned(),
    })
}

/// Who is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor {
    pub user_id: i64,
    pub is_gm: bool,
}

/// The scene's shape and doors, for bounds, movement and sight. Built once
/// per room; doors change as they're opened and closed.
#[derive(Debug, Clone)]
pub struct SceneRules {
    /// Map picture size, in pixels.
    pub width: f64,
    pub height: f64,
    /// The scene grid: pixels per square, and where the first grid lines are.
    pub pixels_per_grid: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// Walls (UVTT `line_of_sight`), in image pixels: block movement and sight.
    walls: Vec<Segment>,
    /// Objects (UVTT `objects_line_of_sight`): block sight only.
    objects: Vec<Segment>,
    /// Each door's line, in image pixels, and its state.
    door_lines: Vec<Segment>,
    doors: Vec<DoorState>,
    /// Kept up to date by `rebuild`: walls + closed doors (movement), and
    /// walls + objects + closed doors (sight).
    barriers: Vec<Segment>,
    sight: Vec<Segment>,
}

impl SceneRules {
    /// Converts the UVTT file's walls, objects and doors (in the *file's* grid
    /// squares) to image pixels, which don't change when the GM adjusts the
    /// scene's grid. Doors start as the file says.
    pub fn new(
        map: &UvttMap,
        width: u32,
        height: u32,
        pixels_per_grid: f64,
        offset_x: f64,
        offset_y: f64,
    ) -> Self {
        let file_ppg = f64::from(map.resolution.pixels_per_grid);
        let origin = map.resolution.map_origin;
        let to_px = |p: crate::uvtt::GridPoint| {
            Point::new((p.x - origin.x) * file_ppg, (p.y - origin.y) * file_ppg)
        };
        // `windows(2)` walks each pair of neighbouring points: one segment each.
        let lines = |polylines: &Vec<Vec<crate::uvtt::GridPoint>>| -> Vec<Segment> {
            polylines
                .iter()
                .flat_map(|w| {
                    w.windows(2)
                        .map(|pair| Segment::new(to_px(pair[0]), to_px(pair[1])))
                })
                .collect()
        };
        let door_lines = map
            .portals
            .iter()
            .map(|d| Segment::new(to_px(d.bounds[0]), to_px(d.bounds[1])))
            .collect();
        let doors = map
            .portals
            .iter()
            .map(|d| DoorState {
                open: !d.closed,
                locked: false,
            })
            .collect();
        let mut rules = Self {
            width: f64::from(width),
            height: f64::from(height),
            pixels_per_grid,
            offset_x,
            offset_y,
            walls: lines(&map.line_of_sight),
            objects: lines(&map.objects_line_of_sight),
            door_lines,
            doors,
            barriers: Vec::new(),
            sight: Vec::new(),
        };
        rules.rebuild();
        rules
    }

    /// A map of `width` × `height` px with these walls and no doors. For tests.
    pub fn plain(width: f64, height: f64, pixels_per_grid: f64, walls: Vec<Segment>) -> Self {
        let mut rules = Self {
            width,
            height,
            pixels_per_grid,
            offset_x: 0.0,
            offset_y: 0.0,
            walls,
            objects: Vec::new(),
            door_lines: Vec::new(),
            doors: Vec::new(),
            barriers: Vec::new(),
            sight: Vec::new(),
        };
        rules.rebuild();
        rules
    }

    /// Adds a door. For tests.
    pub fn with_door(mut self, line: Segment, state: DoorState) -> Self {
        self.door_lines.push(line);
        self.doors.push(state);
        self.rebuild();
        self
    }

    fn rebuild(&mut self) {
        let closed = self
            .door_lines
            .iter()
            .zip(&self.doors)
            .filter(|(_, d)| !d.open)
            .map(|(line, _)| *line);
        self.barriers = self.walls.iter().copied().chain(closed.clone()).collect();
        self.sight = self
            .walls
            .iter()
            .chain(&self.objects)
            .copied()
            .chain(closed)
            .collect();
    }

    pub fn doors(&self) -> &[DoorState] {
        &self.doors
    }

    pub fn door_lines(&self) -> &[Segment] {
        &self.door_lines
    }

    /// Sets a door's state (loading saved states, or applying a change).
    /// Unknown door numbers are ignored.
    pub fn set_door(&mut self, door: u32, state: DoorState) {
        if let Some(d) = self.doors.get_mut(door as usize) {
            *d = state;
            self.rebuild();
        }
    }

    /// What blocks movement: walls and closed doors.
    pub fn barriers(&self) -> &[Segment] {
        &self.barriers
    }

    /// What blocks sight: walls, objects and closed doors (VIS-002).
    pub fn sight(&self) -> &[Segment] {
        &self.sight
    }

    /// The centre of a token at (x, y) of `size`, in image pixels.
    pub fn center_px(&self, x: f64, y: f64, size: u8) -> Point {
        let half = f64::from(size) / 2.0;
        Point::new(
            (x + half) * self.pixels_per_grid + self.offset_x,
            (y + half) * self.pixels_per_grid + self.offset_y,
        )
    }

    /// A token must have its centre on the map picture.
    fn on_map(&self, x: f64, y: f64, size: u8) -> bool {
        let c = self.center_px(x, y, size);
        x.is_finite()
            && y.is_finite()
            && (0.0..=self.width).contains(&c.x)
            && (0.0..=self.height).contains(&c.y)
    }

    /// A token's vision range in pixels, if limited.
    pub fn range_px(&self, token: &Token) -> Option<f64> {
        token
            .vision_range
            .map(|ft| ft / FEET_PER_SQUARE * self.pixels_per_grid)
    }

    /// The area a token sees, as a polygon in image pixels.
    pub fn vision_polygon(&self, token: &Token) -> Vec<Point> {
        visibility_polygon(
            self.center_px(token.x, token.y, token.size),
            &self.sight,
            self.width,
            self.height,
            self.range_px(token),
        )
    }

    /// Can `viewer` see any part of `target`?
    pub fn sees(&self, viewer: &Token, target: &Token) -> bool {
        square_visible(
            self.center_px(viewer.x, viewer.y, viewer.size),
            self.center_px(target.x, target.y, target.size),
            f64::from(target.size) * self.pixels_per_grid / 2.0,
            &self.sight,
            self.range_px(viewer),
        )
    }

    /// Is the door within one square of the token? True when the door's line
    /// touches the token's square grown by one square on every side.
    pub fn door_near(&self, door: u32, token: &Token) -> bool {
        let Some(line) = self.door_lines.get(door as usize) else {
            return false;
        };
        let ppg = self.pixels_per_grid;
        let x0 = (token.x - 1.0) * ppg + self.offset_x;
        let y0 = (token.y - 1.0) * ppg + self.offset_y;
        let side = (f64::from(token.size) + 2.0) * ppg;
        segment_touches_rect(*line, x0, y0, x0 + side, y0 + side)
    }
}

/// Does segment `s` touch the rectangle from (x0, y0) to (x1, y1)? Either an
/// end is inside, or it crosses one of the four sides.
fn segment_touches_rect(s: Segment, x0: f64, y0: f64, x1: f64, y1: f64) -> bool {
    let inside = |p: Point| (x0..=x1).contains(&p.x) && (y0..=y1).contains(&p.y);
    if inside(s.a) || inside(s.b) {
        return true;
    }
    let (a, b, c, d) = (
        Point::new(x0, y0),
        Point::new(x1, y0),
        Point::new(x1, y1),
        Point::new(x0, y1),
    );
    [(a, b), (b, c), (c, d), (d, a)]
        .into_iter()
        .any(|(p, q)| segments_intersect(s, Segment::new(p, q)))
}

/// A checked change, ready to save and then `apply`.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The token's `id` is 0 until the database assigns one.
    Create(Token),
    Update(Token),
    Move {
        token_id: i64,
        x: f64,
        y: f64,
    },
    Remove {
        token_id: i64,
    },
    Door {
        door: u32,
        state: DoorState,
    },
    ResetExplored,
}

/// The tokens of one scene, by id. A `BTreeMap` keeps them in id order, so
/// snapshots come out the same every time.
pub type Tokens = BTreeMap<i64, Token>;

/// Checks a request against the rules and works out the change. Doesn't
/// modify anything: `tokens` is borrowed, not mutably.
pub fn plan(
    tokens: &Tokens,
    rules: &SceneRules,
    actor: Actor,
    op: &SceneOp,
) -> Result<Change, Rejection> {
    match op {
        SceneOp::CreateToken { settings, x, y } => {
            require_gm(actor)?;
            if tokens.len() >= MAX_TOKENS_PER_SCENE {
                return reject(
                    RejectCode::Invalid,
                    "This scene has as many tokens as it can hold.",
                );
            }
            let settings = check_settings(settings)?;
            if !rules.on_map(*x, *y, settings.size) {
                return reject(RejectCode::Invalid, "Tokens must be placed on the map.");
            }
            Ok(Change::Create(token_from(0, &settings, *x, *y)))
        }

        SceneOp::MoveToken { token_id, x, y } => {
            let token = find_for(tokens, actor, *token_id)?;
            if !actor.is_gm && token.owner_id != Some(actor.user_id) {
                return reject(RejectCode::Forbidden, "You can only move your own tokens.");
            }
            if !rules.on_map(*x, *y, token.size) {
                return reject(RejectCode::Invalid, "Tokens must stay on the map.");
            }
            // Players can't walk through walls; GMs can put tokens anywhere.
            if !actor.is_gm {
                let path = Segment::new(
                    rules.center_px(token.x, token.y, token.size),
                    rules.center_px(*x, *y, token.size),
                );
                if crosses_any(path, rules.barriers()) {
                    return reject(
                        RejectCode::Blocked,
                        "There's a wall or a closed door in the way.",
                    );
                }
            }
            Ok(Change::Move {
                token_id: *token_id,
                x: *x,
                y: *y,
            })
        }

        SceneOp::UpdateToken { token_id, settings } => {
            require_gm(actor)?;
            let token = find_for(tokens, actor, *token_id)?;
            let settings = check_settings(settings)?;
            if !rules.on_map(token.x, token.y, settings.size) {
                return reject(
                    RejectCode::Invalid,
                    "At that size the token would be off the map.",
                );
            }
            Ok(Change::Update(token_from(
                token.id, &settings, token.x, token.y,
            )))
        }

        SceneOp::DeleteToken { token_id } => {
            require_gm(actor)?;
            find_for(tokens, actor, *token_id)?;
            Ok(Change::Remove {
                token_id: *token_id,
            })
        }

        SceneOp::SetDoor { door, open } => {
            let Some(current) = rules.doors().get(*door as usize).copied() else {
                return reject(RejectCode::NotFound, "There's no such door.");
            };
            if !actor.is_gm {
                if current.locked {
                    return reject(RejectCode::Forbidden, "That door is locked.");
                }
                let near = tokens.values().any(|t| {
                    t.owner_id == Some(actor.user_id) && !t.hidden && rules.door_near(*door, t)
                });
                if !near {
                    return reject(
                        RejectCode::Forbidden,
                        "Move one of your tokens next to the door first.",
                    );
                }
            }
            Ok(Change::Door {
                door: *door,
                state: DoorState {
                    open: *open,
                    ..current
                },
            })
        }

        SceneOp::LockDoor { door, locked } => {
            require_gm(actor)?;
            let Some(current) = rules.doors().get(*door as usize).copied() else {
                return reject(RejectCode::NotFound, "There's no such door.");
            };
            Ok(Change::Door {
                door: *door,
                state: DoorState {
                    locked: *locked,
                    ..current
                },
            })
        }

        SceneOp::ResetExplored => {
            require_gm(actor)?;
            Ok(Change::ResetExplored)
        }
    }
}

/// Applies a checked (and saved) change to the state, and returns the event
/// that describes it. For `Create`, the token must have its real id by now.
pub fn apply(tokens: &mut Tokens, rules: &mut SceneRules, change: Change) -> SceneEvent {
    match change {
        Change::Create(token) => {
            tokens.insert(token.id, token.clone());
            SceneEvent::TokenCreated { token }
        }
        Change::Update(token) => {
            tokens.insert(token.id, token.clone());
            SceneEvent::TokenUpdated { token }
        }
        Change::Move { token_id, x, y } => {
            // `get_mut` borrows the stored token mutably, so the update is in place.
            if let Some(t) = tokens.get_mut(&token_id) {
                t.x = x;
                t.y = y;
            }
            SceneEvent::TokenMoved { token_id, x, y }
        }
        Change::Remove { token_id } => {
            tokens.remove(&token_id);
            SceneEvent::TokenRemoved { token_id }
        }
        Change::Door { door, state } => {
            rules.set_door(door, state);
            SceneEvent::DoorChanged {
                door,
                open: state.open,
                locked: state.locked,
            }
        }
        Change::ResetExplored => SceneEvent::ExploredReset,
    }
}

/// The tokens a player may know about right now (VIS-004, VIS-008): their own
/// tokens, and any other token one of their seeing tokens can see. Hidden
/// tokens never count, even their own.
pub fn visible_to(tokens: &Tokens, rules: &SceneRules, user_id: i64) -> BTreeSet<i64> {
    let mine: Vec<&Token> = tokens
        .values()
        .filter(|t| t.owner_id == Some(user_id) && !t.hidden)
        .collect();
    let eyes: Vec<&Token> = mine.iter().copied().filter(|t| t.vision).collect();
    tokens
        .values()
        .filter(|t| !t.hidden)
        .filter(|t| t.owner_id == Some(user_id) || eyes.iter().any(|e| rules.sees(e, t)))
        .map(|t| t.id)
        .collect()
}

/// What one player is told after a change, given which tokens they could see
/// before and can see now. Tokens that came into view arrive as
/// `TokenCreated`, tokens that left it as `TokenRemoved`; the event itself
/// goes through only if it's about something they saw both before and after,
/// or about the scene (doors, explored fog).
pub fn player_events(
    before: &BTreeSet<i64>,
    after: &BTreeSet<i64>,
    tokens: &Tokens,
    event: &SceneEvent,
) -> Vec<SceneEvent> {
    let mut out: Vec<SceneEvent> = before
        .difference(after)
        .map(|&token_id| SceneEvent::TokenRemoved { token_id })
        .collect();
    out.extend(
        after
            .difference(before)
            .filter_map(|id| tokens.get(id))
            .map(|token| SceneEvent::TokenCreated {
                token: token.clone(),
            }),
    );
    let seen = |id: &i64| before.contains(id) && after.contains(id);
    match event {
        SceneEvent::TokenMoved { token_id, .. } if seen(token_id) => out.push(event.clone()),
        SceneEvent::TokenUpdated { token } if seen(&token.id) => out.push(event.clone()),
        SceneEvent::DoorChanged { .. } | SceneEvent::ExploredReset => out.push(event.clone()),
        _ => {}
    }
    out
}

/// Marks what a player's tokens can see now as explored (VIS-005). Returns
/// whether anything new was seen.
pub fn explore(
    tokens: &Tokens,
    rules: &SceneRules,
    user_id: i64,
    layout: ExploredLayout,
    bits: &mut [u8],
) -> bool {
    let mut changed = false;
    for t in tokens
        .values()
        .filter(|t| t.owner_id == Some(user_id) && !t.hidden && t.vision)
    {
        changed |= rasterize(&rules.vision_polygon(t), layout, bits);
    }
    changed
}

/// Trims and checks a token's name.
pub fn normalize_token_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        Err("Give the token a name.")
    } else if name.chars().count() > MAX_TOKEN_NAME_CHARS {
        Err("Use 40 characters or fewer for a token's name.")
    } else if name.chars().any(char::is_control) {
        Err("A token's name can't contain control characters.")
    } else {
        Ok(name.to_owned())
    }
}

fn check_settings(s: &TokenSettings) -> Result<TokenSettings, Rejection> {
    let name = normalize_token_name(&s.name).map_err(|m| Rejection {
        code: RejectCode::Invalid,
        message: m.into(),
    })?;
    if !(MIN_TOKEN_SIZE..=MAX_TOKEN_SIZE).contains(&s.size) {
        return reject(RejectCode::Invalid, "A token is 1 to 4 squares across.");
    }
    if let Some(r) = s.vision_range
        && !(r.is_finite() && r > 0.0 && r <= MAX_VISION_RANGE_FT)
    {
        return reject(
            RejectCode::Invalid,
            "A vision range is more than 0 and at most 1000 feet.",
        );
    }
    Ok(TokenSettings { name, ..s.clone() })
}

fn token_from(id: i64, s: &TokenSettings, x: f64, y: f64) -> Token {
    Token {
        id,
        name: s.name.clone(),
        asset_id: s.asset_id,
        x,
        y,
        size: s.size,
        owner_id: s.owner_id,
        hidden: s.hidden,
        vision: s.vision,
        vision_range: s.vision_range,
    }
}

fn require_gm(actor: Actor) -> Result<(), Rejection> {
    if actor.is_gm {
        Ok(())
    } else {
        reject(RejectCode::Forbidden, "Only the GM can do that.")
    }
}

/// Finds a token the actor is allowed to know about. To a player, a hidden
/// token is indistinguishable from one that doesn't exist.
fn find_for(tokens: &Tokens, actor: Actor, id: i64) -> Result<&Token, Rejection> {
    match tokens.get(&id) {
        Some(t) if actor.is_gm || !t.hidden => Ok(t),
        _ => reject(RejectCode::NotFound, "That token isn't on this scene."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GM: Actor = Actor {
        user_id: 1,
        is_gm: true,
    };
    const VEX: Actor = Actor {
        user_id: 2,
        is_gm: false,
    };
    const ROOK: Actor = Actor {
        user_id: 3,
        is_gm: false,
    };

    /// A 10 × 10 square map at 50 px per square, with one wall down the
    /// middle (x = 250 px, from top to bottom).
    fn rules() -> SceneRules {
        SceneRules::plain(
            500.0,
            500.0,
            50.0,
            vec![Segment::new(
                Point::new(250.0, 0.0),
                Point::new(250.0, 500.0),
            )],
        )
    }

    fn settings(name: &str, owner: Option<i64>, hidden: bool) -> TokenSettings {
        TokenSettings {
            name: name.into(),
            asset_id: None,
            size: 1,
            owner_id: owner,
            hidden,
            vision: true,
            vision_range: None,
        }
    }

    /// Vex's token at (1, 1), and a hidden ambusher at (8, 8).
    fn tokens() -> Tokens {
        let mut t = Tokens::new();
        t.insert(1, token_from(1, &settings("Vex", Some(2), false), 1.0, 1.0));
        t.insert(
            2,
            token_from(2, &settings("Ambusher", None, true), 8.0, 8.0),
        );
        t
    }

    fn mv(token_id: i64, x: f64, y: f64) -> SceneOp {
        SceneOp::MoveToken { token_id, x, y }
    }

    #[test]
    fn only_gms_create_and_names_are_checked() {
        let op = SceneOp::CreateToken {
            settings: settings("  Thug ", None, false),
            x: 2.0,
            y: 2.0,
        };
        match plan(&tokens(), &rules(), GM, &op).unwrap() {
            Change::Create(t) => assert_eq!(t.name, "Thug"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            plan(&tokens(), &rules(), VEX, &op).unwrap_err().code,
            RejectCode::Forbidden
        );

        let blank = SceneOp::CreateToken {
            settings: settings(" ", None, false),
            x: 2.0,
            y: 2.0,
        };
        assert_eq!(
            plan(&tokens(), &rules(), GM, &blank).unwrap_err().code,
            RejectCode::Invalid
        );
        let huge = SceneOp::CreateToken {
            settings: TokenSettings {
                size: 5,
                ..settings("Mech", None, false)
            },
            x: 2.0,
            y: 2.0,
        };
        assert_eq!(
            plan(&tokens(), &rules(), GM, &huge).unwrap_err().code,
            RejectCode::Invalid
        );
    }

    #[test]
    fn players_move_only_their_own_tokens() {
        assert!(plan(&tokens(), &rules(), VEX, &mv(1, 2.0, 1.0)).is_ok());
        assert_eq!(
            plan(&tokens(), &rules(), ROOK, &mv(1, 2.0, 1.0))
                .unwrap_err()
                .code,
            RejectCode::Forbidden
        );
    }

    #[test]
    fn hidden_tokens_dont_exist_for_players() {
        assert_eq!(
            plan(&tokens(), &rules(), VEX, &mv(2, 7.0, 7.0))
                .unwrap_err()
                .code,
            RejectCode::NotFound
        );
        assert!(plan(&tokens(), &rules(), GM, &mv(2, 7.0, 7.0)).is_ok());
        assert_eq!(visible_to(&tokens(), &rules(), 2), BTreeSet::from([1]));
    }

    #[test]
    fn walls_block_players_but_not_gms() {
        // From (1, 1) to (6, 1) crosses the wall at x = 250 px (square 5).
        let blocked = plan(&tokens(), &rules(), VEX, &mv(1, 6.0, 1.0)).unwrap_err();
        assert_eq!(blocked.code, RejectCode::Blocked);
        assert!(plan(&tokens(), &rules(), GM, &mv(1, 6.0, 1.0)).is_ok());
    }

    #[test]
    fn tokens_stay_on_the_map() {
        assert_eq!(
            plan(&tokens(), &rules(), VEX, &mv(1, -1.0, 1.0))
                .unwrap_err()
                .code,
            RejectCode::Invalid
        );
        assert_eq!(
            plan(&tokens(), &rules(), GM, &mv(1, 10.0, 1.0))
                .unwrap_err()
                .code,
            RejectCode::Invalid
        );
        assert_eq!(
            plan(&tokens(), &rules(), GM, &mv(1, f64::NAN, 1.0))
                .unwrap_err()
                .code,
            RejectCode::Invalid
        );
        // A token half off the edge is fine as long as its centre is on the map.
        assert!(plan(&tokens(), &rules(), GM, &mv(1, 9.4, 1.0)).is_ok());
    }

    #[test]
    fn apply_updates_state() {
        let (mut t, mut r) = (tokens(), rules());
        let ev = apply(
            &mut t,
            &mut r,
            Change::Move {
                token_id: 1,
                x: 3.0,
                y: 4.0,
            },
        );
        assert_eq!((t[&1].x, t[&1].y), (3.0, 4.0));
        assert_eq!(
            ev,
            SceneEvent::TokenMoved {
                token_id: 1,
                x: 3.0,
                y: 4.0
            }
        );
        apply(&mut t, &mut r, Change::Remove { token_id: 2 });
        assert!(!t.contains_key(&2));
    }

    /// Vex at (1, 1) and Rook at (7, 1) on opposite sides of the wall; a
    /// thug at (2, 3) on Vex's side.
    fn party() -> Tokens {
        let mut t = Tokens::new();
        t.insert(1, token_from(1, &settings("Vex", Some(2), false), 1.0, 1.0));
        t.insert(
            2,
            token_from(2, &settings("Rook", Some(3), false), 7.0, 1.0),
        );
        t.insert(3, token_from(3, &settings("Thug", None, false), 2.0, 3.0));
        t
    }

    #[test]
    fn players_see_only_what_their_tokens_see() {
        let (t, r) = (party(), rules());
        assert_eq!(visible_to(&t, &r, 2), BTreeSet::from([1, 3]));
        assert_eq!(visible_to(&t, &r, 3), BTreeSet::from([2]));
        // Without vision, Vex's player sees only Vex.
        let mut blind = t.clone();
        blind.get_mut(&1).unwrap().vision = false;
        assert_eq!(visible_to(&blind, &r, 2), BTreeSet::from([1]));
        // With a 5 ft range, the thug two squares away is out of sight.
        let mut short = t.clone();
        short.get_mut(&1).unwrap().vision_range = Some(5.0);
        assert_eq!(visible_to(&short, &r, 2), BTreeSet::from([1]));
        // Hidden tokens never count.
        let mut hidden = t.clone();
        hidden.get_mut(&3).unwrap().hidden = true;
        assert_eq!(visible_to(&hidden, &r, 2), BTreeSet::from([1]));
    }

    #[test]
    fn player_events_follow_their_view() {
        let mut t = party();
        let before = BTreeSet::from([1, 3]);
        // The thug walks behind the wall: Vex's player is told it left.
        t.get_mut(&3).unwrap().x = 8.0;
        let after = visible_to(&t, &rules(), 2);
        let moved = SceneEvent::TokenMoved {
            token_id: 3,
            x: 8.0,
            y: 3.0,
        };
        assert_eq!(
            player_events(&before, &after, &t, &moved),
            vec![SceneEvent::TokenRemoved { token_id: 3 }]
        );
        // Rook's player sees it arrive, as a new token at its new place.
        let rook_after = visible_to(&t, &rules(), 3);
        match &player_events(&BTreeSet::from([2]), &rook_after, &t, &moved)[..] {
            [SceneEvent::TokenCreated { token }] => assert_eq!(token.x, 8.0),
            other => panic!("unexpected {other:?}"),
        }
        // Something seen before and after: the event itself goes through.
        let vex_moved = SceneEvent::TokenMoved {
            token_id: 1,
            x: 1.0,
            y: 2.0,
        };
        assert_eq!(
            player_events(&after, &after, &t, &vex_moved),
            vec![vex_moved.clone()]
        );
    }

    #[test]
    fn doors_open_for_nearby_players_and_block_until_open() {
        // A closed door in the wall, from (250, 0) to (250, 100): squares 0–1 of column 5.
        let door = Segment::new(Point::new(250.0, 0.0), Point::new(250.0, 100.0));
        let closed = DoorState {
            open: false,
            locked: false,
        };
        let mut r = SceneRules::plain(500.0, 500.0, 50.0, vec![]).with_door(door, closed);
        let mut t = party();
        let open = SceneOp::SetDoor {
            door: 0,
            open: true,
        };

        // Vex at (1, 1) is too far; at (4, 1) he's next to it.
        assert_eq!(
            plan(&t, &r, VEX, &open).unwrap_err().code,
            RejectCode::Forbidden
        );
        t.get_mut(&1).unwrap().x = 4.0;
        // Closed, the door hides Rook from Vex.
        assert_eq!(visible_to(&t, &r, 2), BTreeSet::from([1, 3]));
        let change = plan(&t, &r, VEX, &open).unwrap();
        let ev = apply(&mut t, &mut r, change);
        assert_eq!(
            ev,
            SceneEvent::DoorChanged {
                door: 0,
                open: true,
                locked: false
            }
        );
        assert!(visible_to(&t, &r, 2).contains(&2));
        assert!(r.barriers().is_empty());

        // Locked: only the GM can open or close it.
        let lock = plan(
            &t,
            &r,
            GM,
            &SceneOp::LockDoor {
                door: 0,
                locked: true,
            },
        )
        .unwrap();
        apply(&mut t, &mut r, lock);
        let close = SceneOp::SetDoor {
            door: 0,
            open: false,
        };
        assert_eq!(
            plan(&t, &r, VEX, &close).unwrap_err().message,
            "That door is locked."
        );
        assert!(plan(&t, &r, GM, &close).is_ok());
        assert_eq!(
            plan(
                &t,
                &r,
                VEX,
                &SceneOp::LockDoor {
                    door: 0,
                    locked: false
                }
            )
            .unwrap_err()
            .code,
            RejectCode::Forbidden
        );
        assert_eq!(
            plan(
                &t,
                &r,
                GM,
                &SceneOp::SetDoor {
                    door: 9,
                    open: true
                }
            )
            .unwrap_err()
            .code,
            RejectCode::NotFound
        );
    }

    #[test]
    fn explored_area_grows_as_tokens_move() {
        let (mut t, r) = (party(), rules());
        let layout = ExploredLayout::for_map(r.width, r.height);
        let mut bits = vec![0u8; layout.byte_len()];
        assert!(explore(&t, &r, 2, layout, &mut bits));
        let count = |b: &[u8]| b.iter().map(|x| x.count_ones()).sum::<u32>();
        let first = count(&bits);
        // Vex sees the left half of the map: about half the cells.
        let total = layout.cols * layout.rows;
        assert!(
            (first as f64 / total as f64 - 0.5).abs() < 0.02,
            "{first} of {total}"
        );
        // Seeing the same again adds nothing; Rook's side is new to Vex's player.
        assert!(!explore(&t, &r, 2, layout, &mut bits));
        t.get_mut(&1).unwrap().x = 7.0;
        assert!(explore(&t, &r, 2, layout, &mut bits));
        assert_eq!(count(&bits), total);
    }

    #[test]
    fn rules_convert_uvtt_walls_and_closed_doors_to_pixels() {
        let json = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/small-bodega.uvtt"
        ));
        let map = UvttMap::parse(json).unwrap();
        let rules = SceneRules::new(&map, 840, 700, 70.0, 0.0, 0.0);
        let segments: usize = map.line_of_sight.iter().map(|w| w.len() - 1).sum();
        let closed = map.portals.iter().filter(|p| p.closed).count();
        assert_eq!(rules.barriers().len(), segments + closed);
        assert_eq!(rules.doors().len(), map.portals.len());
        // Every barrier lies on the picture.
        assert!(
            rules
                .barriers()
                .iter()
                .all(|s| (0.0..=840.0).contains(&s.a.x) && (0.0..=700.0).contains(&s.b.y))
        );
    }

    #[test]
    fn ops_and_events_serialize_with_type_tags() {
        let op: SceneOp =
            serde_json::from_str(r#"{"type":"moveToken","tokenId":7,"x":1.5,"y":2}"#).unwrap();
        assert_eq!(op, mv(7, 1.5, 2.0));
        let ev = serde_json::to_string(&SceneEvent::TokenRemoved { token_id: 7 }).unwrap();
        assert_eq!(ev, r#"{"type":"tokenRemoved","tokenId":7}"#);
    }
}
