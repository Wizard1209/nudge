import { describe, it, expect } from "vitest";

import {
  buildTrayIcon,
  editElements,
  PETAL_COUNT,
  visiblePetals,
  daisyFrame,
  formatTooltip,
} from "../../src/electron/trayIcon";

// Spec §5: daisy icon — 16 petals around a yellow centre, drawn from
// assets/daisy.svg. Petals fall one per `interval/16`, top petal (12 o'clock, index 0) first, then clockwise. At
// timer expiry only the centre remains. Tooltip is "~N min" (ceil to whole
// minutes) and flips to "now" when the timer is at zero.

describe("trayIcon / visiblePetals", () => {
  it("returns all 16 petals at progress 0", () => {
    const petals = visiblePetals(0);
    expect(petals).toHaveLength(PETAL_COUNT);
    expect(petals).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
  });

  it("returns no petals at progress 1", () => {
    expect(visiblePetals(1)).toEqual([]);
  });

  it("drops the 12-o'clock petal (index 0) first", () => {
    // Just past 1/16 of the interval: petal 0 has fallen, 1..15 remain.
    const petals = visiblePetals(1 / 16 + 0.001);
    expect(petals).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
  });

  it("drops petals clockwise (index ascends)", () => {
    // Halfway: petals 0..7 fallen, 8..15 remain.
    expect(visiblePetals(0.5)).toEqual([8, 9, 10, 11, 12, 13, 14, 15]);
  });

  it("keeps the last petal until progress fully hits 1", () => {
    // 15/16 < progress < 1: 15 fallen, petal 15 still there.
    expect(visiblePetals(15 / 16 + 0.001)).toEqual([15]);
  });

  it("clamps progress to [0, 1]", () => {
    expect(visiblePetals(-0.5)).toHaveLength(16);
    expect(visiblePetals(1.5)).toEqual([]);
  });
});

describe("trayIcon / the SVG", () => {
  it("names the 16 petals and the disc by id", () => {
    for (let k = 0; k < PETAL_COUNT; k++) expect(SVG).toContain(`id="petal-${k}"`);
    expect(SVG).toContain('id="disc"');
    expect(petalAngles()).toHaveLength(PETAL_COUNT);
  });
});

describe("trayIcon / editElements", () => {
  const doc = [
    "<svg viewBox='0 0 64 64'>",
    "<!-- <g id=\"a\"> in a comment is not an element -->",
    "<g id=\"a\" data-note='x > y'><g id=\"inner\"><path/></g><path/></g>",
    "<g data-id=\"a\"><circle id='b'/></g>",
    "</svg>",
  ].join("");

  it("removes the whole element with its subtree, nested groups included", () => {
    const out = editElements(doc, new Map([["a", null]]));
    expect(out).not.toContain('id="inner"');
    expect(out).toContain("<!-- <g id=\"a\">");
    expect(out).toContain("<g data-id=\"a\"><circle id='b'/></g>");
  });

  it("removes a self-closing element, single-quoted id", () => {
    expect(editElements(doc, new Map([["b", null]]))).toContain('<g data-id="a"></g>');
  });

  it("wraps an element when the edit is a function", () => {
    const out = editElements(doc, new Map([["inner", (el: string) => `<g opacity="0.5">${el}</g>`]]));
    expect(out).toContain('<g opacity="0.5"><g id="inner"><path/></g></g><path/>');
  });

  it("leaves the document alone when no id matches", () => {
    expect(editElements(doc, new Map([["zzz", null]]))).toBe(doc);
  });
});

