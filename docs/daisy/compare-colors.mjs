#!/usr/bin/env node
// The tray daisy's colours and disc size, compared at the sizes the trays use.
//
//   node docs/daisy/compare-colors.mjs <out_dir>
//
// 1. Exports SVGs by running constructor.html's own model (MODEL-BEGIN … MODEL-END):
//    the current icon («иконка Nudge»), the proposal («иконка Nudge, дымчатый»), and the
//    proposal with its disc radius or disc colour swapped for the candidates below.
// 2. Rasterises them with `cargo run --example daisy_raster`, resvg 0.49 with the
//    splice-and-fit of src/daisy.rs (pixel-identical to the hashes pinned there).
// 3. Counts, per size, the pixels that read as clearly yellow and as petal, and writes
//    <out_dir>/daisy-colors.html with every PNG inlined.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { inflateSync } from "node:zlib";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const out = process.argv[2];
if (!out) throw new Error("usage: compare-colors.mjs <out_dir>");

const CURRENT = "иконка Nudge", PROPOSAL = "иконка Nudge, дымчатый";
const RADII = [8.5, 9.5, 10.5, 11]; // the constructor's disc slider stops at 11
const DISCS = ["#ffc837", "#ffc400", "#ffbb00", "#ffb300"];
const PETAL = "#f8f8fc"; // the petal's base colour in svgOf

// ---- 1. SVGs, from the constructor
const page = readFileSync(join(here, "constructor.html"), "utf8");
const ctx = vm.createContext({});
vm.runInContext(`${/^const DATA = .*$/m.exec(page)[0]}
${page.slice(page.indexOf("// MODEL-BEGIN"), page.indexOf("// MODEL-END"))}
this.DATA=DATA; this.makePetals=makePetals; this.svgOf=svgOf;`, ctx);
const { DATA, makePetals, svgOf } = ctx;
const preset = (name) => DATA.presets.find((p) => p.name === name) ?? (() => { throw new Error(`no preset «${name}»`); })();
function exportSvg(pr, over = {}) {
  const v = { disc_color: "#ffc837", ...pr.values, ...over };
  const prof = DATA.profiles[pr.profile], spread = DATA.profiles[pr.spread];
  const p = { ...v, profileName: prof.name, table: prof.table, spread: spread.spread, spreadName: spread.name,
    presetName: pr.name, presetEdited: Object.keys(over).length > 0 };
  return { v, svg: svgOf(p, makePetals(p)) };
}
const cur = preset(CURRENT), prop = preset(PROPOSAL);
const icons = [
  { slug: "current", label: "сейчас", ...exportSvg(cur) },
  { slug: "proposal", label: "дымчатый", ...exportSvg(prop) },
  ...RADII.map((r) => ({ slug: `r${r}`, label: `r ${r}`, ...exportSvg(prop, { disc: r }) })),
  ...DISCS.map((c) => ({ slug: `c${c.slice(1)}`, label: c, ...exportSvg(prop, { disc_color: c }) })),
];
mkdirSync(join(out, "svg"), { recursive: true });
for (const i of icons) writeFileSync(join(out, "svg", `${i.slug}.svg`), i.svg);

// The reference must be the shipped icon: same geometry and colours, metadata aside.
const strip = (s) => s.replace(/<metadata>[\s\S]*?<\/metadata>/, "");
if (strip(icons[0].svg) !== strip(readFileSync(join(root, "assets/daisy.svg"), "utf8")))
  throw new Error(`the «${CURRENT}» preset no longer exports assets/daisy.svg`);

// ---- 2. pixels, the app's way
const SIZES = [16, 18, 22, 24, 32, 36, 44];
const FALL = [16, 12, 8, 4, 1];
const raster = (sizes, petals, slugs) => execFileSync("cargo", ["run", "--quiet", "--example", "daisy_raster", "--",
  join(out, "png"), sizes.join(","), petals.join(","), ...slugs.map((s) => join(out, "svg", `${s}.svg`))],
  { cwd: root, stdio: "inherit" });
raster(SIZES, [16], icons.map((i) => i.slug));
raster([36], FALL, ["proposal"]);
const file = (slug, size, petals = 16) => join(out, "png", `${slug}-${size}-${petals}.png`);
const png = (slug, size, petals) => "data:image/png;base64," + readFileSync(file(slug, size, petals)).toString("base64");

