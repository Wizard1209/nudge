import path from "node:path";
import { createRequire } from "node:module";
import type { BrowserWindow } from "electron";
import { CARD_CONTENT_HEIGHT, CARD_RADIUS } from "../shared/card";
import type { Rect } from "../shared/sprite";

interface BackdropAddon {
  attach(handle: Buffer, x: number, y: number, width: number, height: number, radius: number): void;
  setFrame(x: number, y: number, width: number, height: number): void;
  detach(): void;
  state(): BackdropState | null;
  frontWindowBounds(): Rect | null;
}

export interface BackdropState {
  frame: Rect;
  hidden: boolean;
  radius: number;
  contentFlipped: boolean;
}

const ADDON = path.join(__dirname, "..", "..", "native", "backdrop", "build", "Release", "backdrop.node");

let addon: BackdropAddon | null | undefined;

function load(): BackdropAddon | null {
  if (addon !== undefined) return addon;
  addon = null;
  if (process.platform !== "darwin") return null;
  try {
    addon = createRequire(__filename)(ADDON) as BackdropAddon;
  } catch (err) {
    console.error("Nudge: no card backdrop (the popup still works):", err);
  }
  return addon;
}

export function syncBackdrop(win: BrowserWindow, card: Rect): void {
  const backdrop = load();
  if (!backdrop || win.isDestroyed()) return;
  backdrop.attach(win.getNativeWindowHandle(), card.x, card.y, card.width, CARD_CONTENT_HEIGHT, CARD_RADIUS);
}

export function hideBackdrop(): void {
  load()?.detach();
}

export function backdropAttached(): boolean {
  return !!load()?.state();
}

export function backdropState(): { loaded: boolean } & Partial<BackdropState> {
  const backdrop = load();
  return { loaded: !!backdrop, ...(backdrop?.state() ?? {}) };
}

export function frontWindowBounds(): Rect | null {
  return load()?.frontWindowBounds() ?? null;
}
