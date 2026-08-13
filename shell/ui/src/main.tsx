import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { apply as applyPlatform } from "./platform";
import { apply as applyAppearance, stored } from "./theme";
import "./fonts.css";
import "./tokens.css";

// Both stamp the root element, and both must happen before the first paint so
// the window never flashes the wrong theme or a mispositioned title bar.
applyPlatform();
applyAppearance(stored());

const root = document.getElementById("root");
if (!root) throw new Error("#root is missing from index.html");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
