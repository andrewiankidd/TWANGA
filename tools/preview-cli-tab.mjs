// Quick helper to preview the CLI tab on a docs page. Useful when
// updating CLI codeblocks to verify the rendered output without
// clicking through the live SPA. Usage:
//   cd tools && node preview-cli-tab.mjs <slug>
// e.g. `node preview-cli-tab.mjs playback`. Output goes to
// `C:/tmp/docs-<slug>-cli.png`. Not used by the normal docs
// screenshot pipeline.

import { chromium } from 'playwright';

const slug = process.argv[2] ?? 'tuner';
const browser = await chromium.launch();
const page = await browser.newPage({ viewport: { width: 1100, height: 1400 } });
await page.goto(`http://localhost:8000/app.html#docs/${slug}`, { waitUntil: 'networkidle' });
await page.waitForSelector('.docs-tab[data-tab="cli"]');
await page.click('.docs-tab[data-tab="cli"]');
await page.waitForTimeout(200);
await page.screenshot({ path: `C:/tmp/docs-${slug}-cli.png`, fullPage: true });
await browser.close();
console.log(`-> C:/tmp/docs-${slug}-cli.png`);
