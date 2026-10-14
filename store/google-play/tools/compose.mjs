// The Play listing's graphics, drawn as HTML and rendered by Chromium.
// node compose.mjs ../graphics (after shoot.mjs has filled raw/)
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const here = path.dirname(new URL(import.meta.url).pathname);
const out = path.resolve(process.argv[2]);
const app = path.resolve(here, "../../../tracker-app");
const fonts = `${app}/public/fonts`;
const iconSvg = fs.readFileSync(`${app}/src-tauri/icons/source-icon.svg`, "utf8");
const raw = (f) => `file://${here}/raw/${f}`; // shoot.mjs writes these

const BASE = `
@font-face { font-family: Plex; font-weight: 400; src: url(file://${fonts}/ibm-plex-sans-400.woff2); }
@font-face { font-family: Plex; font-weight: 500; src: url(file://${fonts}/ibm-plex-sans-500.woff2); }
@font-face { font-family: Plex; font-weight: 600; src: url(file://${fonts}/ibm-plex-sans-600.woff2); }
@font-face { font-family: PlexMono; font-weight: 500; src: url(file://${fonts}/ibm-plex-mono-500.woff2); }
* { margin: 0; box-sizing: border-box; }
html, body { width: 100%; height: 100%; overflow: hidden; }
body { font-family: Plex, sans-serif; color: #e7e9ee; background: #14161a; }
.bg { position: absolute; inset: 0; background:
  radial-gradient(120% 70% at 50% 0%, rgba(45, 212, 191, 0.22), transparent 60%),
  linear-gradient(#171a1f, #101215); }
.accent { color: #2dd4bf; }
`;

// The icon full-bleed: Google Play rounds the corners itself.
const fullBleed = iconSvg.replace('<rect x="64" y="64" width="896" height="896" rx="200" fill="#14161a"/>', '<rect width="1024" height="1024" fill="#14161a"/>');

function shotPage({ w, h, title, sub, img, shotW, top, radius }) {
  return `<style>${BASE}
  .cap { position: absolute; left: 0; right: 0; top: ${Math.round(top * 0.26)}px; text-align: center; padding: 0 ${Math.round(w * 0.06)}px; }
  h1 { font-weight: 600; font-size: ${Math.round(Math.min(w, h) * 0.064)}px; line-height: 1.12; letter-spacing: -0.01em; }
  p { margin-top: ${Math.round(Math.min(w, h) * 0.018)}px; font-size: ${Math.round(Math.min(w, h) * 0.03)}px; color: #9aa3b2; }
  .shot { position: absolute; left: 50%; top: ${top}px; width: ${shotW}px; transform: translateX(-50%);
    border-radius: ${radius}px; border: ${Math.max(2, Math.round(radius / 12))}px solid #2c313a; overflow: hidden;
    box-shadow: 0 ${radius}px ${radius * 3}px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(45, 212, 191, 0.15); }
  .shot img { display: block; width: 100%; }
  </style><div class="bg"></div>
  <div class="cap"><h1>${title}</h1>${sub ? `<p>${sub}</p>` : ""}</div>
  <div class="shot"><img src="${img}"></div>`;
}

const JOBS = [];

// Phone: 1080 x 1920.
const phone = (n, title, sub, img) => JOBS.push({ file: `phone-${n}.png`, w: 1080, h: 1920, html: shotPage({ w: 1080, h: 1920, title, sub, img: raw(img), shotW: 840, top: 400, radius: 40 }) });
phone(1, `Every tracker,<br><span class="accent">in your pocket</span>`, "MOD, S3M, XM, IT and some eighty formats more", "phone-1-pattern.png");
phone(2, `Write patterns<br><span class="accent">with your fingers</span>`, "A piano, hex keys and the pattern editor", "phone-2-keys.png");
phone(3, `A sample editor<br><span class="accent">that does it all</span>`, "Cut, fade, loop, slice, stretch, normalize", "phone-3-samples.png");
phone(4, `Mix it,<br><span class="accent">then export it</span>`, "Faders, effect slots, WAV and stems", "phone-4-mix.png");

