import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const backendPort = process.env.LOFT_PORT ?? "7420";
const backend = process.env.LOFT_BACKEND ?? `http://127.0.0.1:${backendPort}`;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: Number(process.env.LOFT_WEB_PORT ?? 7421),
    strictPort: true,
    proxy: {
      // changeOrigin sends the backend its own Host, so the Host check passes here; Vite's own allowedHosts
      // check on the dev server is what stops DNS rebinding in dev.
      "/api": { target: backend, changeOrigin: true },
    },
  },
});
