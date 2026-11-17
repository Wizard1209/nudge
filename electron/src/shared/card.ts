import type { Platform } from "./platform";

export const CARD_RADIUS = 16;

export const ROW_LINE_HEIGHT = 18.5;
export const ROW_INSET_Y = 12;
export const ROW_HEIGHT = ROW_LINE_HEIGHT + 2 * ROW_INSET_Y;

export const SEPARATOR_HEIGHT = 1;

export const CARD_CONTENT_HEIGHT = 3 * ROW_HEIGHT + 2 * SEPARATOR_HEIGHT;

export type CardMaterial = "glass" | "acrylic" | "none";

export function cardMaterial(host: {
  electron: boolean;
  platform: Platform;
  sprite: boolean;
  backdrop: boolean;
}): CardMaterial {
  if (!host.electron) return "none";
  if (host.platform === "macos") return host.backdrop ? "glass" : "none";
  return host.sprite ? "none" : "acrylic";
}

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
