/**
 * The frosted material under the popup card on macOS (spec §2): the native
 * addon in native/backdrop puts an NSVisualEffectView, clipped to the card,
 * under the window's content view. Everywhere else, and when the addon is not
 * built, the popup simply has no glass behind it.
 *
 * The same addon is also where macOS is asked which window has the keyboard
 * focus (`frontWindowBounds`, spec §1): one native module, one load.
 */

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

/**
 * Put the glass under `card` (window content coordinates) and show it. `card`
 * is the window row the card sits in, a whole number of points; the glass
 * stops at the card's own 129.5 pt, so no bare frost shows under the bottom
 * hairline.
 */
export function syncBackdrop(win: BrowserWindow, card: Rect): void {
  const backdrop = load();
  if (!backdrop || win.isDestroyed()) return;
  backdrop.attach(win.getNativeWindowHandle(), card.x, card.y, card.width, CARD_CONTENT_HEIGHT, CARD_RADIUS);
}

/** Take the glass away with the popup. */
export function hideBackdrop(): void {
  load()?.detach();
}

/** Whether the glass is under a window: what the card's fill is told. */
export function backdropAttached(): boolean {
  return !!load()?.state();
}

/** Where the glass is, for tests: null when it is not there. */
export function backdropState(): { loaded: boolean } & Partial<BackdropState> {
  const backdrop = load();
  return { loaded: !!backdrop, ...(backdrop?.state() ?? {}) };
}

/**
 * The frontmost application's front window in screen coordinates (its
 * first normal-layer window on screen, front to back), for choosing the
 * popup's display:
 * null when it has none, and off macOS, where the addon is not built.
 */
export function frontWindowBounds(): Rect | null {
  return load()?.frontWindowBounds() ?? null;
}