// ---- 3. what reads as yellow and as petal
// A pixel the icon covers at least half counts as yellow when its CIELAB b* is over 50
// (the disc colours sit at 74–84, the petal at −2, so that is a pixel at least ~60 %
// disc), and as petal when b* is under 20. Both sit on the icon's own pixels, never on
// the bar: the disc is ringed by petals everywhere, so the counts hold on every bar.
function rgba(path) {
  const buf = readFileSync(path); let o = 8, w, h; const idat = [];
  while (o < buf.length) {
    const len = buf.readUInt32BE(o), type = buf.toString("latin1", o + 4, o + 8), d = buf.subarray(o + 8, o + 8 + len);
    if (type === "IHDR") { w = d.readUInt32BE(0); h = d.readUInt32BE(4); if (d[8] !== 8 || d[9] !== 6 || d[12]) throw new Error("8-bit RGBA, not interlaced"); }
    if (type === "IDAT") idat.push(d);
    o += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat)), st = w * 4, px = Buffer.alloc(st * h);
  for (let y = 0; y < h; y++) {
    const f = raw[y * (st + 1)];
    for (let x = 0; x < st; x++) {
      const a = x >= 4 ? px[y * st + x - 4] : 0, b = y ? px[(y - 1) * st + x] : 0, c = x >= 4 && y ? px[(y - 1) * st + x - 4] : 0;
      const pr = () => { const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c); return pa <= pb && pa <= pc ? a : pb <= pc ? b : c; };
      px[y * st + x] = raw[y * (st + 1) + 1 + x] + [0, a, b, (a + b) >> 1, f === 4 ? pr() : 0][f];
    }
  }
  return px;
}
const lin = (v) => ((v /= 255) <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
function lab(r, g, b) {
  const R = lin(r), G = lin(g), B = lin(b), f = (t) => (t > 0.008856 ? Math.cbrt(t) : 7.787 * t + 16 / 116);
  const X = f((0.4124 * R + 0.3576 * G + 0.1805 * B) / 0.95047), Y = f(0.2126 * R + 0.7152 * G + 0.0722 * B), Z = f((0.0193 * R + 0.1192 * G + 0.9505 * B) / 1.08883);
  return [116 * Y - 16, 500 * (X - Y), 200 * (Y - Z)];
}
const hexRgb = (h) => h.slice(1).match(/../g).map((x) => parseInt(x, 16));
function counts(slug, size) {
  const px = rgba(file(slug, size)); let yellow = 0, petal = 0;
  for (let i = 0; i < px.length; i += 4) {
    if (px[i + 3] < 128) continue;
    const b = lab(px[i], px[i + 1], px[i + 2])[2];
    if (b > 50) yellow++; else if (b < 20) petal++;
  }
  return { yellow, petal };
}
const M = Object.fromEntries(icons.map((i) => [i.slug, Object.fromEntries(SIZES.map((s) => [s, counts(i.slug, s)]))]));
const relLum = (h) => { const [r, g, b] = hexRgb(h).map(lin); return 0.2126 * r + 0.7152 * g + 0.0722 * b; };
const contrast = (a, b) => { const x = relLum(a), y = relLum(b); return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05); };

// ---- 4. the page
const BARS = {
  light: { label: "macOS, светлая строка меню", cls: "mac-l" },
  dark: { label: "macOS, тёмная строка меню", cls: "mac-d" },
  win: { label: "Windows 11, панель задач", cls: "win" },
};
// [css size, pixel size, caption]
const MAC = [[16, 16, "16 pt @1x"], [16, 32, "16 pt @2x"], [18, 18, "18 pt @1x"], [18, 36, "18 pt @2x"], [22, 22, "22 pt @1x"], [22, 44, "22 pt @2x"]];
const WIN = [[16, 16, "100 % · 16 px"], [24, 24, "150 % · 24 px"], [32, 32, "200 % · 32 px"]];
const bySlug = Object.fromEntries(icons.map((i) => [i.slug, i]));
const img = (src, css, px, alt) => `<img src="${src}" width="${css}" height="${css}" alt="${alt}"${px <= css ? ' class="px"' : ""}>`;
const chip = (hex) => `<span class="chip"><i style="background:${hex}"></i><code>${hex}</code></span>`;
const PICK = { r: `r${prop.values.disc}`, c: `c${prop.values.disc_color.slice(1)}` };

