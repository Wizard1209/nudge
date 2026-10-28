/**
 * Tray icon (spec §5): a daisy with 16 petals around a yellow centre. The
 * drawing is assets/daisy.svg at the repo root; this module knows nothing
 * about petal shapes or colours. It takes the SVG's elements `petal-0` ..
 * `petal-15` out as they fall and rasterises what is left with resvg, the
 * engine the Rust app uses too, so both draw the same pixels from one file.
 *
 * Petals fall one per `interval/16`; petal 0 (12 o'clock) drops first, then
 * clockwise. A falling petal slides outward, sags and fades over the fade
 * window; that is the SVG's own petal element wrapped in a translate and an
 * opacity.
 *
 * Returns an RGBA PNG buffer for `nativeImage` at the pixel size the caller
 * asks for: 18 and 36 px for a macOS menu bar icon at 1x and 2x, the shell's
 * small-icon size at the current DPI on Windows. resvg draws at that size
 * from the vector, so no host resamples a bigger bitmap.
 */
import { Resvg } from "@resvg/resvg-js";
// Inlined into the bundle at build time (Vite `?raw`), so the built app
// carries the drawing in main.js and finds no file at run time.
import DAISY_SVG from "../../../assets/daisy.svg?raw";

/** Spec §5: 16 petals, `petal-0` .. `petal-15` in the SVG. */
export const PETAL_COUNT = 16;

/** The SVG's design width in user units (its viewBox); 64 for the daisy. */
const VIEWBOX_WIDTH = viewBoxWidth(DAISY_SVG);
/** The size `buildTrayIcon` renders at when the caller names none. */
export const REFERENCE_SIZE = Math.round(VIEWBOX_WIDTH);

// Spec §5 fall: the petal slides outward from the centre, droops slightly
// under "gravity", and fades to transparent over the fade window. On a 64
// unit canvas; scaled to the SVG's own viewBox.
const REF_FALL_OUTWARD = 4;
const REF_FALL_GRAVITY = 3;

/**
 * Indices of petals still on the flower for the given progress in [0, 1].
 * 0 = freshly restarted timer (all 16 visible), 1 = timer expired (none).
 * Petals fall in ascending index order; the lowest-indexed surviving petal
 * is the next to go.
 */
export function visiblePetals(progress: number): number[] {
  const clamped = Math.min(1, Math.max(0, progress));
  const fallen = Math.min(PETAL_COUNT, Math.floor(clamped * PETAL_COUNT));
  const out: number[] = [];
  for (let i = fallen; i < PETAL_COUNT; i++) out.push(i);
  return out;
}

export interface FallingPetal {
  index: number;
  /** 0 = just started fading, 1 = fully gone. */
  phase: number;
}

export interface DaisyFrame {
  stable: number[];
  falling?: FallingPetal;
}

/**
 * Daisy rendering instructions at a given moment of the current interval.
 * Petal k starts falling at t = (k+1) * intervalMs/16 and is fully gone
 * after `fadeMs` of fall. The fade duration is clamped to one slot so a
 * falling petal never overlaps the next petal's departure on short
 * intervals (spec §5: "the fall duration is shortened").
 */
export function daisyFrame(elapsedMs: number, intervalMs: number, fadeMs: number): DaisyFrame {
  if (intervalMs <= 0) {
    return { stable: Array.from({ length: PETAL_COUNT }, (_, i) => i) };
  }
  const t = Math.max(0, elapsedMs);
  const slot = intervalMs / PETAL_COUNT;
  const effectiveFade = Math.min(fadeMs, slot);

  // Number of slots fully consumed; the petal currently in motion (if any)
  // is `K - 1`, having started its fall at K * slot.
  const K = Math.min(PETAL_COUNT, Math.floor(t / slot));

  const stable: number[] = [];
  for (let i = K; i < PETAL_COUNT; i++) stable.push(i);

  if (K >= 1) {
    const idx = K - 1;
    const elapsedInFall = t - K * slot;
    if (elapsedInFall < effectiveFade) {
      return {
        stable,
        falling: { index: idx, phase: elapsedInFall / effectiveFade },
      };
    }
  }
  return { stable };
}

/**
 * Tooltip string per spec §5: `~N min`, ceiling to whole minutes, falling
 * to `now` once the deadline is past.
 */
export function formatTooltip(remainingMs: number): string {
  if (remainingMs <= 0) return "now";
  return `~${Math.ceil(remainingMs / 60_000)} min`;
}

/** The id of the k-th petal to fall, as the SVG names it. */
export function petalId(k: number): string {
  return `petal-${k}`;
}

/**
 * Edit the SVG element carrying each id in `edits`, the whole element with
 * its subtree: `null` removes it, a function replaces its text. A tag scan
 * rather than a parser — comments, CDATA, processing instructions and
 * quoted `>` are skipped; an element whose id is not in the map is left
 * alone, and so are the elements nested inside an edited one.
 */
