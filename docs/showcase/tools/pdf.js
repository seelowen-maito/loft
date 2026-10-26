// Render dist/print.html to the showcase PDF.
const { chromium } = require("playwright");
const path = require("path"), os = require("os");
// Playwright's own Chromium (npx playwright install chromium).
(async () => {
  const b = await chromium.launch();
  const p = await b.newPage();
  await p.goto("file://" + path.resolve(process.argv[2]), { waitUntil: "load" });
  await p.waitForTimeout(1500);
  await p.pdf({ path: process.argv[3], width: "297mm", height: "210mm", printBackground: true, preferCSSPageSize: true });
  await b.close();
  // Clean metadata instead of the HeadlessChrome defaults.
  const { PDFDocument } = require("pdf-lib");
  const fs = require("fs");
  const doc = await PDFDocument.load(fs.readFileSync(process.argv[3]));
  doc.setTitle("Loft"); doc.setSubject("Your own crew of AI developers, working from your Mac.");
  doc.setAuthor("Loft"); doc.setCreator("Loft showcase"); doc.setProducer("Loft showcase");
  doc.setKeywords([]); const d = new Date(); doc.setCreationDate(d); doc.setModificationDate(d);
  fs.writeFileSync(process.argv[3], await doc.save());
})();
