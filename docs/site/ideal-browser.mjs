import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { example } from './source.mjs';

export async function checkIdeal(browser,url,output) {
  const source=await readFile(new URL('../examples/checked_operations.rs',import.meta.url),'utf8');
  const video=await readFile(new URL('../examples/checked_video.rs',import.meta.url),'utf8');
  const positional=await readFile(new URL('../examples/checked_source.rs',import.meta.url),'utf8');
  const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce',permissions:['clipboard-read','clipboard-write']});
  const errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('console',message=>{if(message.type()==='error')errors.push(message.text());});
  try {
    const response=await page.goto(new URL('ideal.html',url).href);
    assert.equal(response.status(),200);
    assert.ok((await page.locator('h1').textContent()).includes('executes today'));
    assert.equal(await page.locator('.ideal-step').count(),5);
    for(const role of ['consumer','adapter','implementor']) {
      assert.equal(await page.locator(`section#${role}.${role}`).count(),1);
    }
    for(const panel of await page.locator('.code-panel').all()) {
      const exampleSource=await panel.getAttribute('data-example-source');
      const sources={'docs/examples/checked_operations.rs':source,'docs/examples/checked_video.rs':video,'docs/examples/checked_source.rs':positional};
      assert.ok(exampleSource in sources);
      // An adapter example may start inside a collapsed explanation. Open that
      // container without changing the independent above/below default states.
      await panel.evaluate(el=>{for(let p=el.parentElement;p;p=p.parentElement)if(p.tagName==='DETAILS')p.open=true;});
      const parts=example(sources[exampleSource],await panel.getAttribute('data-example'));
      assert.equal((await panel.locator('[data-example-part]').allTextContents()).join(''),parts.full);
      const above=panel.locator('[data-direction="above"]');
      const below=panel.locator('[data-direction="below"]');
      assert.ok(!await above.evaluate(el=>el.open));
      assert.ok(!await below.evaluate(el=>el.open));
      await above.locator('summary').focus();await page.keyboard.press('Enter');
      assert.ok(await above.locator('pre').isVisible());
      assert.ok(!await below.evaluate(el=>el.open),'independent below context');
      await below.locator('summary').click();
      assert.ok(await below.locator('pre').isVisible());
      await above.locator('summary').click();
      assert.ok(await below.evaluate(el=>el.open),'closing above preserves below');
      await panel.locator('[data-copy-full]').click();
      await page.waitForFunction(id=>document.querySelector(`#${id} [data-copy-full]`).textContent==='Copied',await panel.getAttribute('id'));
      assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),parts.full);
    }
    for(const details of await page.locator('.ideal-signatures').all()) {
      if(!await details.evaluate(el=>el.open)) await details.locator(':scope>summary').click();
    }
    const signatures=(await page.locator('.ideal-signatures,.api-panel').allTextContents()).join('\n');
    for(const required of ['with_send_encoder','PolicyResolver','At<Self::Error>','step_controlled','transcode_async','prepare_send','resolve_frame','is_failed','ReadAtReader','stream_position']) assert.ok(signatures.includes(required),required);
    assert.ok((await page.locator('#target').textContent()).includes('Native/CMS workspace'));
    assert.ok((await page.locator('#target').textContent()).includes('source version'));
    for(const width of [390,768,1440]) {
      await page.setViewportSize({width,height:844});
      const bounds=await page.evaluate(()=>({width:innerWidth,scroll:document.documentElement.scrollWidth,offenders:[...document.querySelectorAll('section,article,details,pre,nav,figure,table,div')].filter(el=>el.getBoundingClientRect().right>innerWidth).map(el=>({tag:el.tagName,cls:el.className,id:el.id,right:el.getBoundingClientRect().right})).slice(0,16)}));
      assert.ok(bounds.scroll<=bounds.width,`expanded ideal page overflows: ${JSON.stringify(bounds)}`);
      if(output) await page.screenshot({path:`${output}/ideal-${width}.png`,fullPage:true});
    }
    await page.goto(new URL('ideal.html',url).href);
    await page.keyboard.press('Tab');
    assert.equal(await page.locator(':focus').getAttribute('class'),'skip');
    await page.keyboard.press('Enter');
    await page.waitForURL(url=>url.hash==='#ideal-main');
    assert.deepEqual(errors,[]);
    console.log('Checked API browser checks passed: exact examples, independent expansion, signatures, clipboard, keyboard and responsive roles.');
  } finally {await page.close();}
}