// 7-inch tablet: 1920 x 1200.
const t7 = (n, title, sub, img) => JOBS.push({ file: `tablet7-${n}.png`, w: 1920, h: 1200, html: shotPage({ w: 1920, h: 1200, title, sub, img: raw(img), shotW: 1500, top: 250, radius: 28 }) });
t7(1, `Tracker modules, <span class="accent">played and edited</span>`, "Each format played the way its own tracker played it", "tablet7-1-pattern.png");
t7(2, `Every sample, <span class="accent">shaped by hand</span>`, "A waveform editor with loops, slices and synths", "tablet7-2-samples.png");

// 10-inch tablet: 2560 x 1600.
const t10 = (n, title, sub, img) => JOBS.push({ file: `tablet10-${n}.png`, w: 2560, h: 1600, html: shotPage({ w: 2560, h: 1600, title, sub, img: raw(img), shotW: 2000, top: 330, radius: 36 }) });
t10(1, `The whole tracker, <span class="accent">on a tablet</span>`, "Patterns, orders, samples and instruments side by side", "tablet10-1-pattern.png");
t10(2, `Mix with <span class="accent">faders and effects</span>`, "Levels, panning and effect slots for every channel", "tablet10-2-mix.png");
t10(3, `Your song, <span class="accent">your rules</span>`, "Tempo, timing, swing and several order lists", "tablet10-3-song.png");

// The app icon: 512 x 512, 32-bit PNG.
JOBS.push({ file: "icon-512.png", w: 512, h: 512, html: `<style>${BASE} svg { width: 512px; height: 512px; display: block; }</style>${fullBleed}` });

// The feature graphic: 1024 x 500, no alpha.
JOBS.push({
  file: "feature-graphic.png", w: 1024, h: 500, jpegSafe: true,
  html: `<style>${BASE}
  .bg { background: radial-gradient(80% 120% at 85% 50%, rgba(45, 212, 191, 0.25), transparent 60%), linear-gradient(110deg, #14161a 40%, #0f1d1c); }
  .left { position: absolute; left: 64px; top: 0; bottom: 0; width: 470px; display: flex; flex-direction: column; justify-content: center; }
  .mark { width: 96px; height: 96px; margin-bottom: 26px; }
  .mark svg { width: 100%; height: 100%; }
  h1 { font-size: 54px; font-weight: 600; line-height: 1.05; letter-spacing: -0.015em; }
  p { margin-top: 16px; font-size: 22px; color: #9aa3b2; line-height: 1.35; }
  .tags { margin-top: 22px; font-family: PlexMono; font-size: 16px; color: #2dd4bf; letter-spacing: 0.04em; }
  .shot { position: absolute; left: 560px; top: 58px; width: 720px; border-radius: 16px; overflow: hidden;
    border: 2px solid #2c313a; box-shadow: 0 24px 60px rgba(0,0,0,.6); transform: perspective(1400px) rotateY(-14deg); transform-origin: left center; }
  .shot img { display: block; width: 100%; }
  </style><div class="bg"></div>
  <div class="shot"><img src="${raw("tablet7-1-pattern.png")}"></div>
  <div class="left">
    <div class="mark">${iconSvg}</div>
    <h1>Neumetik<span class="accent">.</span> Tracker</h1>
    <p>Tracker modules played and edited,<br>the way their own trackers did.</p>
    <div class="tags">MOD · S3M · XM · IT · AHX · 80+ MORE</div>
  </div>`,
});

fs.mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ executablePath: "/opt/pw-browsers/chromium", args: ["--allow-file-access-from-files"] });
for (const j of JOBS) {
  const page = await browser.newPage({ viewport: { width: j.w, height: j.h }, deviceScaleFactor: 1 });
  const file = path.join(here, "page.html");
  fs.writeFileSync(file, `<!doctype html><meta charset="utf-8">${j.html}`);
  await page.goto(`file://${file}`);
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(200);
  await page.screenshot({ path: path.join(out, j.file), omitBackground: false });
  await page.close();
  console.log(j.file);
}
await browser.close();
