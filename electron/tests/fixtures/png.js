import { deflateSync, inflateSync, crc32 } from 'node:zlib';

const SIGNATURE = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
const CHANNELS = { 0: 1, 2: 3, 4: 2, 6: 4 };

function chunk(type, data) {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(crc32(body));
  return Buffer.concat([len, body, crc]);
}

export function encodePng({ width, height, colorType, pixel, trns }) {
  const ch = CHANNELS[colorType];
  const raw = Buffer.alloc((width * ch + 1) * height);
  for (let y = 0; y < height; y++) {
    const row = y * (width * ch + 1);
    raw[row] = 0;
    for (let x = 0; x < width; x++) {
      pixel(x, y).forEach((v, i) => (raw[row + 1 + x * ch + i] = v));
    }
  }
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(width, 0);
  ihdr.writeUInt32BE(height, 4);
  ihdr.set([8, colorType, 0, 0, 0], 8);
  return Buffer.concat([
    SIGNATURE,
    chunk('IHDR', ihdr),
    ...(trns ? [chunk('tRNS', Buffer.from(trns))] : []),
    chunk('IDAT', deflateSync(raw)),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

export function decodePng(bytes) {
  const buf = Buffer.from(bytes);
  let pos = 8;
  let width, height, colorType, trns;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('ascii', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      colorType = data[9];
    } else if (type === 'tRNS') trns = data;
    else if (type === 'IDAT') idat.push(data);
    pos += 12 + len;
  }
  const ch = CHANNELS[colorType];
  const stride = width * ch;
  const raw = inflateSync(Buffer.concat(idat));
  const px = Buffer.alloc(stride * height);
  for (let y = 0; y < height; y++) {
    const filter = raw[y * (stride + 1)];
    for (let i = 0; i < stride; i++) {
      const cur = raw[y * (stride + 1) + 1 + i];
      const a = i >= ch ? px[y * stride + i - ch] : 0;
      const b = y > 0 ? px[(y - 1) * stride + i] : 0;
      const c = i >= ch && y > 0 ? px[(y - 1) * stride + i - ch] : 0;
      let pred = 0;
      if (filter === 1) pred = a;
      else if (filter === 2) pred = b;
      else if (filter === 3) pred = (a + b) >> 1;
      else if (filter === 4) {
        const p = a + b - c;
        const [pa, pb, pc] = [Math.abs(p - a), Math.abs(p - b), Math.abs(p - c)];
        pred = pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
      }
      px[y * stride + i] = (cur + pred) & 0xff;
    }
  }
  const out = new Uint8Array(width * height * 4);
  for (let p = 0; p < width * height; p++) {
    const s = px.subarray(p * ch, p * ch + ch);
    const rgb = ch < 3 ? [s[0], s[0], s[0]] : [s[0], s[1], s[2]];
    let alpha = ch === 2 || ch === 4 ? s[ch - 1] : 255;
    if (trns && colorType === 0 && trns.readUInt16BE(0) === s[0]) alpha = 0;
    if (trns && colorType === 2 && [0, 2, 4].every((o, i) => trns.readUInt16BE(o) === s[i])) alpha = 0;
    out.set([...rgb, alpha], p * 4);
  }
  return out;
}

export const rgbPng = () =>
  encodePng({ width: 4, height: 4, colorType: 2, pixel: () => [10, 20, 30] });

export const opaqueRgbaPng = () =>
  encodePng({ width: 4, height: 4, colorType: 6, pixel: () => [10, 20, 30, 255] });

export const cutoutRgbaPng = () =>
  encodePng({ width: 4, height: 4, colorType: 6, pixel: (x) => [10, 20, 30, x < 2 ? 0 : 255] });
