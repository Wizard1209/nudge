/**
 * Which display the popup goes on (spec §1, "Which monitor"): the one with
 * the keyboard focus — the display holding the frontmost application's front
 * window, as Spotlight chooses — and with no such window the display under
 * the pointer. Pure: the OS-facing reads (the front window, the pointer, the
 * displays) are the caller's, so the multi-display cases are unit-testable
 * on a one-display machine.
 *
 * Every rect and point is in one top-left-origin coordinate system. In
 * Electron that is screen DIPs; the macOS addon's CG bounds share the origin
 * (the primary display's top left, y down), so no flip is needed.
 */

import type { Rect } from "./sprite";

export interface Point {
  x: number;
  y: number;
}

export interface HasBounds {
  bounds: Rect;
}

/**
 * The display for `front` (the frontmost app's front window, or null when
 * it has none): the one whose bounds contain its centre, else the one it
 * overlaps most; with no front window, or one touching no display, the
 * display under `pointer`; with the pointer off every display, the primary
 * — the one at the origin, or failing that the first listed.
 */
export function displayFor<D extends HasBounds>(front: Rect | null, pointer: Point, displays: D[]): D {
  if (front) {
    const centre = { x: front.x + front.width / 2, y: front.y + front.height / 2 };
    const underCentre = displays.find((d) => contains(d.bounds, centre));
    if (underCentre) return underCentre;
    const overlapping = displays
      .map((d) => ({ d, area: intersection(d.bounds, front) }))
      .filter(({ area }) => area > 0)
      .sort((a, b) => b.area - a.area)[0];
    if (overlapping) return overlapping.d;
  }
  const underPointer = displays.find((d) => contains(d.bounds, pointer));
  if (underPointer) return underPointer;
  return displays.find((d) => contains(d.bounds, { x: 0, y: 0 })) ?? displays[0];
}

/** Half-open on the far edges: a point on the seam belongs to the display that starts there. */
function contains(r: Rect, p: Point): boolean {
  return p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height;
}

function intersection(a: Rect, b: Rect): number {
  const w = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}
