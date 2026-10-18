import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  spriteLayout,
  windowOrigin,
  inspectPng,
  hasTransparentPixel,
  checkSpritePng,
  SPRITE_ERROR,
} from "../../src/shared/sprite";
import {
  encodePng,
  decodePng,
  rgbPng,
  opaqueRgbaPng,
  cutoutRgbaPng,
} from "../fixtures/png.js";

const GREEN = readFileSync(join(__dirname, "../e2e/fixtures/sprite-green.png"));
const card = { width: 480, height: 170 };

describe("spriteLayout", () => {
  it("our sprite: hangs above and left of a 679px card", () => {
    expect(
      spriteLayout({ width: 679, height: 170 }, { x: -41, y: -277, width: 720, height: 430 }),
    ).toEqual({
      window: { width: 720, height: 447 },
      card: { x: 41, y: 277, width: 679, height: 170 },
      sprite: { x: 0, y: 0, width: 720, height: 430 },
    });
  });

  it("sprite fully above the card", () => {
    expect(spriteLayout(card, { x: 100, y: -60, width: 50, height: 40 })).toEqual({
      window: { width: 480, height: 230 },
      card: { x: 0, y: 60, width: 480, height: 170 },
      sprite: { x: 100, y: 0, width: 50, height: 40 },
    });
  });

  it("sprite below the card", () => {
    expect(spriteLayout(card, { x: 10, y: 200, width: 50, height: 40 })).toEqual({
      window: { width: 480, height: 240 },
      card: { x: 0, y: 0, width: 480, height: 170 },
      sprite: { x: 10, y: 200, width: 50, height: 40 },
    });
  });

  it("sprite left of the card", () => {
    expect(spriteLayout(card, { x: -80, y: 20, width: 60, height: 300 })).toEqual({
      window: { width: 560, height: 320 },
      card: { x: 80, y: 0, width: 480, height: 170 },
      sprite: { x: 0, y: 20, width: 60, height: 300 },
    });
  });

  it("sprite right of the card", () => {
    expect(spriteLayout(card, { x: 500, y: -10, width: 60, height: 50 })).toEqual({
      window: { width: 560, height: 180 },
      card: { x: 0, y: 10, width: 480, height: 170 },
      sprite: { x: 500, y: 0, width: 60, height: 50 },
    });
  });

  it("sprite fully inside the card leaves the window at card size", () => {
    expect(spriteLayout(card, { x: 20, y: 30, width: 100, height: 100 })).toEqual({
      window: { width: 480, height: 170 },
      card: { x: 0, y: 0, width: 480, height: 170 },
      sprite: { x: 20, y: 30, width: 100, height: 100 },
    });
  });

  it("sprite overlapping the bottom-right corner", () => {
    expect(spriteLayout(card, { x: 450, y: 150, width: 100, height: 60 })).toEqual({
      window: { width: 550, height: 210 },
      card: { x: 0, y: 0, width: 480, height: 170 },
      sprite: { x: 450, y: 150, width: 100, height: 60 },
    });
  });

  it("sprite overlapping the top-left corner", () => {
    expect(spriteLayout(card, { x: -30, y: -20, width: 100, height: 60 })).toEqual({
      window: { width: 510, height: 190 },
      card: { x: 30, y: 20, width: 480, height: 170 },
      sprite: { x: 0, y: 0, width: 100, height: 60 },
    });
  });
});

describe("windowOrigin", () => {
  const workArea = { x: 0, y: 33, width: 1470, height: 833 };

  it("centres the card and puts its top at 25% of the work area", () => {
    const card = { x: 0, y: 0, width: 480, height: 170 };
    expect(windowOrigin(workArea, card)).toEqual({ x: 495, y: 33 + 208 });
  });

  it("offsets the window so the card, not the window, lands on the spot", () => {
    const card = { x: 41, y: 100, width: 679, height: 170 };
    expect(windowOrigin(workArea, card)).toEqual({ x: 396 - 41, y: 33 + 208 - 100 });
  });

  it("pins the window to the top of the work area when the sprite would stick out above it", () => {
    const card = { x: 41, y: 277, width: 679, height: 170 };
    expect(windowOrigin(workArea, card)).toEqual({ x: 355, y: 33 });
  });

  it("does not pin on a work area tall enough for the sprite", () => {
    const tall = { x: 0, y: 25, width: 2560, height: 1415 };
    const card = { x: 41, y: 277, width: 679, height: 170 };
    expect(windowOrigin(tall, card)).toEqual({ x: 941 - 41, y: 25 + 354 - 277 });
  });
});

