import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import App from "./App";
import { Overlay } from "./overlay/Overlay";
import "./styles.css";

function currentWindowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    return "settings";
  }
}

const isOverlay = currentWindowLabel() === "overlay";

// Set before render so the overlay window never paints the paper background.
if (isOverlay) {
  document.documentElement.dataset.window = "overlay";
  document.body.dataset.window = "overlay";
}

const container = document.getElementById("root");

if (container) {
  createRoot(container).render(
    <StrictMode>{isOverlay ? <Overlay /> : <App />}</StrictMode>,
  );
}
