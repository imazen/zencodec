// Targeted viewports avoid enormous full-page images and keep comparisons legible.
import { chromium } from 'playwright';
import { mkdir, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';
const [url, output] = process.argv.slice(2);
if (!url || !output) throw Error('URL and output directory required');
await mkdir(output, { recursive: true });
const browser = await chromium.launch({ headless: true, ...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE } : {}) });
const measurements = [];
try {
  for (const width of [390, 1440]) {
    const page = await browser.newPage({ viewport: { width, height: 1100 }, reducedMotion: 'reduce' });
    for (const id of ['entry-registry', 'consumer-2', 'adapter-2', 'policy']) {
      await page.goto(`${url}#${id}`);
      await page.waitForSelector('#lab-table tr');
      const section = page.locator(`#${id}`);
      await section.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `${output}/${id}-${width}.png` });
      measurements.push(await section.evaluate((el, width) => ({
        id: el.id, width, height: el.getBoundingClientRect().height,
        examples: [...el.querySelectorAll('.code-panel')].map(p => ({
          name: p.dataset.example, height: p.getBoundingClientRect().height,
          colors: [...new Set([...p.querySelectorAll('.code-focus span')].map(t => getComputedStyle(t).color))],
        })),
      }), width));
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    }
    await page.goto(`${url}#entry-registry`);
    await page.locator('#entry-registry [data-direction="above"] summary').click();
    await page.locator('#entry-registry [data-direction="below"] summary').click();
    await page.locator('#entry-registry-example').screenshot({ path: `${output}/expanded-${width}.png` });
    await page.goto(new URL('ideal.html#changes', url).href);
    await page.locator('#changes').screenshot({ path: `${output}/changes-${width}.png` });
    for (const id of ['video', 'resolution']) {
      const change = page.locator(`#change-${id}`);
      await change.locator(':scope>summary').click();
      await change.screenshot({ path: `${output}/change-${id}-${width}.png` });
      assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
      await change.locator(':scope>summary').click();
    }
    await page.close();
  }
  await writeFile(`${output}/measurements.json`, JSON.stringify(measurements, null, 2) + '\n');
} finally { await browser.close(); }
