# Holotable

A virtual tabletop for **New York 2069**, built from scratch: Rust server and shared core, TypeScript + React client.

Current milestone: **M4, live tokens.** Everyone on a scene sees the same tokens, live. GMs add tokens (name, picture, size, owner, hidden), edit, hide and delete them, and put everyone on a scene with **Play for everyone**. Players drag their own tokens; walls and closed doors stop them. Moves snap to the grid (hold Alt, or turn Snap off, to place freely) and show the distance while dragging. Right-click a token (or long-press it on a touch screen) for the GM menu. Campaigns, uploads and invites from M3 work as before; the M1 **map sandbox** is still linked from the home page.

## Layout

```
holotable/
├── Cargo.toml              Cargo workspace (all Rust crates build together)
├── rust-toolchain.toml     Pins stable Rust + rustfmt, clippy, wasm32 target
├── crates/
│   ├── core/               holotable-core: protocol, UVTT parser, geometry, scene rules. Native + WASM.
│   ├── rules2069/          New York 2069 rules (check classification so far)
│   └── server/             axum + tokio server; binary is called `holotable`
│       ├── migrations/     SQL that sets up the database, applied at startup
│       ├── src/auth/       sign-up/in/out, sessions, password hashing, rate limits
│       ├── src/campaigns/  campaigns, members, invites, uploads, scenes, access checks
│       ├── src/live/       live scenes: the hub, one room task per scene, message rate limits
│       └── tests/          integration tests that drive the whole app
├── client/                 Vite + React + TypeScript + PixiJS
│   └── src/
│       ├── api/            typed calls to the HTTP API
│       ├── auth/           sign-in screen and account menu
│       ├── pages/          one component per page (home, campaign, scene, invite, sandbox)
│       ├── map/            coordinate spaces, map loading, pan/zoom input, MapCanvas
│       ├── render/         renderer.ts: the only file that uses PixiJS
│       ├── net/            WebSocket connection (shared through a React context)
│       ├── scene/          live scene state, token dragging, token dialog
│       └── protocol/generated/   TypeScript types generated from Rust. Do not edit.
├── fixtures/               Test maps (.uvtt) and the YAML they were generated from
└── .github/workflows/ci.yml   CI: the checks below, on every push
```

## First-time setup (macOS)

1. Install Rust: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
   The first `cargo` command in this folder installs the toolchain named in `rust-toolchain.toml`.
2. Install Node 22 or newer (for example `brew install node`).
3. Install client packages: `cd client && npm install`

## Run it

**One server, like production:**

```sh
cd client && npm run build && cd ..
cargo run
```

Open http://127.0.0.1:3000 and create an account. The database (`data/holotable.db`) and uploads (`data/campaigns/`) are created on first run; `data/` is in `.gitignore`.

1. Start a campaign.
2. Under **Maps and images**, upload one or more `.uvtt` files (the ones in `fixtures/` work), plus any `.png`/`.jpg` pictures you want on tokens.
3. Under **Scenes**, give a scene a name, pick a map, and create it. As GM you can line the grid up with the picture in the bar above the map.
4. Under **Invites**, create a link and open it in another browser (or a private window) to join as a player.
5. In the scene, **Add token**: pick that player as the owner. Drag it in either window and watch the other one. **Play for everyone** moves every player to this scene.

**Inviting someone on another computer:** they have to be able to reach your server. Start it with `HOLOTABLE_ADDR=0.0.0.0:3000 cargo run`, open Holotable on *your* machine at `http://<your LAN IP>:3000` (not 127.0.0.1), and create the invite from there, so the link has an address they can open.

In a scene or the map sandbox:

- **Pan:** drag with the left or middle mouse button, one finger, or the Pencil; or scroll with two fingers on a trackpad.
- **Zoom:** mouse wheel, trackpad pinch, or two-finger pinch on iPad. Zoom is limited to 10%–400%.
- **Fit** shows the whole map again; **Grid** toggles the grid overlay.
- **Tokens:** drag one you own (GMs: any). It snaps to the nearest square on release; hold **Alt** while releasing, or turn **Snap** off, to drop it exactly. Click empty map or press **Esc** to deselect.
- **GM token menu:** right-click a token, or press and hold it on a touch screen: **Edit…**, **Hide from players / Show to players**, **Delete** (or select it and press **Delete**).

**Development, with hot reload:**

```sh
cargo run                    # terminal 1: server on :3000
cd client && npm run dev     # terminal 2: Vite on :5173, forwards /ws and /api to :3000
```

Open http://localhost:5173.

**Test on an iPad:** run the server on all interfaces, then open `http://<your-mac's-LAN-IP>:3000` in Safari on the iPad:

