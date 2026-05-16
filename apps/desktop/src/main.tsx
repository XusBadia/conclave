import React from "react";
import ReactDOM from "react-dom/client";

import { App } from "./App";
import "./i18n";
import { applyTheme, getStoredTheme } from "./lib/theme";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "./index.css";

// Apply stored theme (re-running the same logic as the inline script in
// index.html keeps the React-side state in sync if storage was mutated).
applyTheme(getStoredTheme());

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("missing #root");

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
