/**
 * The popup card's metrics (spec §2/§3), shared by the renderer, which draws
 * them, and the main process, which sizes the window to them and clips the
 * macOS backdrop to the card.
 */

import type { Platform } from "./platform";

/** Corner radius in points, shared by the fill, the contour and the backdrop. */
export const CARD_RADIUS = 16;

/** 16px type, inset 12px above and below: a 42.5 row. */
export const ROW_LINE_HEIGHT = 18.5;
export const ROW_INSET_Y = 12;
export const ROW_HEIGHT = ROW_LINE_HEIGHT + 2 * ROW_INSET_Y;

/** The line between two rows. */
export const SEPARATOR_HEIGHT = 1;

/** Three rows and the two lines between them. */
export const CARD_CONTENT_HEIGHT = 3 * ROW_HEIGHT + 2 * SEPARATOR_HEIGHT;

/**
 * What the OS puts behind the card: the frosted glass of a masked
 * NSVisualEffectView on macOS, the DWM's Acrylic on a Windows window that is
 * not transparent, or nothing.
 */
export type CardMaterial = "glass" | "acrylic" | "none";

export function cardMaterial(host: {
  electron: boolean;
  platform: Platform;
  sprite: boolean;
  /** Whether the main process has the frosted view under this card. */
  backdrop: boolean;
}): CardMaterial {
  if (!host.electron) return "none";
  // The glass is a native addon the build can go without; a card that
  // assumed it would be a half-opaque flat fill.
  if (host.platform === "macos") return host.backdrop ? "glass" : "none";
  // Acrylic only composites on a window that is not transparent, and a
  // sprite needs a transparent window around the card.
  return host.sprite ? "none" : "acrylic";
}

/**
 * Alpha of the (24,24,27) fill. The card reads as 95% opaque in every case;
 * what the fill has to contribute depends on the material behind it. Over the
 * glass the desktop reaching the card is already blurred and the fill only
 * tints it. Over Acrylic 0.8 is the Windows way to the same 95%. With nothing
 * behind it the fill is the surface.
 */
export function cardFillAlpha(material: CardMaterial): number {
  switch (material) {
    case "glass":
      return 120 / 255;
    case "acrylic":
      return 0.8;
    case "none":
      return 0.95;
  }
}
