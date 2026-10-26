// Screenshots of the demo Loft web app (port 7440). The web app is dark only.
const { chromium } = require("playwright");
const path = require("path");
const os = require("os");

// Playwright's own Chromium (npx playwright install chromium).
const BASE = "http://localhost:7440";
const [ids, tasks] = [JSON.parse(process.argv[2]), JSON.parse(process.argv[3])];
const out = process.argv[4];

// [name, url, hide the agent settings panel]
const shots = [
  ["repos", "/repos"],
  ["repos-agents", "/repos", false, 520],
  ["agents", "/agents"],
  ["chat-orchestrator", `/agents/${ids["Orchestrator"]}`, true],
  ["chat-webdev", `/agents/${ids["Web Dev"]}`, true],
  ["chat-approval", `/agents/${ids["iOS Dev"]}`, true],
  ["chat-reviewer", `/agents/${ids["Reviewer"]}`, true],
  ["agent-settings", `/agents/${ids["Writer"]}`],
  ["tasks", "/tasks"],
  ["task-detail", `/tasks/${tasks["One-page checkout with a pickup time"]}`],
  ["task-epic", `/tasks/${tasks["Checkout redesign"]}`],
  ["settings", "/settings"],
];

// Hide this Mac's folder path: show a generic one instead.
async function scrub(page) {
  await page.evaluate(() => {
    const walk = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let n; (n = walk.nextNode()); ) {
      if (n.nodeValue.includes("/Users/")) n.nodeValue = n.nodeValue.replace(/\/Users\/[^\s·]*\/projects/g, "~/projects").replace(/\/Users\/\S*/g, "~/loft");
    }
    for (const i of document.querySelectorAll("input")) {
      if (i.value === "/usr/bin/false") i.value = "claude";
      if (i.value.includes("/Users/")) i.value = i.value.replace(/\/Users\/[^\s]*\/projects/, "~/projects").replace(/\/Users\/\S*/, "~/loft");
    }
  });
}

// Demo numbers for the header, instead of this Mac's live plan usage and temperature.
const soon = h => new Date(Date.now() + h * 3600e3).toISOString();
async function demoHeader(ctx) {
  await ctx.route("**/api/usage", r => r.fulfill({ json: { error: null, windows: [
    { key: "five_hour", label: "session", percent: 42, resets_at: soon(2) },
    { key: "seven_day", label: "week", percent: 18, resets_at: soon(80) }] } }));
  await ctx.route("**/api/system", r => r.fulfill({ json: { cpu_temp_c: 48, level: "nominal", sampled_at: new Date().toISOString() } }));
}

(async () => {
  const browser = await chromium.launch();
  for (const [name, url, hidePanel, scroll] of shots) {
    const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 2, colorScheme: "dark" });
    await demoHeader(ctx);
    const page = await ctx.newPage();
    await page.goto(BASE + url, { waitUntil: "load" });
    await page.waitForTimeout(2500);
    if (hidePanel) {
      const b = page.getByRole("button", { name: "Hide agent settings" });
      if (await b.count()) await b.first().click();
      await page.waitForTimeout(600);
    }
    if (scroll) { await page.mouse.wheel(0, scroll); await page.waitForTimeout(800); }
    await scrub(page);
    await page.screenshot({ path: `${out}/web-${name}.png` });
    await ctx.close();
  }
  for (const [name, url] of [["phone-chat", `/agents/${ids["Orchestrator"]}`], ["phone-tasks", "/tasks"]]) {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, colorScheme: "dark", isMobile: true, hasTouch: true });
    await demoHeader(ctx);
    const page = await ctx.newPage();
    await page.goto(BASE + url, { waitUntil: "load" });
    await page.waitForTimeout(2500);
    await scrub(page);
    await page.screenshot({ path: `${out}/web-${name}.png` });
    await ctx.close();
  }
  await browser.close();
})();
