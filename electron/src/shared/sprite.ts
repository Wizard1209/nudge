import { CARD_CONTENT_HEIGHT } from "./card";

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

export interface SpriteLayout {
  window: Size;
  card: Rect;
  sprite: Rect;
}

export interface SpriteView {
  url: string;
  layout: SpriteLayout;
}

export function spritePoints(png: Size, scaleFactor: number): Size {
  return { width: png.width / scaleFactor, height: png.height / scaleFactor };
}

export function spriteLayout(card: Size, sprite: Rect): SpriteLayout {
  const left = Math.min(0, sprite.x);
  const top = Math.min(0, sprite.y);
  const right = Math.max(card.width, sprite.x + sprite.width);
  const bottom = Math.max(card.height, sprite.y + sprite.height);
  return {
    window: { width: Math.ceil(right - left), height: Math.ceil(bottom - top) },
    card: { x: 0 - left, y: 0 - top, width: card.width, height: card.height },
    sprite: { x: sprite.x - left, y: sprite.y - top, width: sprite.width, height: sprite.height },
  };
}

export const CARD_TOP_FRACTION = 0.25;
export const CARD_TOP_OFFSET = 80;

export interface Display {
  bounds: Rect;
  workArea: Rect;
}

export function windowOrigin({ bounds, workArea }: Display, card: Rect): { x: number; y: number } {
  const x = Math.floor(bounds.x + (bounds.width - card.width) / 2) - card.x;
  const y = Math.floor(bounds.y + bounds.height * CARD_TOP_FRACTION) + CARD_TOP_OFFSET - card.y;
  return { x, y: Math.max(workArea.y, y) };
}

export const CARD_SIDE_MARGIN = 24;

export function cardWidthOn(cardWidth: number, workArea: Rect): number {
  return Math.min(cardWidth, workArea.width - 2 * CARD_SIDE_MARGIN);
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
    if (type === "IDAT" || type === "IEND") break;
  }
  return { ok: true, width, height, hasAlpha };
}

export function hasTransparentPixel(pixels: Uint8Array): boolean {
  for (let i = 3; i < pixels.length; i += 4) if (pixels[i] === 0) return true;
  return false;
}

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