describe("trayIcon / buildTrayIcon", () => {
  it("renders the full daisy at 36 px: opaque body, yellow centre, pink near the top tip", () => {
    const px = pixels(buildTrayIcon(0, 36), 36);
    expect(px.count((p) => p[3] > 0)).toBeGreaterThan(36 * 36 * 0.3);
    const [r, g, b, a] = px.at(18, 18);
    expect([a, r > 230, g > 170, b < 120]).toEqual([255, true, true, true]);
    // Near the tip of the top petal (petal-0, about 12 o'clock): the
    // warmest pixel there is pink, not the white of the petal's body.
    let pinkest = 0;
    for (let y = 1; y <= 4; y++)
      for (let x = 16; x <= 20; x++) {
        const [pr, pg, , pa] = px.at(x, y);
        if (pa > 128) pinkest = Math.max(pinkest, pr - pg);
      }
    expect(pinkest).toBeGreaterThan(30);
  });

  it.each([16, 18, 20, 36])("renders a %i px square", (size) => {
    const png = buildTrayIcon(0, size);
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([size, size]);
  });

  // The petals on the flower are exactly the frame's stable ones: each is
  // found by sampling its own axis, at 3/4 of the tip radius, at the angle
  // the SVG's transform gives it.
  it.each(Array.from({ length: PETAL_COUNT + 1 }, (_, k) => k))(
    "with %i petals fallen, the rest are on the flower and only they",
    (fallen) => {
      const stable = Array.from({ length: PETAL_COUNT - fallen }, (_, i) => fallen + i);
      expect(petalsOnFlower(buildTrayIcon({ stable }, 36), 36)).toEqual(stable);
    },
  );

  it("keeps the centre at every progress", () => {
    for (const p of [0, 0.5, 1]) {
      const [r, g, b, a] = pixels(buildTrayIcon(p, 36), 36).at(18, 18);
      expect([a, r > 230, g > 170, b < 120], `progress=${p}`).toEqual([255, true, true, true]);
    }
  });

  it("draws the falling petal half faded mid-fall", () => {
    const size = 36;
    const falling = buildTrayIcon({ stable: [], falling: { index: 0, phase: 0.5 } }, size);
    const px = pixels(falling, size);
    let top = 0;
    for (let y = 0; y < 9; y++) for (let x = 14; x <= 22; x++) top = Math.max(top, px.at(x, y)[3]);
    expect(top).toBeGreaterThan(90);
    expect(top).toBeLessThan(170);
  });

  it("returns the same buffer for a frame it has drawn before", () => {
    expect(buildTrayIcon({ stable: [3, 4, 5] }, 18)).toBe(buildTrayIcon({ stable: [3, 4, 5] }, 18));
  });
});

describe("trayIcon / daisyFrame", () => {
  // §5 fall animation: petal k starts falling at t = (k+1) * interval/16
  // and is fully gone after `fadeMs`. While falling, it counts as the "in
  // motion" petal (still drawn, but offset + fading), not part of `stable`.
  const interval = 16_000; // 16s for easy mental math (slot = 1s)
  const fadeMs = 250;

  it("returns all 16 stable petals at t=0, none falling", () => {
    const f = daisyFrame(0, interval, fadeMs);
    expect(f.stable).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    expect(f.falling).toBeUndefined();
  });

  it("marks petal 0 as falling right when its slot ends", () => {
    const f = daisyFrame(1000, interval, fadeMs); // exactly at slot boundary
    expect(f.stable).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
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
    expect(f.stable).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]);
    expect(f.falling).toBeUndefined();
  });

  it("renders the final petal falling at the very end of the interval", () => {
    const f = daisyFrame(16_000, interval, fadeMs);
    expect(f.stable).toEqual([]);
    expect(f.falling?.index).toBe(15);
    expect(f.falling?.phase).toBeCloseTo(0, 5);
  });

  it("ends with no petals once t exceeds interval + fade", () => {
    const f = daisyFrame(interval + fadeMs + 1, interval, fadeMs);
    expect(f.stable).toEqual([]);
    expect(f.falling).toBeUndefined();
  });

  it("clamps the fade duration to the slot length on short intervals", () => {
    // For a 1.6s interval, slot = 100ms — shorter than the default 250ms
    // fade. The animation must compress so that one falling petal never
    // overlaps the next slot.
    const short = 1600;
    const slot = short / 16;
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

import { readFileSync } from "node:fs";
import { decodePng } from "../fixtures/png.js";

const SVG = readFileSync(new URL("../../../assets/daisy.svg", import.meta.url), "utf8");

// Each petal's angle, degrees clockwise from 12 o'clock, from its group's
// `rotate(a cx cy)`.
function petalAngles(): number[] {
  const out: number[] = [];
  for (let k = 0; k < PETAL_COUNT; k++) {
    const tag = new RegExp(`<g[^>]*\\sid="petal-${k}"[^>]*>`).exec(SVG)?.[0] ?? "";
    out.push(Number(/rotate\(\s*([-\d.]+)/.exec(tag)?.[1]));
  }
  return out;
}

function pixels(png: Buffer, size: number) {
  const rgba = decodePng(png);
  const at = (x: number, y: number) => Array.from(rgba.subarray((y * size + x) * 4, (y * size + x) * 4 + 4));
  const count = (pred: (p: number[]) => boolean) => {
    let n = 0;
    for (let i = 0; i < size * size; i++) if (pred(at(i % size, Math.floor(i / size)))) n++;
    return n;
  };
  return { at, count };
}

/** Petal tip radius on the 64-unit canvas (the table's `petal_tip`). */
const TIP_RADIUS = 31;

function petalsOnFlower(png: Buffer, size: number): number[] {
  const px = pixels(png, size);
  const r = 0.75 * TIP_RADIUS * (size / 64);
  const c = size / 2;
  return petalAngles().flatMap((deg, k) => {
    const a = (deg * Math.PI) / 180;
    const [, , , alpha] = px.at(Math.floor(c + r * Math.sin(a)), Math.floor(c - r * Math.cos(a)));
    return alpha > 128 ? [k] : [];
  });
}
