import React from "react";
import ReactDOM from "react-dom/client";

import { App } from "./App";
import "./i18n";
import "./index.css";

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("missing #root");

ReactDOM.createRoot(rootElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
