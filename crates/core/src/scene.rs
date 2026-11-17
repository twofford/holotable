//! The rules for changing a scene's tokens (milestone M4), as pure functions.
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

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{
    geometry::{Point, Segment, crosses_any},
    uvtt::UvttMap,
};

const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

pub const MAX_TOKEN_NAME_CHARS: usize = 40;
/// Token sizes, in squares per side (spec TOK-001: 1×1 to 4×4).
pub const MIN_TOKEN_SIZE: u8 = 1;
pub const MAX_TOKEN_SIZE: u8 = 4;
/// Most tokens one scene can hold. Keeps a buggy or hostile client from
/// filling the database.
pub const MAX_TOKENS_PER_SCENE: usize = 500;

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

/// The scene's shape, for bounds and wall checks. Built once per room.
#[derive(Debug, Clone)]
pub struct SceneRules {
    /// Map picture size, in pixels.
    pub width: f64,
    pub height: f64,
    /// The scene grid: pixels per square, and where the first grid lines are.
    pub pixels_per_grid: f64,
    pub offset_x: f64,
    pub offset_y: f64,
    /// Walls and closed doors, in image pixels. Movement can't cross these.
    pub barriers: Vec<Segment>,
}

impl SceneRules {
    /// Converts the UVTT file's walls and closed doors (in the *file's* grid
    /// squares) to image pixels, which don't change when the GM adjusts the
    /// scene's grid.
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

        let mut barriers = Vec::new();
        for wall in &map.line_of_sight {
            // `windows(2)` walks each pair of neighbouring points: one segment each.
            for pair in wall.windows(2) {
                barriers.push(Segment::new(to_px(pair[0]), to_px(pair[1])));
            }
        }
        for door in map.portals.iter().filter(|p| p.closed) {
            barriers.push(Segment::new(to_px(door.bounds[0]), to_px(door.bounds[1])));
        }
        Self {
            width: f64::from(width),
            height: f64::from(height),
            pixels_per_grid,
            offset_x,
            offset_y,
            barriers,
        }
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
                if crosses_any(path, &rules.barriers) {
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
    }
}

/// The result of applying a change: the event, and whether the token was
/// hidden before and after, which decides what players are told.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub event: SceneEvent,
    pub hidden_before: bool,
    pub hidden_after: bool,
}

/// Applies a checked (and saved) change to the state. For `Create`, the token
/// must have its real id by now.
pub fn apply(tokens: &mut Tokens, change: Change) -> Applied {
    match change {
        Change::Create(token) => {
            let hidden = token.hidden;
            tokens.insert(token.id, token.clone());
            Applied {
                event: SceneEvent::TokenCreated { token },
                hidden_before: hidden,
                hidden_after: hidden,
            }
        }
        Change::Update(token) => {
            let before = tokens.get(&token.id).is_some_and(|t| t.hidden);
            let after = token.hidden;
            tokens.insert(token.id, token.clone());
            Applied {
                event: SceneEvent::TokenUpdated { token },
                hidden_before: before,
                hidden_after: after,
            }
        }
        Change::Move { token_id, x, y } => {
            // `get_mut` borrows the stored token mutably, so the update is in place.
            let hidden = match tokens.get_mut(&token_id) {
                Some(t) => {
                    t.x = x;
                    t.y = y;
                    t.hidden
                }
                None => false,
            };
            Applied {
                event: SceneEvent::TokenMoved { token_id, x, y },
                hidden_before: hidden,
                hidden_after: hidden,
            }
        }
        Change::Remove { token_id } => {
            let hidden = tokens.remove(&token_id).is_some_and(|t| t.hidden);
            Applied {
                event: SceneEvent::TokenRemoved { token_id },
                hidden_before: hidden,
                hidden_after: hidden,
            }
        }
    }
}

/// What a player is told about an event, if anything. Hidden tokens don't
/// exist for players: hiding one looks like it was removed, and revealing one
/// looks like it was created.
pub fn for_player(applied: &Applied) -> Option<SceneEvent> {
    let (before, after) = (applied.hidden_before, applied.hidden_after);
    match &applied.event {
        SceneEvent::TokenUpdated { token } => match (before, after) {
            (true, true) => None,
            (false, true) => Some(SceneEvent::TokenRemoved { token_id: token.id }),
            (true, false) => Some(SceneEvent::TokenCreated {
                token: token.clone(),
            }),
            (false, false) => Some(applied.event.clone()),
        },
        _ if after || before => None,
        other => Some(other.clone()),
    }
}

/// The tokens someone should see when they join.
pub fn visible_tokens(tokens: &Tokens, is_gm: bool) -> Vec<Token> {
    tokens
        .values()
        .filter(|t| is_gm || !t.hidden)
        .cloned()
        .collect()
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
        SceneRules {
            width: 500.0,
            height: 500.0,
            pixels_per_grid: 50.0,
            offset_x: 0.0,
            offset_y: 0.0,
            barriers: vec![Segment::new(
                Point::new(250.0, 0.0),
                Point::new(250.0, 500.0),
            )],
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
        let visible = visible_tokens(&tokens(), false);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible_tokens(&tokens(), true).len(), 2);
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
    fn apply_updates_state_and_reports_hidden_changes() {
        let mut t = tokens();
        let applied = apply(
            &mut t,
            Change::Move {
                token_id: 1,
                x: 3.0,
                y: 4.0,
            },
        );
        assert_eq!((t[&1].x, t[&1].y), (3.0, 4.0));
        assert_eq!(
            for_player(&applied),
            Some(SceneEvent::TokenMoved {
                token_id: 1,
                x: 3.0,
                y: 4.0
            })
        );

        // The GM hides Vex's token: players are told it was removed.
        let hidden = Token {
            hidden: true,
            ..t[&1].clone()
        };
        let applied = apply(&mut t, Change::Update(hidden));
        assert_eq!(
            for_player(&applied),
            Some(SceneEvent::TokenRemoved { token_id: 1 })
        );

        // Moving a hidden token tells players nothing.
        let applied = apply(
            &mut t,
            Change::Move {
                token_id: 1,
                x: 5.0,
                y: 5.0,
            },
        );
        assert_eq!(for_player(&applied), None);

        // Revealing it looks like it was just created, at its new position.
        let shown = Token {
            hidden: false,
            ..t[&1].clone()
        };
        match for_player(&apply(&mut t, Change::Update(shown))) {
            Some(SceneEvent::TokenCreated { token }) => assert_eq!((token.x, token.y), (5.0, 5.0)),
            other => panic!("unexpected {other:?}"),
        }

        // Removing a hidden token tells players nothing.
        assert_eq!(
            for_player(&apply(&mut t, Change::Remove { token_id: 2 })),
            None
        );
        assert!(!t.contains_key(&2));
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
        assert_eq!(rules.barriers.len(), segments + closed);
        // Every barrier lies on the picture.
        assert!(
            rules
                .barriers
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
