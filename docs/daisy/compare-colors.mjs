#!/usr/bin/env node
// Colour candidates for the tray daisy, side by side at the sizes the trays use.
//
//   node docs/daisy/compare-colors.mjs <out_dir>
//
// 1. Exports one SVG per «иконка Nudge…» preset of constructor.html by running the
//    constructor's own model (MODEL-BEGIN … MODEL-END), so a candidate is exactly the
//    file the constructor would hand over as assets/daisy.svg.
// 2. Rasterises them with `cargo run --example daisy_raster`, resvg 0.49 with the
//    splice-and-fit of src/daisy.rs (pixel-identical to the hashes pinned there).
// 3. Writes <out_dir>/daisy-colors.html with every PNG inlined.
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import vm from "node:vm";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "../..");
const out = process.argv[2];
if (!out) throw new Error("usage: compare-colors.mjs <out_dir>");

// ---- 1. the candidates, from the constructor
const page = readFileSync(join(here, "constructor.html"), "utf8");
const data = /^const DATA = .*$/m.exec(page)[0];
const model = page.slice(page.indexOf("// MODEL-BEGIN"), page.indexOf("// MODEL-END"));
const ctx = vm.createContext({});
vm.runInContext(`${data}\n${model}\nthis.DATA=DATA; this.makePetals=makePetals; this.svgOf=svgOf;`, ctx);
const { DATA, makePetals, svgOf } = ctx;

const presets = DATA.presets.filter((p) => p.name.startsWith("иконка Nudge"));
const variants = presets.map((pr, i) => {
  const v = { disc_color: "#ffc837", ...pr.values };
  const prof = DATA.profiles[pr.profile], spread = DATA.profiles[pr.spread];
  const p = { ...v, profileName: prof.name, table: prof.table, spread: spread.spread,
    spreadName: spread.name, presetName: pr.name, presetEdited: false };
  const slug = i === 0 ? "current" : `v${i}`;
  return { slug, name: pr.name.replace(/^иконка Nudge,?\s*/, "") || "сейчас (assets/daisy.svg)",
    tip: v.tip_color, disc: v.disc_color, svg: svgOf(p, makePetals(p)) };
});
mkdirSync(join(out, "svg"), { recursive: true });
for (const v of variants) writeFileSync(join(out, "svg", `${v.slug}.svg`), v.svg);

// The reference must be the shipped icon: same geometry and colours, metadata aside.
const strip = (s) => s.replace(/<metadata>[\s\S]*?<\/metadata>/, "");
if (strip(variants[0].svg) !== strip(readFileSync(join(root, "assets/daisy.svg"), "utf8")))
  throw new Error("the «иконка Nudge» preset no longer exports assets/daisy.svg");

// ---- 2. pixels, the app's way
const SIZES = [16, 18, 22, 24, 32, 36, 44];
const FALL = [16, 12, 8, 4, 1];
execFileSync("cargo", ["run", "--quiet", "--example", "daisy_raster", "--",
  join(out, "png"), SIZES.join(","), "16", ...variants.map((v) => join(out, "svg", `${v.slug}.svg`))],
  { cwd: root, stdio: "inherit" });
const RECO = "v2";
execFileSync("cargo", ["run", "--quiet", "--example", "daisy_raster", "--",
  join(out, "png"), "36,18", FALL.join(","), join(out, "svg", `${RECO}.svg`)],
  { cwd: root, stdio: "inherit" });
const png = (slug, size, petals = 16) =>
  "data:image/png;base64," + readFileSync(join(out, "png", `${slug}-${size}-${petals}.png`)).toString("base64");

// ---- 3. the page
const BARS = {
  light: { label: "macOS, светлая строка меню", cls: "mac-l" },
  dark: { label: "macOS, тёмная строка меню", cls: "mac-d" },
  win: { label: "Windows 11, панель задач", cls: "win" },
};
// [css size, pixel size, caption]
const MAC = [[16, 16, "16 pt @1x"], [16, 32, "16 pt @2x"], [18, 18, "18 pt @1x"], [18, 36, "18 pt @2x"], [22, 22, "22 pt @1x"], [22, 44, "22 pt @2x"]];
const WIN = [[16, 16, "100 % · 16 px"], [24, 24, "150 % · 24 px"], [32, 32, "200 % · 32 px"]];
const img = (src, css, px, alt) =>
  `<img src="${src}" width="${css}" height="${css}" alt="${alt}"${px <= css ? ' class="px"' : ""}>`;
