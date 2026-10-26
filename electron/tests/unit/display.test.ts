import { describe, it, expect } from "vitest";

import { displayFor } from "../../src/shared/display";

// Spec §1, "Which monitor": the popup goes on the display with the keyboard
// focus — the one holding the frontmost application's front window — and
// with no such window on the display under the pointer. All rects share one
// top-left-origin coordinate system (Electron's screen DIPs; CG points on
// macOS, which agree with them without a flip).
const primary = { id: "primary", bounds: { x: 0, y: 0, width: 1470, height: 956 } };
const right = { id: "right", bounds: { x: 1470, y: -200, width: 2560, height: 1440 } };
const displays = [primary, right];
const pointerOnPrimary = { x: 100, y: 100 };

describe("displayFor", () => {
  it("picks the display holding the front window", () => {
    const front = { x: 1800, y: 100, width: 800, height: 600 };
    expect(displayFor(front, pointerOnPrimary, displays)).toBe(right);
  });

  it("a window straddling two displays goes where its centre is", () => {
    // Centre at x = 1500: just over the seam, on the right display.
    const front = { x: 1100, y: 100, width: 800, height: 600 };
    expect(displayFor(front, pointerOnPrimary, displays)).toBe(right);
  });

  it("a window whose centre is on no display goes where most of it is", () => {
    // Centre at y = -250: above both displays; 300 pt of its height overlap
    // the right display and none the primary.
    const front = { x: 1600, y: -500, width: 400, height: 500 };
    expect(displayFor(front, pointerOnPrimary, displays)).toBe(right);
  });

  it("with no front window it is the pointer's display", () => {
    expect(displayFor(null, { x: 2000, y: 300 }, displays)).toBe(right);
    expect(displayFor(null, pointerOnPrimary, displays)).toBe(primary);
  });

  it("a front window touching no display counts as none", () => {
    const front = { x: 5000, y: 5000, width: 100, height: 100 };
    expect(displayFor(front, { x: 2000, y: 300 }, displays)).toBe(right);
  });

  it("with the pointer off every display it is the primary", () => {
    // Above the primary and left of the right display: nobody's ground.
    expect(displayFor(null, { x: 100, y: -100 }, [right, primary])).toBe(primary);
  });

  it("reads bounds as half-open, so the seam belongs to the right display", () => {
    expect(displayFor(null, { x: 1470, y: 100 }, displays)).toBe(right);
    expect(displayFor(null, { x: 1469, y: 100 }, displays)).toBe(primary);
  });
});
