import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {example} from './source.mjs';

export async function checkMigration(browser,url,output) {
  const source=await readFile(new URL('../examples/pixel_conversion.rs',import.meta.url),'utf8');
  const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce',permissions:['clipboard-read','clipboard-write']});
  const errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('console',message=>{if(message.type()==='error')errors.push(message.text());});
  try {
    const response=await page.goto(new URL('migration.html',url).href);
    assert.equal(response.status(),200);
    assert.equal(await page.locator('meta[name="robots"]').getAttribute('content'),'noindex, nofollow');
    assert.equal(await page.locator('.migration-case:visible').count(),6);
    for(const item of await page.locator('.migration-case').all()) {
      assert.deepEqual(await item.locator('[data-stage]').evaluateAll(els=>els.map(el=>el.dataset.stage)),['released','current','proposed']);
    }
    if(output) await page.screenshot({path:`${output}/migration-desktop.png`});
    for(const [role,count] of [['consumer',5],['adapter',5],['implementor',4]]) {
      await page.selectOption('#migration-audience',role);
      assert.equal(await page.locator('.migration-case:visible').count(),count);
    }
    await page.locator('.migration-controls a[href="#pixels"]').click();
    assert.equal(await page.locator('#migration-audience').inputValue(),'all');
    assert.ok(await page.locator('#pixels').isVisible());
    if(output) await page.locator('#pixels .migration-diagram').screenshot({path:`${output}/migration-pixels.png`});
    await page.locator('.conversion-step>summary').click();
    for(const panel of await page.locator('.conversion-story .code-panel').all()) {
      const name=await panel.getAttribute('data-example');
      const parts=example(source,name);
      assert.equal((await panel.locator('[data-example-part]').allTextContents()).join(''),parts.full);
      const above=panel.locator('[data-direction="above"]');
      const below=panel.locator('[data-direction="below"]');
      assert.ok(!await above.evaluate(el=>el.open));
      assert.ok(!await below.evaluate(el=>el.open));
      await above.locator('summary').click();
      assert.ok(!await below.evaluate(el=>el.open),'above/below expansion must stay independent');
      await below.locator('summary').click();
      await panel.locator('[data-copy-full]').click();
      await page.waitForFunction(id=>document.querySelector(`#${id} [data-copy-full]`).textContent==='Copied',await panel.getAttribute('id'));
      assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),parts.full);
    }
    await page.locator('#requirements .migration-proof>summary').click();
    assert.ok((await page.locator('#requirements .api-section').textContent()).includes('try_set_policy'));
    assert.ok((await page.locator('#requirements .api-section').textContent()).includes('At<Self::Error>'));
    for(const width of [390,768,1440]) {
      await page.setViewportSize({width,height:844});
      const bounds=await page.evaluate(()=>({width:innerWidth,scroll:document.documentElement.scrollWidth}));
      assert.ok(bounds.scroll<=bounds.width,`expanded migration page overflows: ${JSON.stringify(bounds)}`);
      const columns=await page.locator('#pixels .migration-stages').evaluate(el=>getComputedStyle(el).gridTemplateColumns.split(' ').length);
      assert.equal(columns,width<=850?1:3);
      if(output&&width===390) await page.locator('#pixels .migration-diagram').screenshot({path:`${output}/migration-mobile.png`});
    }
    await page.goto(new URL('migration.html',url).href);
    await page.keyboard.press('Tab');
    assert.equal(await page.locator(':focus').getAttribute('class'),'skip');
    await page.keyboard.press('Enter');
    assert.ok(page.url().endsWith('#migration-main'));
    assert.deepEqual(errors,[]);
    console.log('Migration browser checks passed: release-state diagrams, audience filters, deep links, exact full-example copy, independent inline expansion, signatures, keyboard and responsive layouts.');
  } finally {await page.close();}
}
