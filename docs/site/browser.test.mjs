import { chromium } from 'playwright';
import assert from 'node:assert/strict';
import { mkdir, readFile } from 'node:fs/promises';
import { example } from './source.mjs';
import { entries } from './api.mjs';
import { checkMigration } from './migration-browser.mjs';
import { checkIdeal } from './ideal-browser.mjs';
import { checkGaps } from './gaps-browser.mjs';
import { checkCodeFirst } from './code-first-browser.mjs';
const examples=await readFile(new URL('../examples/journeys.rs',import.meta.url),'utf8');
const policyExamples=await readFile(new URL('../examples/policy.rs',import.meta.url),'utf8');
const jpegExamples=await readFile(new URL('../examples/jpeg_policy.rs',import.meta.url),'utf8');
const pixelExamples=await readFile(new URL('../examples/pixel_conversion.rs',import.meta.url),'utf8');
const sources={'docs/examples/journeys.rs':examples,'docs/examples/policy.rs':policyExamples,'docs/examples/jpeg_policy.rs':jpegExamples,'docs/examples/pixel_conversion.rs':pixelExamples};
const url=process.env.SITE_URL || 'http://127.0.0.1:3537/';
const output=process.env.SITE_SCREENSHOTS;
if(output) await mkdir(output,{recursive:true});
const browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE?{executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE}:{})});
const errors=[];
try {
  const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce',permissions:['clipboard-read','clipboard-write']});
  page.on('pageerror',e=>errors.push(e.message));
  page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  const response=await page.goto(url);assert.equal(response.status(),200);
  await page.waitForSelector('#lab-table tr');
  assert.equal(await page.locator('.chapter:visible').count(),1);
  assert.equal(await page.locator('.entry-panel:visible').getAttribute('id'),'entry-registry');
  for(const panel of await page.locator('.code-panel').all()) {
    const name=await panel.getAttribute('data-example');
    const parts=await panel.locator('[data-example-part]').allTextContents();
    const source=await panel.getAttribute('data-example-source');
    assert.equal(parts.join(''),example(sources[source],name).full,`rendered complete scope: ${name}`);
  }
  const contrastFailures=await page.evaluate(()=>{
    const rgb=s=>s.match(/[\d.]+/g).map(Number);
    const luminance=c=>c.slice(0,3).map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4).reduce((n,v,i)=>n+v*[.2126,.7152,.0722][i],0);
    const failures=[];
    for(const token of document.querySelectorAll('.code-panel pre span,.api-panel pre span')) {
      if(!token.textContent.trim()) continue;
      const color=rgb(getComputedStyle(token).color);
      let parent=token,bg;
      while(parent) {
        const candidate=rgb(getComputedStyle(parent).backgroundColor);
        if(candidate.length===3||candidate[3]===1){bg=candidate;break;}
        parent=parent.parentElement;
      }
      if(!bg) throw Error('Code has no opaque background');
      const a=luminance(color),b=luminance(bg),ratio=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);
      if(ratio<4.5) failures.push({text:token.textContent,color,bg,ratio});
    }
    return failures;
  });
  assert.deepEqual(contrastFailures,[],'small code text needs 4.5:1 contrast, including expanded context');
  if(output) await page.screenshot({path:`${output}/desktop.png`,fullPage:true});
  for(const entry of entries) {
    await page.selectOption('#entry-select',entry.id);
    await page.waitForFunction(id=>!document.getElementById(`entry-${id}`).hidden,entry.id);
    assert.equal(await page.locator('.entry-panel:visible').count(),1);
    const panel=page.locator(`#entry-${entry.id}`);
    assert.ok((await panel.locator('.api-panel[open] pre').textContent()).includes('fn '));
    const code=panel.locator('.code-panel');
    await code.locator('[data-copy-full]').click();
    await page.waitForFunction(id=>document.querySelector(`#entry-${id} [data-copy-full]`).textContent==='Copied',entry.id);
    assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),example(examples,entry.snippet).full);
  }
  await page.goto(`${url}#entry-registry`);
  await page.waitForSelector('#entry-registry:visible');
  const registry=page.locator('#entry-registry-example');
  const above=registry.locator('[data-direction="above"]'),below=registry.locator('[data-direction="below"]');
  assert.ok(!await above.evaluate(el=>el.open));assert.ok(!await below.evaluate(el=>el.open));
  await above.locator('summary').focus();await page.keyboard.press('Enter');
  assert.ok(await above.locator('pre').isVisible());
  assert.ok((await above.textContent()).includes('pub fn gallery_codecs() -> CodecSet'));
  assert.ok(!await below.evaluate(el=>el.open));
  await below.locator('summary').click();
  assert.ok(await below.locator('pre').isVisible());
  assert.ok((await below.textContent()).includes('Ok(image)'));
  assert.ok(await registry.locator('.code-focus').isVisible());
  if(output) await page.locator('#entry-registry').screenshot({path:`${output}/registry-expanded.png`});
  await above.locator('summary').click();
  assert.ok(!await above.evaluate(el=>el.open));assert.ok(await below.evaluate(el=>el.open));
  await below.locator('summary').click();
  assert.ok(!await below.evaluate(el=>el.open));
  await page.goto(`${url}#policy`);
  assert.ok((await page.locator('#policy .api-panel[data-api="policyResolver"][open]').textContent()).includes('At<Self::Error>'));
  for(const [name,id] of [['policy_default','policy-default'],['policy_custom','policy-custom'],['jpeg_rewrite','policy-jpeg'],['jpeg_icc_refusal','policy-jpeg-checked'],['jpeg_icc_admission','policy-jpeg-icc']]) {
    if(name==='jpeg_icc_admission' && !await page.locator('.policy-deep-dive').evaluate(el=>el.open)) await page.locator('.policy-deep-dive>summary').click();
    const panel=page.locator(`#${id}-example`);
    for(const summary of await panel.locator('.code-context summary').all()) await summary.click();
    assert.ok(await panel.locator('[data-example-part="above"]').isVisible());
    assert.ok(await panel.locator('[data-example-part="below"]').isVisible());
    await panel.locator('[data-copy-full]').click();
    await page.waitForFunction(id=>document.querySelector(`#${id}-example [data-copy-full]`).textContent==='Copied',id);
    assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),example(sources[await panel.getAttribute('data-example-source')],name).full);
  }
  if(output) {
    await page.locator('#policy').screenshot({path:`${output}/policy-expanded.png`});
    await page.locator('.policy-lifecycle').screenshot({path:`${output}/policy-lifecycle.png`});
  }
  for(const lane of ['consumer','adapter','implementor']) {
    await page.locator(`[data-lane-link="${lane}"]`).click();
    for(let step=1;step<=5;step++) {
      await page.locator(`.step-link[href="#${lane}-${step}"]`).click();
      await page.waitForFunction(id=>!document.getElementById(id).hidden,`${lane}-${step}`);
      assert.equal(await page.locator('.chapter:visible').count(),1);
      assert.equal(await page.locator('.chapter:visible').getAttribute('id'),`${lane}-${step}`);
      assert.ok(await page.locator(`#${lane}-${step} .api-panel[open] pre`).first().isVisible());
      assert.ok((await page.locator(`#${lane}-${step} .api-panel[open] pre`).first().textContent()).includes('fn '));
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
  for(const context of await page.locator('#adapter-5 .code-context summary').all()) await context.click();
  assert.ok(await page.locator('#adapter-5 .code-provenance a').first().isVisible());
  await page.locator('#adapter-5 .code-source summary').click();
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
    await page.goto(`${url}#entry-registry`);await page.waitForSelector('#entry-registry:visible');
    for(const context of await page.locator('#entry-registry .code-context').all()) {
      if(!await context.evaluate(el=>el.open)) await context.locator('summary').click();
    }
    const expanded=await page.evaluate(()=>({w:innerWidth,scroll:document.documentElement.scrollWidth}));
    assert.ok(expanded.scroll<=expanded.w,`expanded context overflows at ${width}: ${JSON.stringify(expanded)}`);
    assert.ok(await page.locator('#entry-registry [data-example-part="above"]').isVisible());
    if(output) await page.locator('#entry-registry').screenshot({path:`${output}/registry-expanded-${width}.png`});
    await page.goto(`${url}#policy`);
    if(!await page.locator('.policy-deep-dive').evaluate(el=>el.open)) await page.locator('.policy-deep-dive>summary').click();
    for(const context of await page.locator('#policy-jpeg-icc-example .code-context').all()) {
      if(!await context.evaluate(el=>el.open)) await context.locator('summary').click();
    }
    const policyBounds=await page.evaluate(()=>({w:innerWidth,scroll:document.documentElement.scrollWidth}));
    assert.ok(policyBounds.scroll<=policyBounds.w,`expanded policy overflows at ${width}`);
    assert.ok(await page.locator('#policy-jpeg-icc-example [data-example-part="above"]').isVisible());
  }
  const links=await page.locator('a[href^="https://github.com/imazen/zencodec/blob/main/"]').evaluateAll(els=>els.map(el=>el.href));
  assert.ok(links.length>20);
  assert.deepEqual(errors,[]);
  await checkMigration(browser,url,output);
  await checkIdeal(browser,url,output);
  await checkGaps(browser,url,output);
  await checkCodeFirst(browser,url,output);
  console.log('Browser checks passed: 15 chapters with signatures, 7 entry points, exact full-example clipboard contents, independent inline context, deep links, 8 lab combinations, keyboard, 390/768/1440px layouts.');
} finally {await browser.close();}
