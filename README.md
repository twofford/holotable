# Holotable

A virtual tabletop for **New York 2069**, built from scratch: Rust server and shared core, TypeScript + React client.

Current milestone: **M0, project skeleton.** The Rust server serves the built client, and a WebSocket ping works in both directions.

## Layout

```
holotable/
├── Cargo.toml              Cargo workspace (all Rust crates build together)
├── rust-toolchain.toml     Pins stable Rust + rustfmt, clippy, wasm32 target
├── crates/
│   ├── core/               holotable-core: protocol types (later geometry, UVTT, dice). Native + WASM.
│   ├── rules2069/          New York 2069 rules (check classification so far)
│   └── server/             axum + tokio server; binary is called `holotable`
├── client/                 Vite + React + TypeScript
│   └── src/protocol/generated/   TypeScript types generated from Rust. Do not edit.
└── ci/github-actions.yml  CI workflow (move to .github/workflows/ci.yml, see below)
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

Open http://127.0.0.1:3000. The page should say **open**, show the server version, and log a `ping` from the server every 5 seconds. Press **Ping server** to send one the other way.

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

## CI

The CI workflow is in `ci/github-actions.yml`. GitHub only runs workflows from `.github/workflows/`, so move it there once:

```sh
mkdir -p .github/workflows && mv ci/github-actions.yml .github/workflows/ci.yml && rmdir ci
```

## Checks (the same ones CI runs)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace        # also regenerates client/src/protocol/generated
cargo build -p holotable-core --target wasm32-unknown-unknown
cd client && npm run typecheck && npm run build
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

## Next: M1, map rendering

Parse a `.uvtt` file in `holotable-core` with serde, render the map and grid with PixiJS, and add pan and zoom with Pointer Events. Test it in Safari on an iPad.
