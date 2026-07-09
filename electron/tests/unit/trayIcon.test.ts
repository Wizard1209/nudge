import { describe, it, expect } from "vitest";

import {
  buildTrayIcon,
  visiblePetals,
  daisyFrame,
  formatTooltip,
} from "../../src/electron/trayIcon";

// Spec §5: daisy icon — 12 petals around a yellow centre. Petals fall one per
// `interval/12`, top petal (12 o'clock, index 0) first, then clockwise. At
// timer expiry only the centre remains. Tooltip is "~N min" (ceil to whole
// minutes) and flips to "now" when the timer is at zero.

describe("trayIcon / visiblePetals", () => {
  it("returns all 12 petals at progress 0", () => {
    const petals = visiblePetals(0);
    expect(petals).toHaveLength(12);
    expect(petals).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
  });

  it("returns no petals at progress 1", () => {
    expect(visiblePetals(1)).toEqual([]);
  });

  it("drops the 12-o'clock petal (index 0) first", () => {
    // Just past 1/12 of the interval: petal 0 has fallen, 1..11 remain.
    const petals = visiblePetals(1 / 12 + 0.001);
    expect(petals).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
  });

  it("drops petals clockwise (index ascends)", () => {
    // Halfway: petals 0..5 fallen, 6..11 remain.
    expect(visiblePetals(0.5)).toEqual([6, 7, 8, 9, 10, 11]);
  });

  it("keeps the last petal until progress fully hits 1", () => {
    // 11/12 < progress < 1: 11 fallen, petal 11 still there.
    expect(visiblePetals(11 / 12 + 0.001)).toEqual([11]);
  });

  it("clamps progress to [0, 1]", () => {
    expect(visiblePetals(-0.5)).toHaveLength(12);
    expect(visiblePetals(1.5)).toEqual([]);
  });
});

describe("trayIcon / buildTrayIcon", () => {
  // The tray icon is a 64x64 RGBA PNG. We verify the shape (PNG magic,
  // dimensions) and the contract that progress controls petal visibility.
  // Pixel-perfect rendering is intentionally not asserted — that's a visual
  // concern best caught by eye, not pixel diffs.

  const PNG_MAGIC = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

  it("returns a valid PNG buffer", () => {
    const buf = buildTrayIcon(0);
    expect(buf.subarray(0, 8).equals(PNG_MAGIC)).toBe(true);
  });

  it("has at least one yellow center pixel at every progress (centre never falls)", () => {
    for (const p of [0, 0.25, 0.5, 0.75, 1]) {
      const buf = buildTrayIcon(p);
      expect(hasYellow(buf), `progress=${p}`).toBe(true);
    }
  });

  it("renders petals at progress 0 and no petals at progress 1", () => {
    expect(hasWhiteOrPink(buildTrayIcon(0))).toBe(true);
    expect(hasWhiteOrPink(buildTrayIcon(1))).toBe(false);
  });

  it("petal area at progress 0 is roughly 12x larger than at progress 11/12", () => {
    // Sanity check that visiblePetals actually drives the rendered output —
    // fewer indices → fewer petal pixels.
    const full = countPetalPixels(buildTrayIcon(0));
    const last = countPetalPixels(buildTrayIcon(11 / 12 + 0.001));
    expect(full).toBeGreaterThan(0);
    expect(last).toBeGreaterThan(0);
    // Allow generous slack; we only care about the order of magnitude.
    expect(full).toBeGreaterThan(last * 6);
    expect(full).toBeLessThan(last * 20);
  });
});

