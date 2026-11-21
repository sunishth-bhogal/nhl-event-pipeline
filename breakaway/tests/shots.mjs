// Review helper: drives the replay page in headless Chromium.
import { createRequire } from "module";
import fs from "fs";
const require = createRequire(process.env.NODE_PATH + "/");
const { chromium } = require("playwright");
fs.mkdirSync("shots", { recursive: true });
const URL = "http://localhost:8123/";
const log = [];
const browser = await chromium.launch();
for (const [dev, w, h] of [["desktop", 1440, 900], ["mobile", 390, 844]]) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => log.push(`[${dev}] pageerror: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && log.push(`[${dev}] console: ${m.text()}`));
  try {
    await page.goto(URL + "#2025020497", { waitUntil: "networkidle" });
    await page.waitForTimeout(1200);
    await page.screenshot({ path: `shots/${dev}-1-start.png`, fullPage: dev === "desktop" });
    // jump near the end of a dramatic game, then let it play
    await page.locator("#scrub").evaluate((el) => { el.value = 3420; el.dispatchEvent(new Event("input")); });
    await page.locator('.speeds button[data-speed="30"]').click();
    await page.locator("#playBtn").click();
    await page.waitForTimeout(5000);
    await page.screenshot({ path: `shots/${dev}-2-late.png` });
    await page.locator("#marketCard").screenshot({ path: `shots/${dev}-2b-market.png` });
    log.push(`[${dev}] market: last ${await page.locator("#mkLast").textContent()} quote ${await page.locator("#mkQuote").textContent()} vol ${await page.locator("#mkVol").textContent()} mm ${await page.locator("#mkMM").textContent()} ladder rows ${await page.locator("#ladder li").count()}`);
    await page.waitForTimeout(9000);
    await page.screenshot({ path: `shots/${dev}-3-end.png`, fullPage: true });
    log.push(`[${dev}] final: ${await page.locator("#period").textContent()} ${await page.locator("#clock").textContent()} | feed items ${await page.locator("#feed li").count()} | docWidth ${await page.evaluate(() => document.documentElement.scrollWidth)}`);
    // Cup clincher: next-goal button
    await page.selectOption("#gamePicker", "2024030416");
    await page.waitForTimeout(800);
    await page.locator("#nextGoal").click();
    await page.waitForTimeout(2500);
    await page.screenshot({ path: `shots/${dev}-4-cup-goal.png` });
    log.push(`[${dev}] cup: ${await page.locator("#awayScore").textContent()}-${await page.locator("#homeScore").textContent()} ${await page.locator("#homePct").textContent()}`);
  } catch (e) {
    log.push(`[${dev}] ERROR ${e.message.split("\n")[0]}`);
    await page.screenshot({ path: `shots/${dev}-error.png` }).catch(() => {});
  }
  await ctx.close();
}
await browser.close();
fs.writeFileSync("shots/log.txt", log.join("\n") + "\n");
