import { defineConfig } from "vitest/config";

// Deliberately NOT merged with vite.config.ts: the app config carries
// dev-server/Tauri specifics (fixed port, HMR socket, src-tauri watch
// ignore) that are irrelevant — and mildly harmful — under a test
// runner. Pure-logic tests don't need the React plugin either; add it
// here the day component tests land.
export default defineConfig({
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test-setup.ts"],
  },
});
