/**
 * Generates build/icon.png — 256x256 static app/installer icon.
 * Tray icon is generated dynamically at runtime (see src/electron/trayIcon.ts).
 */
import { deflateSync } from "node:zlib";
import { writeFileSync, mkdirSync } from "node:fs";
import { resolve } from "node:path";

const SIZE = 256;
const BG = [139, 92, 246, 255];
const DOT = [255, 255, 255, 255];
const TRANSPARENT = [0, 0, 0, 0];

function inRoundedSquare(x, y, size, r) {
  if (x < r && y < r) return (r - x) ** 2 + (r - y) ** 2 <= r * r;
  if (x >= size - r && y < r) return (x - (size - r - 1)) ** 2 + (r - y) ** 2 <= r * r;
  if (x < r && y >= size - r) return (r - x) ** 2 + (y - (size - r - 1)) ** 2 <= r * r;
  if (x >= size - r && y >= size - r)
    return (x - (size - r - 1)) ** 2 + (y - (size - r - 1)) ** 2 <= r * r;
  return true;
}

function inCircle(x, y, cx, cy, r) {
  return (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
}

const rows = [];
for (let y = 0; y < SIZE; y++) {
  const row = [0];
  for (let x = 0; x < SIZE; x++) {
    let pixel = TRANSPARENT;
    if (inRoundedSquare(x, y, SIZE, 56)) {
      pixel = BG;
      if (inCircle(x, y, SIZE / 2 - 0.5, SIZE / 2 - 0.5, 32)) pixel = DOT;
    }
    row.push(...pixel);
  }
  rows.push(Buffer.from(row));
}
const raw = Buffer.concat(rows);
const compressed = deflateSync(raw);

function crc32(buf) {
  let table = [];
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c;
  }
  let crc = -1;
  for (let i = 0; i < buf.length; i++) crc = (crc >>> 8) ^ table[(crc ^ buf[i]) & 0xff];
  return (crc ^ -1) >>> 0;
}

function chunk(type, data) {
  const typeAndData = Buffer.concat([Buffer.from(type, "ascii"), data]);
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length, 0);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(typeAndData), 0);
  return Buffer.concat([len, typeAndData, crc]);
}

const ihdr = Buffer.alloc(13);
ihdr.writeUInt32BE(SIZE, 0);
ihdr.writeUInt32BE(SIZE, 4);
ihdr[8] = 8;
ihdr[9] = 6;

const png = Buffer.concat([
  Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
  chunk("IHDR", ihdr),
  chunk("IDAT", compressed),
  chunk("IEND", Buffer.alloc(0)),
]);

mkdirSync(resolve("build"), { recursive: true });
writeFileSync(resolve("build/icon.png"), png);
console.log(`Wrote ${png.length} bytes to build/icon.png (${SIZE}x${SIZE})`);
