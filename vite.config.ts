import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1431,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
});