export function editElements(
  svg: string,
  edits: ReadonlyMap<string, ((element: string) => string) | null>,
): string {
  const tag = /<!--[\s\S]*?-->|<!\[CDATA\[[\s\S]*?\]\]>|<[?!][^>]*>|<\/[^>]*>|<(?:[^>"']|"[^"]*"|'[^']*')*>/g;
  let out = "";
  let copied = 0;
  // The edited element being scanned for its end: its start offset, the
  // edit, and how many of its own descendants are still open.
  let open: { start: number; edit: ((element: string) => string) | null; depth: number } | null = null;
  for (let m = tag.exec(svg); m; m = tag.exec(svg)) {
    const text = m[0];
    if (/^<[!?]/.test(text)) continue;
    const closing = text.startsWith("</");
    const selfClosing = !closing && /\/\s*>$/.test(text);
    if (open) {
      if (closing) open.depth--;
      else if (!selfClosing) open.depth++;
      if (open.depth < 0) {
        const end = m.index + text.length;
        out += svg.slice(copied, open.start) + (open.edit ? open.edit(svg.slice(open.start, end)) : "");
        copied = end;
        open = null;
      }
      continue;
    }
    if (closing) continue;
    const id = /\sid\s*=\s*(?:"([^"]*)"|'([^']*)')/.exec(text);
    const key = id ? (id[1] ?? id[2]) : undefined;
    if (key === undefined || !edits.has(key)) continue;
    const edit = edits.get(key) ?? null;
    if (selfClosing) {
      out += svg.slice(copied, m.index) + (edit ? edit(text) : "");
      copied = m.index + text.length;
    } else {
      open = { start: m.index, edit, depth: 0 };
    }
  }
  return out + svg.slice(copied);
}

function viewBoxWidth(svg: string): number {
  const root = /<svg\b[^>]*>/.exec(svg)?.[0] ?? "";
  const vb = /\sviewBox\s*=\s*["']([^"']*)["']/.exec(root)?.[1];
  const width = vb ? Number(vb.trim().split(/[\s,]+/)[2]) : NaN;
  if (!(width > 0)) throw new Error("assets/daisy.svg: the root <svg> needs a viewBox");
  return width;
}

/** The SVG for one frame: fallen petals out, the falling one in flight. */
export function frameSvg(frame: DaisyFrame): string {
  const edits = new Map<string, ((element: string) => string) | null>();
  const stable = new Set(frame.stable);
  for (let k = 0; k < PETAL_COUNT; k++) if (!stable.has(k)) edits.set(petalId(k), null);
  if (frame.falling) {
    const { index, phase } = frame.falling;
    // Petal k's nominal direction, clockwise from 12 o'clock; the SVG's own
    // angle is a degree or two off, which a 4-unit drift does not show.
    const theta = -Math.PI / 2 + (index * 2 * Math.PI) / PETAL_COUNT;
    const k = VIEWBOX_WIDTH / 64;
    const dx = Math.cos(theta) * REF_FALL_OUTWARD * k * phase;
    const dy = (Math.sin(theta) * REF_FALL_OUTWARD + REF_FALL_GRAVITY) * k * phase;
    const f = (n: number) => +n.toFixed(3);
    edits.set(
      petalId(index),
      (el) => `<g transform="translate(${f(dx)} ${f(dy)})" opacity="${f(1 - phase)}">${el}</g>`,
    );
  }
  return editElements(DAISY_SVG, edits);
}

/**
 * The prototype's own mark, not spec §5: one orbit tilted up to the right
 * with its electron at the upper end, in Electron's light cyan. Drawn over
 * the petals so it stays put while they fall. In the daisy's 64-unit space.
 */
const ELECTRON_MARK =
  '<ellipse cx="32" cy="32" rx="30" ry="11" transform="rotate(-30 32 32)" fill="none" stroke="#9feaf9" stroke-width="3.2"/>' +
  '<circle cx="58" cy="17" r="4.5" fill="#9feaf9"/>';

export interface TrayIconOptions {
  /** Draw ELECTRON_MARK over the daisy. */
  mark?: boolean;
}

// The daisy has no text, and resvg otherwise loads every system font on
// each parse: some 140 ms on macOS for an 18 px icon.
function render(svg: string, size: number): Buffer {
  return new Resvg(svg, {
    fitTo: { mode: "width", value: size },
    font: { loadSystemFonts: false },
  })
    .render()
    .asPng();
}

// Frames without a petal in flight, by size and the number still on: the
// tray asks for the same one every tick between falls. A falling petal's
// frames are each drawn once and not kept.
const cache = new Map<string, Buffer>();

/**
 * Render a daisy as a `size`×`size` PNG. Accepts either a static progress
 * (0..1) for snapshots, or a full DaisyFrame when the caller wants the
 * in-flight falling petal too.
 */
export function buildTrayIcon(
  input: number | DaisyFrame,
  size = REFERENCE_SIZE,
  { mark = false }: TrayIconOptions = {},
): Buffer {
  const frame: DaisyFrame =
    typeof input === "number" ? { stable: visiblePetals(input) } : input;
  const svg = () => (mark ? frameSvg(frame).replace(/<\/svg>\s*$/, `${ELECTRON_MARK}</svg>`) : frameSvg(frame));
  if (frame.falling) return render(svg(), size);
  const key = `${size}:${mark ? "mark:" : ""}${frame.stable.join(",")}`;
  let png = cache.get(key);
  if (!png) {
    png = render(svg(), size);
    cache.set(key, png);
  }
  return png;
}
