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
// ---- trade page against the live Rust server
for (const [dev, w, h] of [["desktop", 1440, 1000], ["mobile", 390, 844]]) {
  const ctx = await browser.newContext({ viewport: { width: w, height: h } });
  const page = await ctx.newPage();
  page.on("pageerror", (e) => log.push(`[trade ${dev}] pageerror: ${e.message}`));
  page.on("console", (m) => m.type() === "error" && log.push(`[trade ${dev}] console: ${m.text()}`));
  try {
    await page.goto(URL + "trade", { waitUntil: "domcontentloaded" });
    await page.waitForFunction(() => document.getElementById("conn").textContent === "Live", null, { timeout: 15000 });
    await page.waitForTimeout(2500);
    await page.screenshot({ path: `shots/trade-${dev}-1-open.png`, fullPage: true });
    await page.locator("#joinBtn").click();
    await page.waitForSelector("#ticket:not([hidden])");
    // take the best ask: click it in the book, then buy 10
    await page.locator("#ladder li.ask").last().click();
    await page.fill("#qty", "10");
    await page.locator("#placeBtn").click();
    await page.waitForTimeout(800);
    const msg = await page.locator("#msg").textContent();
    // and rest a cheap bid
    await page.fill("#price", "3");
    await page.fill("#qty", "5");
    await page.locator("#placeBtn").click();
    await page.waitForTimeout(1800);
    await page.screenshot({ path: `shots/trade-${dev}-2-traded.png`, fullPage: true });
    const acct = (await page.locator("#acct").innerText()).replace(/\n/g, " | ");
    const orders = await page.locator("#orders li").count();
    log.push(`[trade ${dev}] after buy: "${msg}" | account: ${acct} | open orders: ${orders}`);
    await page.locator("#orders [data-cancel]").first().click();
    await page.waitForTimeout(1500);
    log.push(`[trade ${dev}] after cancel: open orders ${await page.locator("#orders [data-cancel]").count()}`);
    log.push(`[trade ${dev}] docWidth ${await page.evaluate(() => document.documentElement.scrollWidth)}`);
  } catch (e) {
    log.push(`[trade ${dev}] ERROR ${e.message.split("\n")[0]}`);
    await page.screenshot({ path: `shots/trade-${dev}-error.png`, fullPage: true }).catch(() => {});
  }
  await ctx.close();
}
await browser.close();
fs.writeFileSync("shots/log.txt", log.join("\n") + "\n");
