import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development, Vite serves the client on :5173 and forwards API and
// WebSocket traffic to the Rust server on :3000. In production the Rust
// server serves the built files itself, so everything shares one origin.
export default defineConfig({
  plugins: [react()],
  server: {
    // The sample maps live in ../fixtures, outside the client folder. Vite's
    // dev server only serves files it's allowed to; let it read the repo root.
    fs: { allow: [".."] },
    // `changeOrigin: false` keeps the browser's Host header (localhost:5173)
    // when forwarding. The server refuses requests whose Origin doesn't match
    // Host (see crates/server/src/origin.rs), so the two must agree. Vite's
    // short form, `"/api": "http://..."`, sets changeOrigin to true and would
    // break sign-in, so both rules are written out in full.
    proxy: {
      "/api": { target: "http://127.0.0.1:3000", changeOrigin: false },
      "/ws": { target: "ws://127.0.0.1:3000", ws: true, changeOrigin: false },
    },
  },
});
