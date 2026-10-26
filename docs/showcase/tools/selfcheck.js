// Self-check of the showcase page: offline load, console errors, theme toggle, overflow, reveals, reduced motion.
const { chromium } = require("playwright");
const path = require("path"), os = require("os");
// Playwright's own Chromium (npx playwright install chromium).
const file = "file://" + path.resolve(process.argv[2]), out = process.argv[3];
(async () => {
  const b = await chromium.launch();
  for (const [dev, vp] of [["desktop", { width: 1440, height: 900 }], ["phone", { width: 390, height: 844 }], ["small-phone", { width: 360, height: 740 }]]) {
    for (const motion of ["no-preference", "reduce"]) {
      const ctx = await b.newContext({ viewport: vp, colorScheme: "light", reducedMotion: motion, isMobile: dev !== "desktop", offline: true });
      const p = await ctx.newPage();
      const errs = [], reqs = [];
      p.on("pageerror", e => errs.push(e.message)); p.on("console", m => m.type() === "error" && errs.push(m.text()));
      p.on("request", r => !r.url().startsWith("file:") && !r.url().startsWith("data:") && reqs.push(r.url()));
      await p.goto(file); await p.waitForTimeout(800);
      const imgs = await p.$$eval("img", is => is.filter(i => getComputedStyle(i).display !== "none").length);
      const broken = await p.$$eval("img", is => is.filter(i => i.complete && i.naturalWidth === 0).length);
      // scroll through to trigger reveals
      const h = await p.evaluate(() => document.documentElement.scrollHeight);
      for (let y = 0; y < h; y += vp.height / 2) { await p.evaluate(y => scrollTo(0, y), y); await p.waitForTimeout(120); }
      await p.waitForTimeout(1300);
      await p.evaluate(() => scrollTo(0, document.documentElement.scrollHeight)); await p.waitForTimeout(1200);
      const hidden = await p.$$eval(".reveal,.reveal-media", els => els.filter(e => !e.classList.contains("in") && getComputedStyle(e).opacity === "0").map(e => e.className + ":" + e.textContent.slice(0, 30)));
      const overflow = await p.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      const bg1 = await p.evaluate(() => getComputedStyle(document.body).backgroundColor);
      await p.evaluate(() => scrollTo(0, 0)); await p.click(".theme"); await p.waitForTimeout(500);
      const bg2 = await p.evaluate(() => getComputedStyle(document.body).backgroundColor);
      const theme = await p.evaluate(() => document.documentElement.dataset.theme);
      const darkImgs = await p.$$eval("img.only-dark", is => is.filter(i => getComputedStyle(i).display !== "none").length);
      if (motion === "no-preference") await p.screenshot({ path: `${out}/check-${dev}-toggled-dark.png` });
      await p.click(".theme"); await p.waitForTimeout(300);
      const theme2 = await p.evaluate(() => document.documentElement.dataset.theme);
      console.log(JSON.stringify({ dev, motion, errs, externalRequests: reqs, visibleImgs: imgs, broken, unrevealed: hidden, overflowPx: overflow, bgLight: bg1, bgAfterToggle: bg2, theme, darkImgsShown: darkImgs, themeAfterSecondToggle: theme2 }));
      await ctx.close();
    }
  }
  await b.close();
})();
