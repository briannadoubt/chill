import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "/",
  publicDir: false,
  plugins: [react()],
  build: {
    outDir: "dist/client",
    target: "es2022",
    sourcemap: true,
  },
  server: {
    host: "127.0.0.1",
    port: 4173,
  },
});
