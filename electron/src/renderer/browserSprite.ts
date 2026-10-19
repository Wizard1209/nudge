import { checkSpritePng, SPRITE_ERROR } from "../shared/sprite";

/** Decode a PNG to RGBA via the browser's image pipeline; null if it can't. */
async function decodeInCanvas(bytes: Uint8Array): Promise<Uint8Array | null> {
  try {
    const bitmap = await createImageBitmap(new Blob([bytes as BlobPart], { type: "image/png" }), {
      premultiplyAlpha: "none",
      colorSpaceConversion: "none",
    });
    const ctx = new OffscreenCanvas(bitmap.width, bitmap.height).getContext("2d")!;
    ctx.drawImage(bitmap, 0, 0);
    return new Uint8Array(ctx.getImageData(0, 0, bitmap.width, bitmap.height).data.buffer);
  } catch {
    return null;
  }
}

/**
 * The browser sub-app's stand-in for the main process check: `path` is a URL
 * here. Resolves with the error message, or null when the sprite is usable.
 */
export async function checkSpriteUrl(path: string): Promise<string | null> {
  if (path === "") return SPRITE_ERROR.noPath;
  let bytes: Uint8Array;
  try {
    const res = await fetch(path);
    if (!res.ok) return SPRITE_ERROR.notFound;
    bytes = new Uint8Array(await res.arrayBuffer());
  } catch {
    return SPRITE_ERROR.notFound;
  }
  const result = await checkSpritePng(bytes, decodeInCanvas);
  return result.ok ? null : result.error;
}
