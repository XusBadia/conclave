/// <reference types="node" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// https://vitejs.dev/config/
export default defineConfig({
    plugins: [react()],
    // Tauri expects a fixed port, fail if that port is not available.
    clearScreen: false,
    server: {
        port: 1420,
        strictPort: true,
        host: "0.0.0.0",
        hmr: {
            protocol: "ws",
            host: "localhost",
            port: 1421,
        },
        watch: {
            // Don't watch the `src-tauri` directory.
            ignored: ["**/src-tauri/**"],
        },
    },
    build: {
        target: "es2022",
        minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
        sourcemap: !!process.env.TAURI_DEBUG,
    },
});
