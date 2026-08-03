/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The Node observer server (server/index.mjs) runs on $LOOP_WEB_PORT (default 7717).
// In dev, Vite serves the UI on 5173 and proxies the data endpoints to that server.
const SERVER_PORT = process.env.LOOP_WEB_PORT ?? "7717";
const serverTarget = `http://127.0.0.1:${SERVER_PORT}`;

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      "/events": { target: serverTarget, changeOrigin: true, ws: false },
      "/api": { target: serverTarget, changeOrigin: true },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    // The model layer is pure — no DOM needed. Fast node environment.
    environment: "node",
    include: ["src/model/**/*.test.ts"],
    globals: true,
  },
});
