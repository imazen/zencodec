import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
const url=process.env.SITE_URL || 'http://127.0.0.1:3537/';
const output=process.env.SITE_SCREENSHOTS;
if(output) await mkdir(output,{recursive:true});
const browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE?{executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE}:{})});
const errors=[];
try {
  const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce'});
  page.on('pageerror',e=>errors.push(e.message));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  const response=await page.goto(url);assert.equal(response.status(),200);
  await page.waitForSelector('#lab-table tr');
  assert.equal(await page.locator('.chapter:visible').count(),1);
  if(output) await page.screenshot({path:`${output}/desktop.png`,fullPage:true});
  for(const lane of ['consumer','adapter','implementor']) {
    await page.locator(`[data-lane-link="${lane}"]`).click();
    for(let step=1;step<=5;step++) {
      await page.locator(`.step-link[href="#${lane}-${step}"]`).click();
      await page.waitForFunction(id=>!document.getElementById(id).hidden,`${lane}-${step}`);
      assert.equal(await page.locator('.chapter:visible').count(),1);
      assert.equal(await page.locator('.chapter:visible').getAttribute('id'),`${lane}-${step}`);
    }
  }
  for(const rtt of ['0','160']) for(const cache of ['0','131072']) for(const count of ['1','8']) {
    await page.selectOption('#rtt',rtt);await page.selectOption('#cache',cache);await page.selectOption('#concurrency',count);
    assert.equal(await page.locator('#lab-table tr').count(),4);
    assert.ok(!(await page.locator('#lab-table').textContent()).includes('NaN'));
  }
  await page.selectOption('#rtt','160');await page.selectOption('#cache','0');await page.selectOption('#concurrency','8');
  assert.ok((await page.locator('#lab-table').textContent()).includes('650.48576'));
  await page.goto(`${url}#adapter-5`);await page.waitForSelector('#adapter-5:visible');
  await page.locator('#adapter-5 details').evaluate(el=>{el.open=true;});
  assert.ok(await page.locator('#adapter-5 details a').first().isVisible());
  await page.locator('#adapter-5 [data-copy]').click();
  await page.waitForFunction(()=>/Copied|Selected/.test(document.querySelector('#adapter-5 [data-copy]').textContent));
  if(output) await page.screenshot({path:`${output}/adapter.png`,fullPage:true});
  await page.goto(url);
  await page.keyboard.press('Tab');
  assert.equal(await page.locator(':focus').getAttribute('class'),'skip');
  await page.keyboard.press('Enter');assert.ok(page.url().endsWith('#journeys'));
  for(const width of [390,768]) {
    await page.setViewportSize({width,height:844});
    await page.goto(`${url}#implementor-2`);
    await page.waitForSelector('#implementor-2:visible');
    const bounds=await page.evaluate(()=>({w:innerWidth,scroll:document.documentElement.scrollWidth}));
    assert.ok(bounds.scroll<=bounds.w,`body overflows at ${width}: ${JSON.stringify(bounds)}`);
    if(output) await page.screenshot({path:`${output}/mobile-${width}.png`,fullPage:true});
  }
  const links=await page.locator('a[href^="https://github.com/imazen/zencodec/blob/main/"]').evaluateAll(els=>els.map(el=>el.href));
  assert.ok(links.length>20);
  assert.deepEqual(errors,[]);
  console.log('Browser checks passed: 15 chapters, deep links, 8 lab combinations, copy, keyboard, 390/768/1440px layouts.');
} finally {await browser.close();}