describe("trayIcon / daisyFrame", () => {
  // §5 fall animation: petal k starts falling at t = (k+1) * interval/12
  // and is fully gone after `fadeMs`. While falling, it counts as the "in
  // motion" petal (still drawn, but offset + fading), not part of `stable`.
  const interval = 12_000; // 12s for easy mental math (slot = 1s)
  const fadeMs = 250;

  it("returns all 12 stable petals at t=0, none falling", () => {
    const f = daisyFrame(0, interval, fadeMs);
    expect(f.stable).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    expect(f.falling).toBeUndefined();
  });

  it("marks petal 0 as falling right when its slot ends", () => {
    const f = daisyFrame(1000, interval, fadeMs); // exactly at slot boundary
    expect(f.stable).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    expect(f.falling?.index).toBe(0);
    expect(f.falling?.phase).toBeCloseTo(0, 5);
  });

  it("advances the falling phase linearly through the fade window", () => {
    const f = daisyFrame(1000 + fadeMs / 2, interval, fadeMs);
    expect(f.falling?.index).toBe(0);
    expect(f.falling?.phase).toBeCloseTo(0.5, 5);
  });

  it("drops the falling petal once the fade window ends", () => {
    const f = daisyFrame(1000 + fadeMs, interval, fadeMs);
    expect(f.stable).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);
    expect(f.falling).toBeUndefined();
  });

  it("renders the final petal falling at the very end of the interval", () => {
    const f = daisyFrame(12_000, interval, fadeMs);
    expect(f.stable).toEqual([]);
    expect(f.falling?.index).toBe(11);
    expect(f.falling?.phase).toBeCloseTo(0, 5);
  });

  it("ends with no petals once t exceeds interval + fade", () => {
    const f = daisyFrame(interval + fadeMs + 1, interval, fadeMs);
    expect(f.stable).toEqual([]);
    expect(f.falling).toBeUndefined();
  });

  it("clamps the fade duration to the slot length on short intervals", () => {
    // For a 1.2s interval, slot = 100ms — shorter than the default 250ms
    // fade. The animation must compress so that one falling petal never
    // overlaps the next slot.
    const short = 1200;
    const slot = short / 12;
    // 60ms into the first slot's fade, with slot=100 the effective fade is
    // 100ms, so phase ≈ 0.6 (not 0.24 you'd get from a literal 250ms fade).
    const f = daisyFrame(slot + 60, short, fadeMs);
    expect(f.falling?.index).toBe(0);
    expect(f.falling?.phase).toBeCloseTo(0.6, 1);
  });
});

describe("trayIcon / formatTooltip", () => {
  it("formats remaining time as '~N min', rounded up", () => {
    expect(formatTooltip(60_000)).toBe("~1 min");
    expect(formatTooltip(61_000)).toBe("~2 min");
    expect(formatTooltip(10 * 60_000)).toBe("~10 min");
  });

  it("uses 'now' once the timer is at or past zero", () => {
    expect(formatTooltip(0)).toBe("now");
    expect(formatTooltip(-1)).toBe("now");
  });
});

// --- Helpers ------------------------------------------------------------

// Decode raw pixels by re-using zlib + the IDAT chunk. The encoder uses no
// filter (filter byte = 0 per row), so each row is N bytes of RGBA after a
// leading filter byte. Good enough for spot-check assertions.
import { inflateSync } from "node:zlib";

function decodeRawRGBA(png: Buffer): { width: number; height: number; pixels: Uint8Array } {
  // Skip 8-byte signature
  let i = 8;
  let width = 0;
  let height = 0;
  const idatChunks: Buffer[] = [];
  while (i < png.length) {
    const len = png.readUInt32BE(i);
    const type = png.slice(i + 4, i + 8).toString("ascii");
    const data = png.slice(i + 8, i + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
    } else if (type === "IDAT") {
      idatChunks.push(data);
    }
    i += 8 + len + 4; // length + type + data + crc
  }
  const raw = inflateSync(Buffer.concat(idatChunks));
  // strip the filter byte at the start of each row
  const stride = width * 4;
  const pixels = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    const src = 1 + y * (stride + 1);
    pixels.set(raw.subarray(src, src + stride), y * stride);
  }
  return { width, height, pixels };
}

function hasYellow(png: Buffer): boolean {
  const { pixels } = decodeRawRGBA(png);
  for (let i = 0; i < pixels.length; i += 4) {
    const [r, g, b, a] = [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]];
    // Warm yellow: R high, G high, B low, opaque.
    if (a > 200 && r > 200 && g > 150 && b < 80) return true;
  }
  return false;
}

function hasWhiteOrPink(png: Buffer): boolean {
  const { pixels } = decodeRawRGBA(png);
  for (let i = 0; i < pixels.length; i += 4) {
    const [r, g, b, a] = [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]];
    // Exclude yellow centre (R/G high, B very low). Petals are white→pink
    // (R high, G & B both substantial, B not near zero).
    if (a > 200 && r > 200 && g > 150 && b > 130) return true;
  }
  return false;
}

function countPetalPixels(png: Buffer): number {
  const { pixels } = decodeRawRGBA(png);
  let n = 0;
  for (let i = 0; i < pixels.length; i += 4) {
    const [r, g, b, a] = [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]];
    if (a > 200 && r > 200 && g > 150 && b > 130) n++;
  }
  return n;
}
