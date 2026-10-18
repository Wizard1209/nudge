/**
 * Sprite overlay: pure geometry + PNG checks, shared by the main process, the
 * popup renderer and the Settings sub-app. No I/O — callers read the bytes and
 * supply a decoder (nativeImage in Electron, canvas in the browser).
 */

import { CARD_CONTENT_HEIGHT } from "./card";

/**
 * The popup card without a sprite. The height is the whole number of points
 * that holds the card's three rows and two separators; it is the same with a
 * sprite, which changes only the width.
 */
export const CARD_WIDTH = 480;
export const CARD_HEIGHT = Math.ceil(CARD_CONTENT_HEIGHT);

export interface Size {
  width: number;
  height: number;
}

export interface Rect extends Size {
  x: number;
  y: number;
}

/** Window size plus card and sprite rects, all in window coordinates. */
export interface SpriteLayout {
  window: Size;
  card: Rect;
  sprite: Rect;
}

/** What the popup renderer needs to draw the sprite. */
export interface SpriteView {
  url: string;
  layout: SpriteLayout;
}

/**
 * A sprite is drawn at its native pixel density: 1 image pixel = 1 device
 * pixel, never resampled. Its size in points is therefore the PNG's pixel
 * size over the display's scale factor; an odd pixel count on a 2x display
 * ends on a half point, which is still exactly one device pixel.
 */
export function spritePoints(png: Size, scaleFactor: number): Size {
  return { width: png.width / scaleFactor, height: png.height / scaleFactor };
}

/**
 * Lay out the card and a sprite placed at (sprite.x, sprite.y) relative to the
 * card's top-left corner, all in points. The window is the union of both
 * rects rounded up to whole points; everything is shifted so that union
 * starts at (0, 0).
 */
export function spriteLayout(card: Size, sprite: Rect): SpriteLayout {
  const left = Math.min(0, sprite.x);
  const top = Math.min(0, sprite.y);
  const right = Math.max(card.width, sprite.x + sprite.width);
  const bottom = Math.max(card.height, sprite.y + sprite.height);
  return {
    window: { width: Math.ceil(right - left), height: Math.ceil(bottom - top) },
    // `0 - left`, not `-left`: a zero offset must stay 0, not -0.
    card: { x: 0 - left, y: 0 - top, width: card.width, height: card.height },
    sprite: { x: sprite.x - left, y: sprite.y - top, width: sprite.width, height: sprite.height },
  };
}

/**
 * Where the card's top edge goes (spec §1): 25% of the monitor's height down,
 * plus 80 pt.
 */
export const CARD_TOP_FRACTION = 0.25;
export const CARD_TOP_OFFSET = 80;

/** The monitor the popup goes on: its full extent and the part free of menu bar, Dock and taskbar. */
export interface Display {
  bounds: Rect;
  workArea: Rect;
}

/**
 * Window origin that puts the card (given in window coordinates) at its spec
 * spot: horizontally centred on the monitor, top edge at 25% of the monitor's
 * height plus 80 pt. A decoration above the card can push the window top
 * above the work area; the window is then pinned to the work area's top
 * instead, and the card moves down by the overhang. Nothing is pinned at the
 * bottom: a card may hang off there.
 */
// TODO: spec §10 floors the half point and clamps card_width to the work area
// minus 48 pt; this rounds and never clamps (tests/unit/sprite.test.ts pins
// 396 where the spec says 395).
export function windowOrigin({ bounds, workArea }: Display, card: Rect): { x: number; y: number } {
  const x = Math.round(bounds.x + (bounds.width - card.width) / 2) - card.x;
  const y = Math.round(bounds.y + bounds.height * CARD_TOP_FRACTION) + CARD_TOP_OFFSET - card.y;
  return { x, y: Math.max(workArea.y, y) };
}

export const SPRITE_ERROR = {
  noPath: "Choose a PNG file for the sprite",
  notFound: "Sprite file not found",
  notPng: "Sprite is not a PNG file",
  noAlpha: "Sprite PNG has no alpha channel — it needs a transparent background",
  opaque: "Sprite PNG has no transparent pixels — it needs a transparent background",
  undecodable: "Sprite PNG could not be decoded",
} as const;

export type SpriteCheck =
  | { ok: true; width: number; height: number }
  | { ok: false; error: string };

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];

function u32(bytes: Uint8Array, at: number): number {
  return ((bytes[at] << 24) | (bytes[at + 1] << 16) | (bytes[at + 2] << 8) | bytes[at + 3]) >>> 0;
}

function chunkType(bytes: Uint8Array, at: number): string {
  return String.fromCharCode(...bytes.subarray(at + 4, at + 8));
}

/**
 * Signature, size and whether the PNG can carry transparency at all: an alpha
 * channel (IHDR colour type 4 or 6) or a tRNS chunk.
 */
export function inspectPng(
  bytes: Uint8Array,
): ({ ok: true; hasAlpha: boolean } & Size) | { ok: false; error: string } {
  const notPng = { ok: false as const, error: SPRITE_ERROR.notPng };
  if (bytes.length < 33 || PNG_SIGNATURE.some((b, i) => bytes[i] !== b)) return notPng;
  if (u32(bytes, 8) !== 13 || chunkType(bytes, 8) !== "IHDR") return notPng;
  const width = u32(bytes, 16);
  const height = u32(bytes, 20);
  const colorType = bytes[25];
  let hasAlpha = colorType === 4 || colorType === 6;
  for (let at = 33; !hasAlpha && at + 8 <= bytes.length; at += 12 + u32(bytes, at)) {
    const type = chunkType(bytes, at);
    if (type === "tRNS") hasAlpha = true;
    if (type === "IDAT" || type === "IEND") break; // tRNS must precede IDAT
  }
  return { ok: true, width, height, hasAlpha };
}

/** True if any pixel of a 4-bytes-per-pixel bitmap (RGBA or BGRA) has alpha 0. */
export function hasTransparentPixel(pixels: Uint8Array): boolean {
  for (let i = 3; i < pixels.length; i += 4) if (pixels[i] === 0) return true;
  return false;
}

/**
 * "A PNG without a background": a PNG with an alpha channel and at least one
 * fully transparent pixel. `decode` returns a 4-bytes-per-pixel bitmap, or
 * null when the image can't be decoded; it runs only if the header passes.
 */
export async function checkSpritePng(
  bytes: Uint8Array,
  decode: (bytes: Uint8Array) => Uint8Array | null | Promise<Uint8Array | null>,
): Promise<SpriteCheck> {
  const info = inspectPng(bytes);
  if (!info.ok) return info;
  if (!info.hasAlpha) return { ok: false, error: SPRITE_ERROR.noAlpha };
  const pixels = await decode(bytes);
  if (!pixels || pixels.length === 0) return { ok: false, error: SPRITE_ERROR.undecodable };
  if (!hasTransparentPixel(pixels)) return { ok: false, error: SPRITE_ERROR.opaque };
  return { ok: true, width: info.width, height: info.height };
}