```sh
HOLOTABLE_ADDR=0.0.0.0:3000 cargo run
```

### Settings

All optional, as environment variables:

| Variable | Default | What it does |
| --- | --- | --- |
| `HOLOTABLE_ADDR` | `127.0.0.1:3000` | Address to listen on. `0.0.0.0:3000` to allow other devices on your network. |
| `HOLOTABLE_STATIC_DIR` | `client/dist` | Where the built client is. |
| `HOLOTABLE_DATABASE_URL` | `sqlite://data/holotable.db` | The SQLite database file. |
| `HOLOTABLE_DATA_DIR` | `data` | Where uploads are stored (under `campaigns/`). Back this up along with the database. |
| `HOLOTABLE_SECURE_COOKIES` | `false` in debug builds, `true` in release builds | Mark the session cookie `Secure` (HTTPS only). Must be `true` on a public server. |
| `RUST_LOG` | `holotable=info,...` | Log detail, e.g. `RUST_LOG=debug`. |

To start over, stop the server and delete the `data` folder.

## Checks (the same ones CI runs)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace        # also regenerates client/src/protocol/generated
cargo build -p holotable-core --target wasm32-unknown-unknown
cd client && npm run typecheck && npm test && npm run build
```

If you change a type in `crates/core/src/protocol.rs`, run `cargo test` and commit the regenerated `.ts` files. CI fails if they are out of date.

## What M0 teaches

**Cargo workspace.** One `Cargo.lock` and one `target/` for all crates. Shared settings (edition, dependency versions, lints) live once in the root `Cargo.toml`, and each crate opts in with `edition.workspace = true` or `serde.workspace = true`. The core crate is named `holotable-core`, not `core`, because `core` is the name of Rust's own built-in library.

**Keeping `core` WASM-safe.** `holotable-core` has no tokio, no file or network I/O, and no dependency on the server. That's what lets the same code run on the server and, from M5, in the browser as WASM. CI builds it for `wasm32-unknown-unknown` to keep it honest.

**One type, two languages.** `ClientMsg` and `ServerMsg` are Rust enums with `#[serde(tag = "type")]`. Serde turns `ServerMsg::Pong { nonce: 3 }` into `{"type":"pong","nonce":3}`, and `ts-rs` turns the enum into a TypeScript union, `{ type: "pong", nonce: number } | …`. In the client, `switch (msg.type)` narrows the union, so each `case` knows which fields exist.

**Exhaustiveness on both sides.** Rust's `match` must cover every variant or it won't compile. TypeScript gets the same guarantee from the `default` branch in `connection.ts`: assigning `msg` to a variable of type `never` only compiles if every variant was handled. Add a variant in Rust, and both compilers point to the code that needs updating.

**Ownership of the socket (`ws.rs`).** `handle_socket(mut socket: WebSocket)` takes the socket by value. The task that runs it owns the connection outright: no locks, no sharing, and the connection closes when the function returns and `socket` is dropped. The helpers borrow it instead (`&mut WebSocket`), because they only need it for the length of one call.

**`tokio::select!`.** One loop waits on two things at once, the ping timer and the next incoming message, and runs whichever finishes first. Both of these futures can be cancelled safely, which is the thing to check whenever you use `select!`.

**`Option::take`.** `awaiting.take()` moves the value out of the `Option` and leaves `None` in its place. It's the usual way to consume something you hold through `&mut` without cloning it.

**Pure logic, easy tests.** `handle_text` and `rules2069::classify` do no I/O, so their tests are plain function calls. Keeping the I/O at the edges and the decisions in pure functions is the pattern for the rest of the project.

**React and an external store.** `Connection` is a plain TypeScript class that owns the WebSocket. React reads it through `useSyncExternalStore`: each change creates a new, `readonly` snapshot object, and React re-renders when the snapshot changes. The map canvas (M1) will read the same kind of store without going through React.

**One origin.** In production the Rust server serves the client and the API from the same address, so cookies and WebSockets need no CORS setup. In development, Vite's proxy (`vite.config.ts`) gives the same effect.

## What M1 teaches

**Three coordinate spaces (`client/src/map/coords.ts`).** Screen pixels (where the pointer is), world pixels (the map image at 100%), and grid squares (the UVTT file). Each has its own *branded* type: at runtime all three are plain `{x, y}`, but TypeScript won't let you pass one where another is expected. The camera (`x`, `y`, `scale`) converts between screen and world; `zoomAt` keeps the point under the pointer fixed while zooming, and `pinch` combines zoom and pan for two fingers. All of it is pure functions, tested in `coords.test.ts`.

