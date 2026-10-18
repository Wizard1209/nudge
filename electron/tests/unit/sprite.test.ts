import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  spriteLayout,
  spritePoints,
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
const card = { width: 480, height: 130 };

describe("spriteLayout", () => {
  it("our sprite: hangs above and left of a 679px card", () => {
    expect(
      spriteLayout({ width: 679, height: 130 }, { x: -41, y: -277, width: 720, height: 430 }),
    ).toEqual({
      window: { width: 720, height: 430 },
      card: { x: 41, y: 277, width: 679, height: 130 },
      sprite: { x: 0, y: 0, width: 720, height: 430 },
    });
  });

  it("sprite fully above the card", () => {
    expect(spriteLayout(card, { x: 100, y: -60, width: 50, height: 40 })).toEqual({
      window: { width: 480, height: 190 },
      card: { x: 0, y: 60, width: 480, height: 130 },
      sprite: { x: 100, y: 0, width: 50, height: 40 },
    });
  });

  it("sprite below the card", () => {
    expect(spriteLayout(card, { x: 10, y: 200, width: 50, height: 40 })).toEqual({
      window: { width: 480, height: 240 },
      card: { x: 0, y: 0, width: 480, height: 130 },
      sprite: { x: 10, y: 200, width: 50, height: 40 },
    });
  });

  it("sprite left of the card", () => {
    expect(spriteLayout(card, { x: -80, y: 20, width: 60, height: 300 })).toEqual({
      window: { width: 560, height: 320 },
      card: { x: 80, y: 0, width: 480, height: 130 },
      sprite: { x: 0, y: 20, width: 60, height: 300 },
    });
  });

  it("sprite right of the card", () => {
    expect(spriteLayout(card, { x: 500, y: -10, width: 60, height: 50 })).toEqual({
      window: { width: 560, height: 140 },
      card: { x: 0, y: 10, width: 480, height: 130 },
      sprite: { x: 500, y: 0, width: 60, height: 50 },
    });
  });

  it("sprite fully inside the card leaves the window at card size", () => {
    expect(spriteLayout(card, { x: 20, y: 30, width: 100, height: 100 })).toEqual({
      window: { width: 480, height: 130 },
      card: { x: 0, y: 0, width: 480, height: 130 },
      sprite: { x: 20, y: 30, width: 100, height: 100 },
    });
  });

  it("sprite overlapping the bottom-right corner", () => {
    expect(spriteLayout(card, { x: 450, y: 150, width: 100, height: 60 })).toEqual({
      window: { width: 550, height: 210 },
      card: { x: 0, y: 0, width: 480, height: 130 },
      sprite: { x: 450, y: 150, width: 100, height: 60 },
    });
  });

  it("sprite overlapping the top-left corner", () => {
    expect(spriteLayout(card, { x: -30, y: -20, width: 100, height: 60 })).toEqual({
      window: { width: 510, height: 150 },
      card: { x: 30, y: 20, width: 480, height: 130 },
      sprite: { x: 0, y: 0, width: 100, height: 60 },
    });
  });
});

describe("spritePoints", () => {
  it("maps 1 PNG pixel to 1 device pixel: a 720×430 PNG is 360×215 pt on a 2x display", () => {
    expect(spritePoints({ width: 720, height: 430 }, 2)).toEqual({ width: 360, height: 215 });
  });

  it("is the PNG's own size on a 1x display", () => {
    expect(spritePoints({ width: 720, height: 430 }, 1)).toEqual({ width: 720, height: 430 });
  });

  it("keeps the half point of an odd pixel count rather than resampling", () => {
    expect(spritePoints({ width: 721, height: 431 }, 2)).toEqual({ width: 360.5, height: 215.5 });
  });
});

describe("spriteLayout with a sprite measured in half points", () => {
  it("rounds the window up to whole points so the sprite is never clipped", () => {
    expect(spriteLayout(card, { x: 0, y: 130, width: 360.5, height: 215.5 })).toEqual({
      window: { width: 480, height: 346 },
      card: { x: 0, y: 0, width: 480, height: 130 },
      sprite: { x: 0, y: 130, width: 360.5, height: 215.5 },
    });
  });
});

describe("windowOrigin", () => {
  // This host: 1470×956 pt with a 33 pt menu bar. 25% of 956 is 239, plus 80
  // puts the card top at 319.
  const display = {
    bounds: { x: 0, y: 0, width: 1470, height: 956 },
    workArea: { x: 0, y: 33, width: 1470, height: 833 },
  };

  it("centres the card on the monitor and puts its top at 25% of its height + 80", () => {
    const card = { x: 0, y: 0, width: 480, height: 130 };
    expect(windowOrigin(display, card)).toEqual({ x: 495, y: 319 });
  });

  it("anchors on the monitor, not the work area: a Dock on the left and the menu bar move nothing", () => {
    const docked = { ...display, workArea: { x: 70, y: 33, width: 1400, height: 833 } };
    const card = { x: 0, y: 0, width: 480, height: 130 };
    expect(windowOrigin(docked, card)).toEqual({ x: 495, y: 319 });
  });

  it("offsets the window so the card, not the window, lands on the spot", () => {
    const card = { x: 41, y: 100, width: 679, height: 130 };
    expect(windowOrigin(display, card)).toEqual({ x: 396 - 41, y: 319 - 100 });
  });

  it("lets a decoration above the card rise into the menu bar strip as long as it stays on the work area", () => {
    const card = { x: 41, y: 277, width: 679, height: 130 };
    expect(windowOrigin(display, card)).toEqual({ x: 355, y: 319 - 277 });
  });

  it("pins the window to the top of the work area when the decoration would stick out above it", () => {
    const short = {
      bounds: { x: 0, y: 0, width: 1280, height: 720 },
      workArea: { x: 0, y: 25, width: 1280, height: 695 },
    };
    const card = { x: 41, y: 277, width: 679, height: 130 };
    // 180 + 80 − 277 = −17, which is above the work area; the card then
    // sits 277 below the work area's top instead.
    expect(windowOrigin(short, card)).toEqual({ x: 301 - 41, y: 25 });
  });

  it("does not pin on a monitor tall enough for the decoration", () => {
    const tall = {
      bounds: { x: 0, y: 0, width: 2560, height: 1440 },
      workArea: { x: 0, y: 25, width: 2560, height: 1415 },
    };
    const card = { x: 41, y: 277, width: 679, height: 130 };
    expect(windowOrigin(tall, card)).toEqual({ x: 941 - 41, y: 360 + 80 - 277 });
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
