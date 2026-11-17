import type { Rect } from "./sprite";

export interface Point {
  x: number;
  y: number;
}

export interface HasBounds {
  bounds: Rect;
}

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

function contains(r: Rect, p: Point): boolean {
  return p.x >= r.x && p.x < r.x + r.width && p.y >= r.y && p.y < r.y + r.height;
}

function intersection(a: Rect, b: Rect): number {
  const w = Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x);
  const h = Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y);
  return w > 0 && h > 0 ? w * h : 0;
}