describe("inspectPng", () => {
  it("reads size and alpha from the green fixture (RGBA)", () => {
    expect(inspectPng(GREEN)).toEqual({ ok: true, width: 720, height: 430, hasAlpha: true });
  });

  it("an RGB PNG has no alpha channel", () => {
    expect(inspectPng(rgbPng())).toMatchObject({ ok: true, hasAlpha: false });
  });

  it("gray+alpha (colour type 4) has an alpha channel", () => {
    const png = encodePng({ width: 2, height: 1, colorType: 4, pixel: (x: number) => [9, x ? 255 : 0] });
    expect(inspectPng(png)).toMatchObject({ ok: true, hasAlpha: true });
  });

  it("an RGB PNG with a tRNS chunk has an alpha channel", () => {
    const png = encodePng({
      width: 2, height: 1, colorType: 2, pixel: () => [0, 0, 0], trns: [0, 0, 0, 0, 0, 0],
    });
    expect(inspectPng(png)).toMatchObject({ ok: true, hasAlpha: true });
  });

  it("rejects bytes without the PNG signature", () => {
    expect(inspectPng(new TextEncoder().encode("<!doctype html><html></html>"))).toEqual({
      ok: false,
      error: SPRITE_ERROR.notPng,
    });
  });

  it("rejects a truncated PNG (signature only)", () => {
    expect(inspectPng(GREEN.subarray(0, 12))).toEqual({ ok: false, error: SPRITE_ERROR.notPng });
  });
});

describe("hasTransparentPixel", () => {
  it("looks only at the alpha byte of each 4-byte pixel", () => {
    expect(hasTransparentPixel(new Uint8Array([0, 0, 0, 255, 0, 0, 0, 255]))).toBe(false);
    expect(hasTransparentPixel(new Uint8Array([0, 0, 0, 255, 255, 255, 255, 0]))).toBe(true);
    // Semi-transparent is not "background removed".
    expect(hasTransparentPixel(new Uint8Array([0, 0, 0, 1]))).toBe(false);
  });
});

describe("checkSpritePng", () => {
  it("accepts the green fixture", async () => {
    expect(await checkSpritePng(GREEN, decodePng)).toEqual({ ok: true, width: 720, height: 430 });
  });

  it("accepts a synthetic cut-out", async () => {
    expect(await checkSpritePng(cutoutRgbaPng(), decodePng)).toEqual({ ok: true, width: 4, height: 4 });
  });

  it("rejects an RGB PNG without decoding it", async () => {
    let decoded = false;
    const result = await checkSpritePng(rgbPng(), (b: Uint8Array) => ((decoded = true), decodePng(b)));
    expect(result).toEqual({ ok: false, error: SPRITE_ERROR.noAlpha });
    expect(decoded).toBe(false);
  });

  it("rejects an RGBA PNG whose every pixel is opaque", async () => {
    expect(await checkSpritePng(opaqueRgbaPng(), decodePng)).toEqual({
      ok: false,
      error: SPRITE_ERROR.opaque,
    });
  });

  it("rejects a PNG the decoder cannot read", async () => {
    expect(await checkSpritePng(GREEN, () => null)).toEqual({
      ok: false,
      error: SPRITE_ERROR.undecodable,
    });
  });

  it("rejects a non-PNG", async () => {
    expect(await checkSpritePng(new Uint8Array([1, 2, 3]), decodePng)).toEqual({
      ok: false,
      error: SPRITE_ERROR.notPng,
    });
  });
});
