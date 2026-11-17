import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// In development, Vite serves the client on :5173 and forwards API and
// WebSocket traffic to the Rust server on :3000. In production the Rust
// server serves the built files itself, so everything shares one origin.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/api": "http://127.0.0.1:3000",
      "/ws": { target: "ws://127.0.0.1:3000", ws: true },
    },
  },
});