**Parsing in Rust (`crates/core/src/uvtt.rs`).** `UvttMap::parse` reads the JSON into typed structs with serde and then checks what types can't express. Look for `#[serde(default)]` (fields some exporters omit), `[GridPoint; 2]` (exactly two door ends, enforced by serde), `?` with `#[from]` (error conversion), and slices (`&[GridPoint]`). The client has a lighter TypeScript copy for now; the server will use the Rust one for uploads in M3.

**Large maps on iPad (`client/src/map/loadUvtt.ts`).** Safari refuses canvases over about 16.7 megapixels, and older iPads limit textures to 4096 px. The client decodes the picture with `createImageBitmap` (no canvas involved), cuts it into tiles of at most 2048 px, and frees the full-size bitmap straight away. The Large sample (5600 × 4200 px) loads as 9 tiles.

**One input path (`client/src/map/panZoom.ts`).** Pointer Events treat mouse, touch and Pencil the same way. The controller tracks the pointers that are down: one pointer pans, two pinch. CSS `touch-action: none` and cancelling Safari's `gesture*` events stop the page itself from zooming. Listeners are removed together with an `AbortController`.

**Keeping React out of the hot path (`client/src/map/MapView.tsx`).** The camera changes every frame during a drag, so it lives in a `useRef`, which doesn't re-render. React only sees the rounded zoom percentage. PixiJS is wrapped in `render/renderer.ts`, so nothing else depends on it.

## What M2 teaches

**A library with a thin `main` (`crates/server/src/lib.rs`).** All the server code is a library; `main.rs` only reads settings and starts listening. That lets `tests/auth.rs` build the same app and send it requests with `oneshot`, with no network port and a fresh in-memory database per test.

**Shared state (`AppState`).** The database pool, settings and rate limiter, reachable from any handler as `State<AppState>`. It's cloned for every request, so the expensive parts sit behind `Arc`, a shared pointer whose clone just bumps a counter.

**Migrations (`crates/server/migrations/`).** The schema is plain SQL, numbered in order. `sqlx::migrate!` builds the files into the binary, and the server applies any new ones at startup.

**Errors as values (`error.rs`).** Every handler returns `Result<_, AppError>`. `AppError` implements `IntoResponse`, so `?` anywhere in a handler turns a failure into the right status code and a JSON `ApiError`. Server-side failures are logged in full, but the user only sees "Something went wrong".

**Extractors (`auth/extract.rs`).** A handler that takes `current: CurrentUser` only runs for signed-in users; the check lives in one `FromRequestParts` impl instead of in every handler. The WebSocket handler takes it too, so `/ws` refuses anyone without a session.

**Blocking work in async code (`auth/password.rs`).** Argon2 is slow on purpose. Running it directly would stall the tokio thread for every other request, so it goes through `spawn_blocking`. The closure must own its data (`move`), because it runs on another thread. A wrong email is checked against a dummy hash, so it takes as long as a wrong password.