const chip = (hex) => `<span class="chip"><i style="background:${hex}"></i><code>${hex}</code></span>`;

const table = (bar, cols) => `
  <figure class="bar">
    <figcaption>${BARS[bar].label}</figcaption>
    <div class="scroll"><table class="${BARS[bar].cls}">
      <thead><tr><th></th>${cols.map(([, , c]) => `<th>${c}</th>`).join("")}</tr></thead>
      <tbody>${variants.map((v) => `<tr><th>${v.slug === "current" ? "сейчас" : v.slug.toUpperCase()}</th>${cols
        .map(([css, px]) => `<td>${img(png(v.slug, px), css, px, `${v.name}, ${px} px`)}</td>`).join("")}</tr>`).join("")}</tbody>
    </table></div>
  </figure>`;

const loupe = (bar, px, zoom) => `
  <figure class="bar">
    <figcaption>${BARS[bar].label}, ${px} px ×${zoom}</figcaption>
    <div class="loupe ${BARS[bar].cls}">${variants.map((v) => `
      <div><img class="px" src="${png(v.slug, px)}" width="${px * zoom}" height="${px * zoom}" alt="${v.name}, ${px} px, увеличено"><span>${v.slug === "current" ? "сейчас" : v.slug.toUpperCase()}</span></div>`).join("")}
    </div>
  </figure>`;

const reco = variants.find((v) => v.slug === RECO);
const fall = (bar) => `
  <div class="fall ${BARS[bar].cls}">${FALL.map((n) => `
    <div><span class="pair">${img(png(RECO, 36, n), 18, 36, `${n} лепестков`)}<img class="px" src="${png(RECO, 36, n)}" width="108" height="108" alt="${n} лепестков, увеличено"></span><span>${n} из 16</span></div>`).join("")}
  </div>`;

