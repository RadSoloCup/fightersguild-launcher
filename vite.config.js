import { defineConfig } from 'vite'

// Standard Tauri + Vite pairing: fixed dev port Tauri points at, and a
// clean build output dir (kept separate from src-tauri/target so Tauri's
// asset embedding never walks into cargo's own build lock files).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
