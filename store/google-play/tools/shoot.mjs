// Screenshots of the web build for the Play listing.
// node shoot.mjs raw <phone|tablet7|tablet10> [view...], with the web build
// served at http://localhost:1490 (npm run build:web; npx vite preview --port 1490)
// and demo.mod made beside it (python3 make_mod.py demo.mod).
import { chromium } from "playwright";
import path from "node:path";

const [outDir, deviceName, ...views] = process.argv.slice(2);
const DEVICES = {
  phone: { viewport: { width: 360, height: 640 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true },
  tablet7: { viewport: { width: 960, height: 600 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
  tablet10: { viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true },
};
const song = path.join(path.dirname(new URL(import.meta.url).pathname), "demo.mod");

const browser = await chromium.launch({ executablePath: "/opt/pw-browsers/chromium", args: ["--autoplay-policy=no-user-gesture-required"] });
const ctx = await browser.newContext(DEVICES[deviceName]);
await ctx.addInitScript(() => localStorage.setItem("neumetik-tracker.in-development-accepted", "yes"));
const page = await ctx.newPage();
page.on("pageerror", (e) => console.error("pageerror", e.message));
await page.goto("http://localhost:1490/", { waitUntil: "networkidle" });
await page.getByTestId("tracker-open").waitFor();
const [chooser] = await Promise.all([page.waitForEvent("filechooser"), page.getByTestId("tracker-open").click()]);
await chooser.setFiles(song);
await page.getByTestId("tracker-info").waitFor();
await page.waitForTimeout(500);

async function shot(name) {
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(outDir, `${deviceName}-${name}.png`) });
  console.log("shot", name);
}

for (const view of views) {
  const i = view.indexOf(":"); const what = i < 0 ? view : view.slice(0, i); const arg = i < 0 ? undefined : view.slice(i + 1);
  if (what === "eval") {
    await page.evaluate(arg);
  } else if (what === "tab") {
    await page.getByTestId(`tracker-tab-${arg}`).click();
  } else if (what === "click") {
    await page.locator(arg).first().click();
  } else if (what === "scroll") {
    await page.locator(arg).first().scrollIntoViewIfNeeded();
  } else if (what === "top") {
    await page.evaluate(() => document.querySelector(".desk")?.scrollTo(0, 0));
  } else if (what === "play") {
    await page.getByTestId("tracker-play-song").click();
    await page.waitForTimeout(Number(arg ?? 3000));
  } else if (what === "stop") {
    await page.getByTestId("tracker-stop").click();
  } else if (what === "wait") {
    await page.waitForTimeout(Number(arg));
  } else if (what === "shot") {
    await shot(arg);
  } else if (what === "html") {
    console.log((await page.locator(arg).first().evaluate((e) => e.outerHTML)).slice(0, 3000));
  }
}
await browser.close();