const html = `<title>Цвета ромашки в трее</title>
<meta name="claude:viewport-fit-sides" content="cover">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap">
<style>
/* Layout: one reading column; every comparison is a strip on the tray's own colour, candidates in rows. */
:root{
  --bg:#f6f6f8; --fg:#1c1c22; --muted:#62626e; --line:#dcdce3; --card:#ffffff; --accent:#4f6f9c;
  --mac-l:#e9e8ec; --mac-l-fg:#000; --mac-d:#2c2c2e; --mac-d-fg:#fff; --win:#202020; --win-fg:#fff;
  --sans:"IBM Plex Sans",system-ui,-apple-system,"Segoe UI",sans-serif; --mono:"IBM Plex Mono",ui-monospace,Menlo,monospace;
}
@media (prefers-color-scheme: dark){:root:not([data-theme="light"]){--bg:#141418;--fg:#ececf1;--muted:#9a9aa8;--line:#2e2e36;--card:#1c1c22;--accent:#93acd3;color-scheme:dark}}
:root[data-theme="dark"]{--bg:#141418;--fg:#ececf1;--muted:#9a9aa8;--line:#2e2e36;--card:#1c1c22;--accent:#93acd3;color-scheme:dark}
body{background:var(--bg);color:var(--fg);font:15px/1.5 var(--sans)}
main{max-width:980px;margin:0 auto;padding-block:32px 64px;padding-left:max(16px,env(safe-area-inset-left,0px));padding-right:max(16px,env(safe-area-inset-right,0px));display:grid;gap:40px}
h1{font-size:28px;line-height:1.2;margin:0 0 8px;font-weight:600;text-wrap:balance}
h2{font-size:19px;margin:0 0 4px;font-weight:600;text-wrap:balance}
h3{font-size:15px;margin:0;font-weight:600}
p{margin:0;max-width:68ch} .muted,figcaption{color:var(--muted);font-size:13px}
section{display:grid;gap:16px;min-width:0}
code{font:500 12.5px var(--mono)}
.cands{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:12px}
.cand{display:flex;gap:14px;align-items:center;background:var(--card);border:1px solid var(--line);border-radius:10px;padding:12px}
.cand.reco{border-color:var(--accent);box-shadow:0 0 0 1px var(--accent)}
.cand .meta{min-width:0;display:grid;gap:4px}
.nm{color:var(--muted);font-size:13px}
.tag{font:500 11px var(--sans);letter-spacing:.04em;text-transform:uppercase;color:var(--accent);margin-left:6px}
dl{display:grid;grid-template-columns:auto 1fr;gap:2px 10px;margin:4px 0 0;font-size:13px} dt{color:var(--muted)} dd{margin:0}
.chip{display:inline-flex;align-items:center;gap:6px} .chip i{width:12px;height:12px;border-radius:3px;border:1px solid var(--line)}
.swatch{display:flex;gap:0;border-radius:8px;overflow:hidden;flex:none}
.swatch span{display:grid;place-items:center;width:60px;height:96px}
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
.pair{display:flex;align-items:center;gap:14px}
.note{border-top:1px solid var(--line);padding-top:16px;display:grid;gap:8px;font-size:13.5px}
</style>
<main>
<header>
  <h1>Цвета ромашки в трее</h1>
  <p>Иконка Nudge с кончиками лепестков, сдвинутыми из розового в серо-голубой, и более насыщенным жёлтым диском. Форма, seed и лепестки у всех вариантов одни и те же, меняются только два цвета. <span class="muted" id="dpr"></span></p>
</header>

<section>
  <h2>Варианты</h2>
  <div class="cands">${variants.map((v) => `
  <article class="cand${v.slug === RECO ? " reco" : ""}">
    <div class="swatch"><span class="mac-l">${img(png(v.slug, 44), 22, 44, v.name + ", светлая строка")}</span><span class="mac-d">${img(png(v.slug, 44), 22, 44, v.name + ", тёмная строка")}</span></div>
    <div class="meta">
      <h3>${v.slug === "current" ? "Сейчас" : v.slug.toUpperCase()}${v.slug === RECO ? '<span class="tag">рекомендую</span>' : ""}</h3>
      <p class="nm">${v.name}</p>
      <dl><dt>кончик</dt><dd>${chip(v.tip)}</dd><dt>диск</dt><dd>${chip(v.disc)}</dd></dl>
    </div>
  </article>`).join("")}
  </div>
</section>

<section>
  <h2>В натуральную величину</h2>
  <p class="muted">Колонки @2x на экране Retina идут пиксель в пиксель, @1x показаны крупными пикселями, как на обычном мониторе.</p>
  ${table("light", MAC)}
  ${table("dark", MAC)}
  ${table("win", WIN)}
</section>

<section>
  <h2>Под лупой: 16 px</h2>
  <p class="muted">Самый мелкий размер, где кончик лепестка занимает один-два пикселя. Каждый пиксель увеличен в 8 раз.</p>
  ${loupe("light", 16, 8)}
  ${loupe("dark", 16, 8)}
  ${loupe("win", 16, 8)}
</section>

<section>
  <h2>Под лупой: 18 pt @2x</h2>
  <p class="muted">Обычный размер в строке меню MacBook, 36 px, увеличено в 3 раза.</p>
  ${loupe("light", 36, 3)}
  ${loupe("dark", 36, 3)}
</section>

<section>
  <h2>${RECO.toUpperCase()} по мере опадания</h2>
  <p class="muted">18 pt @2x: в натуральную величину и увеличено в 3 раза. Лепестки опадают с 12 часов по часовой стрелке.</p>
  ${fall("light")}
  ${fall("dark")}
</section>

<section class="note">
  <h2>Как получены картинки</h2>
  <p>SVG каждого варианта выгружен моделью конструктора (<code>docs/daisy/constructor.html</code>, пресеты «иконка Nudge, …»). Вариант «сейчас» совпадает с <code>assets/daisy.svg</code> во всём, кроме метаданных.</p>
  <p>Пиксели получены тем же путём, что и в приложении: resvg 0.49, опавшие лепестки вырезаны по id, viewBox вписан в квадрат размера иконки (<code>examples/daisy_raster.rs</code>). Для «сейчас» при 36 px результат совпадает с хешами, закреплёнными в <code>src/daisy.rs</code>.</p>
  <p>Цвета строк меню: светлая ${chip("#e9e8ec")}, тёмная ${chip("#2c2c2e")}, панель задач Windows ${chip("#202020")}.</p>
</section>
</main>
<script>
(function(){var d=window.devicePixelRatio||1,el=document.getElementById("dpr");if(el)el.textContent="Ваш экран: DPR "+d+".";})();
</script>
`;
writeFileSync(join(out, "daisy-colors.html"), html);
console.log(join(out, "daisy-colors.html"));
