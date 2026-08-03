import React from "react";
import { createRoot } from "react-dom/client";
import "./theme/tokens.css";
import "./app.css";
import "./graph/graph.css";
import { App } from "./App.tsx";

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