const table = (bar, cols, slugs) => `
  <figure class="bar">
    <figcaption>${BARS[bar].label}</figcaption>
    <div class="scroll"><table class="${BARS[bar].cls}">
      <thead><tr><th></th>${cols.map(([, , c]) => `<th>${c}</th>`).join("")}</tr></thead>
      <tbody>${slugs.map((s) => `<tr><th>${bySlug[s].label}</th>${cols
        .map(([css, px]) => `<td>${img(png(s, px), css, px, `${bySlug[s].label}, ${px} px`)}</td>`).join("")}</tr>`).join("")}</tbody>
    </table></div>
  </figure>`;
const loupe = (bar, px, zoom, slugs, pick) => `
  <figure class="bar">
    <figcaption>${BARS[bar].label}, ${px} px ×${zoom}</figcaption>
    <div class="loupe ${BARS[bar].cls}">${slugs.map((s) => `
      <div${s === pick ? ' class="pick"' : ""}><img class="px" src="${png(s, px)}" width="${px * zoom}" height="${px * zoom}" alt="${bySlug[s].label}, ${px} px, увеличено"><span>${bySlug[s].label}</span></div>`).join("")}
    </div>
  </figure>`;
const measure = (slugs, pick, head) => `
  <div class="scroll"><table class="data">
    <thead><tr><th>${head}</th>${SIZES.map((s) => `<th>${s} px</th>`).join("")}</tr></thead>
    <tbody>${slugs.map((s) => `<tr${s === pick ? ' class="pick"' : ""}><th>${bySlug[s].label}</th>${SIZES
      .map((z) => `<td><b>${M[s][z].yellow}</b><span>${M[s][z].petal}</span></td>`).join("")}</tr>`).join("")}</tbody>
  </table></div>`;
const RS = RADII.map((r) => `r${r}`), CS = DISCS.map((c) => `c${c.slice(1)}`);
const ring = (r, size) => ((31 - r) * size / 64).toFixed(1);

const card = (i, title) => `
  <article class="cand">
    <div class="swatch"><span class="mac-l">${img(png(i.slug, 44), 22, 44, i.label)}</span><span class="mac-d">${img(png(i.slug, 44), 22, 44, i.label)}</span><span class="win">${img(png(i.slug, 32), 32, 32, i.label)}</span></div>
    <div class="meta">
      <h3>${title}</h3>
      <dl><dt>кончик</dt><dd>${chip(i.v.tip_color)}</dd><dt>диск</dt><dd>${chip(i.v.disc_color)}</dd><dt>радиус</dt><dd><code>${i.v.disc}</code> из 64</dd></dl>
    </div>
  </article>`;

