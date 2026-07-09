/**
 * Tray icon (spec §5): a daisy with 12 elliptical petals around a yellow
 * centre. Petals fall one per `interval/12`; petal 0 (12 o'clock) drops
 * first, then clockwise. The icon is the only place in the app with accent
 * colour (warm pink petal tips, yellow centre).
 *
 * Returns a 64×64 RGBA PNG buffer for `nativeImage.createFromBuffer`. The
 * PNG encoder at the bottom is the same minimal one that used to render the
 * old digit icon — no external image deps.
 */
import { deflateSync } from "node:zlib";

const SIZE = 64;
const CENTER = SIZE / 2;
const CENTER_R = 7; // radius of the yellow disc
const PETAL_R_IN = 8; // petal starts just outside the centre
const PETAL_R_OUT = 28; // petal tip stops shy of the canvas edge
const PETAL_HALF_WIDTH = 4.5;
const PETAL_COUNT = 12;

type RGBA = [number, number, number, number];

const CENTER_COLOR: RGBA = [0xff, 0xc8, 0x10, 0xff]; // warm yellow
const PETAL_BASE: RGBA = [0xff, 0xff, 0xff, 0xff]; // white near centre
const PETAL_TIP: RGBA = [0xf5, 0xa8, 0xb8, 0xff]; // warm pink tip
const TRANSPARENT: RGBA = [0, 0, 0, 0];

// Petal 0 sits at 12 o'clock and the sequence runs clockwise. With y growing
// downward (canvas convention), 12 o'clock is angle −π/2.
function petalAngle(index: number): number {
  return -Math.PI / 2 + (index * 2 * Math.PI) / PETAL_COUNT;
}

/**
 * Indices of petals still on the flower for the given progress in [0, 1].
 * 0 = freshly restarted timer (all 12 visible), 1 = timer expired (none).
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
 * Petal k starts falling at t = (k+1) * intervalMs/12 and is fully gone
 * after `fadeMs` of fall. The fade duration is clamped to one slot so a
 * falling petal never overlaps the next petal's departure on short
 * intervals (spec §5: «длительность падения сокращается»).
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

function lerp(a: number, b: number, t: number): number {
  return Math.round(a + (b - a) * t);
}
function lerpRGBA(a: RGBA, b: RGBA, t: number): RGBA {
  return [
    lerp(a[0], b[0], t),
    lerp(a[1], b[1], t),
    lerp(a[2], b[2], t),
    lerp(a[3], b[3], t),
  ];
}

// Returns the RGBA contribution of a single petal at the given pixel offset
// from the centre. Local frame is rotated to the petal's radial direction,
// with the petal modelled as an ellipse. Returns null if the pixel is
// outside the petal.
function petalColorAt(dx: number, dy: number, idx: number): RGBA | null {
  const theta = petalAngle(idx);
  const cos = Math.cos(theta);
  const sin = Math.sin(theta);
  const u = dx * cos + dy * sin;
  const v = -dx * sin + dy * cos;
  if (u < PETAL_R_IN || u > PETAL_R_OUT) return null;
  const cu = (PETAL_R_IN + PETAL_R_OUT) / 2;
  const a = (PETAL_R_OUT - PETAL_R_IN) / 2;
  const norm = ((u - cu) / a) ** 2 + (v / PETAL_HALF_WIDTH) ** 2;
  if (norm > 1) return null;
  const t = (u - PETAL_R_IN) / (PETAL_R_OUT - PETAL_R_IN);
  return lerpRGBA(PETAL_BASE, PETAL_TIP, t);
}

// Spec §5 fall: petal slides outward from the centre, droops slightly under
// "gravity", and fades to transparent over the fade window. These deltas
// stay small — the daisy is only 64×64, so even 4 px of outward motion is
// visually noticeable without crowding the next petal.
const FALL_OUTWARD_PX = 4;
const FALL_GRAVITY_PX = 3;

function pixelColor(x: number, y: number, frame: DaisyFrame): RGBA {
  // 0.5 puts the sample at the pixel centre — symmetric across the 64-px axis.
  const dx = x + 0.5 - CENTER;
  const dy = y + 0.5 - CENTER;
  const d2 = dx * dx + dy * dy;
  if (d2 <= CENTER_R * CENTER_R) return CENTER_COLOR;

  for (const idx of frame.stable) {
    const c = petalColorAt(dx, dy, idx);
    if (c) return c;
  }

  if (frame.falling) {
    const { index, phase } = frame.falling;
    const theta = petalAngle(index);
    // Translate the petal: drift outward along its radial axis + drop a bit
    // under "gravity". To check "does this pixel belong to the translated
    // petal", we subtract the translation from the pixel's offset.
    const outwardX = Math.cos(theta) * FALL_OUTWARD_PX * phase;
    const outwardY = Math.sin(theta) * FALL_OUTWARD_PX * phase;
    const gravityY = FALL_GRAVITY_PX * phase;
    const base = petalColorAt(dx - outwardX, dy - outwardY - gravityY, index);
    if (base) {
      const opacity = 1 - phase;
      return [base[0], base[1], base[2], Math.round(base[3] * opacity)];
    }
  }

  return TRANSPARENT;
}

/**
 * Render a daisy. Accepts either a static progress (0..1) for snapshots, or
 * a full DaisyFrame when the caller wants the in-flight falling petal too.
 */
export function buildTrayIcon(input: number | DaisyFrame): Buffer {
  const frame: DaisyFrame =
    typeof input === "number" ? { stable: visiblePetals(input) } : input;
  const canvas: RGBA[][] = [];
  for (let y = 0; y < SIZE; y++) {
    const row: RGBA[] = [];
    for (let x = 0; x < SIZE; x++) row.push(pixelColor(x, y, frame));
    canvas.push(row);
  }
  return encodePng(canvas);
}

// --- PNG encoder ---

function crc32(buf: Buffer): number {
  const table: number[] = [];
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  let crc = -1;
  for (let i = 0; i < buf.length; i++) crc = (crc >>> 8) ^ table[(crc ^ buf[i]) & 0xff];
  return (crc ^ -1) >>> 0;
}

function chunk(type: string, data: Buffer): Buffer {
  const typeAndData = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(typeAndData), 0);
  return Buffer.concat([len, typeAndData, crc]);
}

function encodePng(canvas: RGBA[][]): Buffer {
  const rows: Buffer[] = [];
  for (let y = 0; y < SIZE; y++) {
    const row = [0]; // filter: None
    for (let x = 0; x < SIZE; x++) row.push(...canvas[y][x]);
    rows.push(Buffer.from(row));
  }
  const raw = Buffer.concat(rows);
  const compressed = deflateSync(raw);

  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(SIZE, 0);
  ihdr.writeUInt32BE(SIZE, 4);
  ihdr[8] = 8; // bit depth
  ihdr[9] = 6; // color type RGBA

  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", compressed),
    chunk("IEND", Buffer.alloc(0)),
  ]);
}