**Sessions (`auth/session.rs`).** The cookie holds 256 random bits; the database holds only their SHA-256 hash. `SessionToken`'s `Debug` output is redacted, so a stray `{:?}` can't print it into a log. The cookie is `HttpOnly` (scripts can't read it), `SameSite=Lax`, and `Secure` in production.

**A lock that never crosses an await (`auth/rate_limit.rs`).** A `std::sync::Mutex` guards the attempt counts. That's fine because the lock is held only for a quick map update and never across `.await`.

**Middleware (`origin.rs`).** Runs before every handler and refuses state-changing requests and WebSocket upgrades whose `Origin` isn't this server.

**Shared validation (`crates/core/src/api.rs`).** The request and response types and the rules for emails, passwords and names live in `holotable-core`, and their TypeScript types are generated like the protocol's.

## What M3 teaches

**One gate for every campaign route (`campaigns/access.rs`).** All `/api/campaigns/{id}/...` routes are nested under one router with a `route_layer` middleware, `require_member`. It looks up your role once and attaches a `Member` value to the request; handlers ask for `member: Member` as an argument. A handler mounted outside the gate has no `Member` to get, so the check can't be forgotten. `tests/campaigns.rs` sends every route a request from an outsider and from a signed-out visitor to prove it. Outsiders get 404, not 403, so they can't tell a campaign exists.

**Rules the database enforces.** `CHECK (role IN ('gm', 'player'))`, `UNIQUE`, `REFERENCES … ON DELETE CASCADE`: deleting a campaign removes its members, invites, uploads and scenes in one statement, and a scene's `asset_id` without `ON DELETE` stops a map that's in use from being deleted.

**Check-and-change in one statement (`campaigns/members.rs`).** "Is there another GM? Then demote this one" as two queries could race. As one `UPDATE … WHERE … AND (SELECT COUNT(*) …) > 1`, SQLite does both at once. The SQL is assembled with `concat!` and a small `macro_rules!`, because sqlx 0.9 only accepts SQL that's a compile-time string, never one built with `format!`: that's how SQL injection is ruled out.

**Transactions (`campaigns/mod.rs`, `campaigns/invites.rs`).** Creating a campaign inserts the campaign and the GM's membership; accepting an invite counts a use and adds a membership. `state.db.begin()` … `tx.commit()` makes each pair all-or-nothing; dropping the transaction without committing rolls it back.

**Trusting content, not names (`campaigns/assets.rs`).** An upload's type comes from its first bytes; its file name is only displayed, and files on disk are named by database id. The picture's size is read from its header before decoding, to refuse a "decompression bomb". Decoding and tiling run in `spawn_blocking`; the closure takes the bytes with `move` and hands them back in its result, which is how ownership travels across threads.

**Tiles from the server (`client/src/map/loadScene.ts`).** Large pictures are cut into 2048 px tiles when uploaded, and the client fetches and decodes each tile on its own, so the full picture never has to be decoded in the browser. `holotable_core::campaign::tile_layout` computes the layout for both sides.

**Client routing and shared state (`client/src/App.tsx`).** React Router maps addresses to pages; the server sends `index.html` for unknown paths so deep links work. The signed-in user is shared through a React context (`useCurrentUser`) instead of being passed through every component. `useLoad` wraps "fetch on mount, ignore stale results, reload on demand".

## What M4 teaches

**Rules as pure functions (`crates/core/src/scene.rs`).** Every change to a scene is a `SceneOp` (create, move, update, delete). `plan(op, actor, tokens, rules)` decides whether it's allowed and what event it produces; it never touches the database or the network, so the tests can feed it any situation. Players may only move tokens they own; hidden tokens don't exist for them (asking about one gives "not found", the same as a token that really doesn't exist); a player's move is refused if the straight line from the old centre to the new one crosses a wall or a closed door (`geometry.rs`, an orientation test). The GM can move anything anywhere. `for_player` decides which events each person is sent, so hidden tokens never leave the server.

**One task per scene (`crates/server/src/live/room.rs`).** Each scene with someone on it gets a room: a tokio task that owns the scene's tokens and receives commands through a channel (`mpsc`). Only that task changes the tokens, so there are no locks around them and changes happen in a fixed order. Every accepted change gets the next sequence number (`seq`), is saved to SQLite, and is sent to everyone in the room. The hub (`live/mod.rs`) finds or starts rooms and knows every connection; HTTP handlers call it to close sockets on sign-out and to take people off a scene when they're removed from the campaign or their role changes.

**Catching up after a dropped connection.** The room keeps its last 256 events. A client that reconnects says "I saw up to seq N of epoch E" (`Resume`); if those events are still kept, it gets only what it missed, otherwise a full snapshot. The epoch changes whenever the room is loaded again (after a server restart, say), so old sequence numbers are never mistaken for new ones.

**Splitting a socket (`crates/server/src/ws.rs`).** The WebSocket is split into a reader and a writer. The writer task drains an outbox channel; anyone can send to a client by putting a message in its outbox. The outbox is bounded: a client too slow to keep up is disconnected instead of letting messages pile up in memory. Each connection may send 50 messages a second (`live/rate.rs`); three bad seconds in a row and it's closed.

**Optimistic updates (`client/src/scene/sceneStore.ts`).** My own changes show at once: the store keeps the server's confirmed tokens and lays my unanswered changes on top. `Ack` drops the pending change (the event has already updated the confirmed copy); `Reject` drops it too, which puts the token back and shows the reason. While dragging, the position is sent at most 20 times a second so others see the token move; only the drop is a real, saved change.

**Pointer handling with a hand-off (`client/src/map/panZoom.ts`, `scene/tokenDrag.ts`).** The pan/zoom controller offers each new press to a handler first. If it lands on a token you may move, the token drag takes that pointer; otherwise it pans as before. One code path covers mouse, finger and Pencil. A long press on touch opens the menu, standing in for right-click.

**Tests at three levels.** `scene.rs` and `geometry.rs` unit tests check the rules; `crates/server/tests/live.rs` opens real WebSockets against a running server (two players and a GM, hidden tokens, walls, resume, kicks); the client tests check the store and the drag maths without a browser.

## Next

M5: line of sight and doors. Walls hide what your tokens can't see, computed with the Rust geometry compiled to WebAssembly so the server and the browser agree. Players open and close doors next to their tokens. The real-iPad test from M1 is still open.