const html = `<title>Цвета ромашки в трее</title>
<meta name="claude:viewport-fit-sides" content="cover">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
/* Layout: one reading column; every comparison is a strip on the tray's own colour, candidates side by side. */
:root{
  --bg:#f6f6f8; --fg:#1c1c22; --muted:#62626e; --line:#dcdce3; --card:#ffffff; --accent:#4f6f9c; --hl:#eef2f8;
  --mac-l:#e9e8ec; --mac-l-fg:#000; --mac-d:#2c2c2e; --mac-d-fg:#fff; --win:#202020; --win-fg:#fff;
  --sans:"IBM Plex Sans",system-ui,-apple-system,"Segoe UI",sans-serif; --mono:"IBM Plex Mono",ui-monospace,Menlo,monospace;
}
@media (prefers-color-scheme: dark){:root:not([data-theme="light"]){--bg:#141418;--fg:#ececf1;--muted:#9a9aa8;--line:#2e2e36;--card:#1c1c22;--accent:#93acd3;--hl:#1f2530;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#141418;--fg:#ececf1;--muted:#9a9aa8;--line:#2e2e36;--card:#1c1c22;--accent:#93acd3;--hl:#1f2530;color-scheme:dark}
body{background:var(--bg);color:var(--fg);font:15px/1.5 var(--sans)}
main{max-width:980px;margin:0 auto;padding-block:32px 64px;padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px));display:grid;gap:40px}
h1{font-size:28px;line-height:1.2;margin:0 0 8px;font-weight:600;text-wrap:balance}
h2{font-size:19px;margin:0 0 4px;font-weight:600;text-wrap:balance}
h3{font-size:15px;margin:0;font-weight:600}
p{margin:0;max-width:68ch} .muted,figcaption{color:var(--muted);font-size:13px}
section{display:grid;gap:16px;min-width:0}
code{font:500 12.5px var(--mono)}
.cands{display:grid;grid-template-columns:repeat(auto-fill,minmax(min(300px,100%),1fr));gap:12px}
.cands.wide{grid-template-columns:repeat(auto-fill,minmax(min(380px,100%),1fr))}
.cand{display:flex;gap:14px;align-items:center;background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px;min-width:0}
.cand .meta{min-width:0;display:grid;gap:4px}
dl{display:grid;grid-template-columns:auto 1fr;gap:2px 10px;margin:4px 0 0;font-size:13px} dt{color:var(--muted)} dd{margin:0}
.chip{display:inline-flex;align-items:center;gap:6px} .chip i{width:12px;height:12px;border-radius:3px;border:1px solid var(--line)}
.swatch{display:flex;border-radius:8px;overflow:hidden;flex:none}
.swatch span{display:grid;place-items:center;width:52px;height:88px}
.bar{margin:0;display:grid;gap:6px;min-width:0}
.scroll{overflow-x:auto;border-radius:8px}
table{border-collapse:collapse;width:100%;font-size:12px}
th{font-weight:500;white-space:nowrap;padding:8px 10px;text-align:center}
tbody th{text-align:left;font:500 12px var(--mono)}
td{padding:6px 10px;text-align:center;vertical-align:middle}
td img,.fall img{vertical-align:middle}
img.px{image-rendering:pixelated}
.mac-l{background:var(--mac-l);color:var(--mac-l-fg)} .mac-d{background:var(--mac-d);color:var(--mac-d-fg)} .win{background:var(--win);color:var(--win-fg)}
.mac-l th,.mac-d th,.win th{opacity:.75}
.loupe,.fall{display:flex;flex-wrap:wrap;gap:18px;padding:16px;border-radius:8px}
.loupe div,.fall>div{display:grid;justify-items:center;gap:6px;font:500 12px var(--mono)}
.loupe .pick span{text-decoration:underline;text-underline-offset:3px}
table.data{background:var(--card);border:1px solid var(--line);font-variant-numeric:tabular-nums}
table.data td,table.data th{border-top:1px solid var(--line)}
table.data td b{font-weight:600} table.data td span{color:var(--muted);margin-left:6px}
table.data tr.pick{background:var(--hl)}
.pair{display:flex;align-items:center;gap:14px}
.note{border-top:1px solid var(--line);padding-top:16px;display:grid;gap:8px;font-size:13.5px}
</style>
<main>
<header>
  <h1>Цвета ромашки в трее</h1>
  <p>Текущая иконка рядом с профилем «дымчатый»: серо-голубые кончики лепестков, более насыщенный жёлтый диск и диск крупнее, чтобы центр было видно на 16 px. Форма, seed и лепестки те же. <span class="muted" id="dpr"></span></p>
</header>

<section>
  <h2>Сейчас и «дымчатый»</h2>
  <div class="cands">${card(bySlug.current, "Сейчас")}${card(bySlug.proposal, "Дымчатый")}</div>
</section>

<section>
  <h2>В натуральную величину</h2>
  <p class="muted">Колонки @2x на экране Retina идут пиксель в пиксель, @1x показаны крупными пикселями, как на обычном мониторе.</p>
  ${table("light", MAC, ["current", "proposal"])}
  ${table("dark", MAC, ["current", "proposal"])}
  ${table("win", WIN, ["current", "proposal"])}
</section>

<section>
  <h2>Под лупой</h2>
  <p class="muted">16 px ×8 и 18 pt @2x (36 px) ×4.</p>
  <div class="cands">
  ${loupe("light", 16, 8, ["current", "proposal"])}
  ${loupe("dark", 16, 8, ["current", "proposal"])}
  ${loupe("win", 16, 8, ["current", "proposal"])}
  </div>
  <div class="cands wide">
  ${loupe("light", 36, 4, ["current", "proposal"])}
  ${loupe("dark", 36, 4, ["current", "proposal"])}
  </div>
</section>

<section>
  <h2>Радиус диска</h2>
  <p>Цвет у всех ${chip(prop.values.disc_color)}, радиус в единицах холста 64. В таблице: сколько пикселей читаются как явно жёлтые (жирным) и сколько как лепесток (серым). При ${RADII.at(-1)} лепестковое кольцо на 16 px — ${ring(RADII.at(-1), 16)} px против ${ring(8.5, 16)} px сейчас.</p>
  ${measure(RS, PICK.r, "радиус")}
  ${loupe("light", 16, 8, RS, PICK.r)}
  ${loupe("dark", 16, 8, RS, PICK.r)}
  ${loupe("light", 36, 4, RS, PICK.r)}
  <p class="muted">На 16 px при радиусе ${prop.values.disc} жёлтых пикселей ${M[PICK.r][16].yellow} против ${M["r8.5"][16].yellow} при 8.5, а лепестки теряют ${M["r8.5"][16].petal - M[PICK.r][16].petal} px из ${M["r8.5"][16].petal}. Радиус 11 на 16, 18 и 22 px жёлтого не прибавляет, а лепестки теряют ещё ${M[PICK.r][16].petal - M["r11"][16].petal} px на 16 px и ${M[PICK.r][32].petal - M["r11"][32].petal} px на 32 px.</p>
</section>

<section>
  <h2>Цвет диска</h2>
  <p>Радиус у всех ${prop.values.disc}. Жёлтым диск выделяется только на фоне белых лепестков: он окружён ими со всех сторон и строки меню не касается.</p>
  <div class="scroll"><table class="data">
    <thead><tr><th>диск</th><th>насыщенность C*</th><th>яркость L*</th><th>контраст с лепестком</th><th>жёлтых px, 16 / 18 / 22</th></tr></thead>
    <tbody>${DISCS.map((c) => { const [L, a, b] = lab(...hexRgb(c)), s = `c${c.slice(1)}`; return `<tr${s === PICK.c ? ' class="pick"' : ""}><th>${chip(c)}${c === "#ffc837" ? " сейчас" : ""}</th><td>${Math.hypot(a, b).toFixed(1)}</td><td>${L.toFixed(1)}</td><td>${contrast(c, PETAL).toFixed(2)} : 1</td><td>${[16, 18, 22].map((z) => M[s][z].yellow).join(" / ")}</td></tr>`; }).join("")}</tbody>
  </table></div>
  ${loupe("light", 16, 8, CS, PICK.c)}
  ${loupe("dark", 36, 4, CS, PICK.c)}
</section>

<section>
  <h2>«Дымчатый» по мере опадания</h2>
  <p class="muted">18 pt @2x: в натуральную величину и увеличено в 3 раза. Лепестки опадают с 12 часов по часовой стрелке.</p>
  ${["light", "dark"].map((bar) => `
  <div class="fall ${BARS[bar].cls}">${FALL.map((n) => `
    <div><span class="pair">${img(png("proposal", 36, n), 18, 36, `${n} лепестков`)}<img class="px" src="${png("proposal", 36, n)}" width="108" height="108" alt="${n} лепестков, увеличено"></span><span>${n} из 16</span></div>`).join("")}
  </div>`).join("")}
</section>

<section class="note">
  <h2>Как получены картинки</h2>
  <p>SVG выгружены моделью конструктора (<code>docs/daisy/constructor.html</code>, профили «${CURRENT}» и «${PROPOSAL}»; варианты радиуса и цвета — «${PROPOSAL}» с одним изменённым полем). «Сейчас» совпадает с <code>assets/daisy.svg</code> во всём, кроме метаданных.</p>
  <p>Пиксели получены тем же путём, что и в приложении: resvg 0.49, опавшие лепестки вырезаны по id, viewBox вписан в квадрат размера иконки (<code>examples/daisy_raster.rs</code>). Для «сейчас» при 36 px результат совпадает с хешами, закреплёнными в <code>src/daisy.rs</code>.</p>
  <p>Подсчёт: пиксель, покрытый иконкой хотя бы наполовину, считается жёлтым при CIELAB b* больше 50 (это пиксель, на 60 % и больше занятый диском) и лепестком при b* меньше 20.</p>
  <p>Цвета строк меню: светлая ${chip("#e9e8ec")}, тёмная ${chip("#2c2c2e")}, панель задач Windows ${chip("#202020")}.</p>
</section>
</main>
<script>
(function(){var d=window.devicePixelRatio||1,el=document.getElementById("dpr");if(el)el.textContent="Ваш экран: DPR "+d+".";})();
</script>
`;
writeFileSync(join(out, "daisy-colors.html"), html);
console.log(join(out, "daisy-colors.html"));
