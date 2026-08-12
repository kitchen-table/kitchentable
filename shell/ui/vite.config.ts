import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The shell loads this over http during `tauri dev` and from dist/ when bundled.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  test: {
    globals: true,
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
  },
});
