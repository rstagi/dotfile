// Export the deck to a vector, text-selectable PDF via reveal's ?print-pdf layout.
// Uses the system Google Chrome if present (channel: "chrome"), else Playwright's
// bundled Chromium. Run: `npm run export:pdf`   (OUT=name.pdf to rename)
import { chromium } from "playwright";
import { startServer } from "./serve.mjs";

const OUT = process.env.OUT || "deck.pdf";

const server = await startServer({ port: 0 });
const port = server.address().port;
const url = `http://127.0.0.1:${port}/?print-pdf`;

let browser;
try {
  browser = await chromium.launch({ channel: "chrome", headless: true });
} catch {
  browser = await chromium.launch({ headless: true }); // fall back to bundled chromium
}

try {
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  await page.goto(url, { waitUntil: "load", timeout: 60000 });

  // Wait for reveal to be ready and for web fonts to settle so glyphs embed.
  await page
    .waitForFunction(() => window.Reveal && window.Reveal.isReady && window.Reveal.isReady(), { timeout: 30000 })
    .catch(() => {});
  await page.evaluate(async () => {
    if (document.fonts && document.fonts.ready) await document.fonts.ready;
  });
  await page.waitForTimeout(600);

  if (errors.length) console.warn("page errors:\n  " + errors.join("\n  "));

  await page.pdf({
    path: OUT,
    printBackground: true,
    preferCSSPageSize: true,
    margin: { top: "0", right: "0", bottom: "0", left: "0" },
  });
  console.log(`wrote ${OUT}`);
} finally {
  await browser.close();
  server.close();
}
