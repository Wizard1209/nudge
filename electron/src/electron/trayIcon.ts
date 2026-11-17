import { Resvg } from "@resvg/resvg-js";
import DAISY_SVG from "../../../assets/daisy.svg?raw";

export const PETAL_COUNT = 16;

const VIEWBOX_WIDTH = viewBoxWidth(DAISY_SVG);
export const REFERENCE_SIZE = Math.round(VIEWBOX_WIDTH);

const REF_FALL_OUTWARD = 4;
const REF_FALL_GRAVITY = 3;

export function visiblePetals(progress: number): number[] {
  const clamped = Math.min(1, Math.max(0, progress));
  const fallen = Math.min(PETAL_COUNT, Math.floor(clamped * PETAL_COUNT));
  const out: number[] = [];
  for (let i = fallen; i < PETAL_COUNT; i++) out.push(i);
  return out;
}

export interface FallingPetal {
  index: number;
  phase: number;
}

export interface DaisyFrame {
  stable: number[];
  falling?: FallingPetal;
}

export function daisyFrame(elapsedMs: number, intervalMs: number, fadeMs: number): DaisyFrame {
  if (intervalMs <= 0) {
    return { stable: Array.from({ length: PETAL_COUNT }, (_, i) => i) };
  }
  const t = Math.max(0, elapsedMs);
  const slot = intervalMs / PETAL_COUNT;
  const effectiveFade = Math.min(fadeMs, slot);

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

export function formatTooltip(remainingMs: number): string {
  if (remainingMs <= 0) return "now";
  return `~${Math.ceil(remainingMs / 60_000)} min`;
}

export function petalId(k: number): string {
  return `petal-${k}`;
}

export function editElements(
  svg: string,
  edits: ReadonlyMap<string, ((element: string) => string) | null>,
): string {
  const tag = /<!--[\s\S]*?-->|<!\[CDATA\[[\s\S]*?\]\]>|<[?!][^>]*>|<\/[^>]*>|<(?:[^>"']|"[^"]*"|'[^']*')*>/g;
  let out = "";
  let copied = 0;
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

export function frameSvg(frame: DaisyFrame): string {
  const edits = new Map<string, ((element: string) => string) | null>();
  const stable = new Set(frame.stable);
  for (let k = 0; k < PETAL_COUNT; k++) if (!stable.has(k)) edits.set(petalId(k), null);
  if (frame.falling) {
    const { index, phase } = frame.falling;
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

const ELECTRON_MARK =
  '<ellipse cx="32" cy="32" rx="30" ry="11" transform="rotate(-30 32 32)" fill="none" stroke="#9feaf9" stroke-width="3.2"/>' +
  '<circle cx="58" cy="17" r="4.5" fill="#9feaf9"/>';

export interface TrayIconOptions {
  mark?: boolean;
}

function render(svg: string, size: number): Buffer {
  return new Resvg(svg, {
    fitTo: { mode: "width", value: size },
    font: { loadSystemFonts: false },
  })
    .render()
    .asPng();
}

const cache = new Map<string, Buffer>();

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
