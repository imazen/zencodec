import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { codeFlows } from './code-first-data.mjs';
import { example, declarations } from './source.mjs';
import { implementation } from './code-first.mjs';

export async function checkCodeFirst(browser,url,output) {
  const page=await browser.newPage({viewport:{width:1440,height:1100},permissions:['clipboard-read','clipboard-write']});
  const errors=[];page.on('pageerror',e=>errors.push(e.message));page.on('console',m=>{if(m.type()==='error')errors.push(m.text());});
  try {
    const response=await page.goto(new URL('code.html',url).href);assert.equal(response.status(),200);
    await page.waitForFunction(()=>document.getElementById('cf-count').textContent.includes('matching declarations'));
    assert.equal(await page.locator('[role=tabpanel]:visible').count(),1);
    assert.equal(await page.locator('[data-flow]:visible').count(),1);
    assert.equal(await page.locator('[data-flow]:visible').getAttribute('id'),'app-start');
    const contrast=await page.evaluate(()=>{
      const rgb=value=>value.match(/[\d.]+/g).map(Number);
      const lum=color=>color.slice(0,3).map(x=>x/255).map(x=>x<=.04045?x/12.92:((x+.055)/1.055)**2.4).reduce((n,x,i)=>n+x*[.2126,.7152,.0722][i],0);
      const checked=new Set(),failures=[];
      for(const token of document.querySelectorAll('pre span')) {
        if(!token.textContent.trim())continue;
        const color=getComputedStyle(token).color;let parent=token,bg;
        while(parent){const candidate=rgb(getComputedStyle(parent).backgroundColor);if(candidate.length===3||candidate[3]===1){bg=candidate;break;}parent=parent.parentElement;}
        const key=color+String(bg);if(checked.has(key))continue;checked.add(key);
        if(!bg)throw Error('Code has no opaque background');
        const a=lum(rgb(color)),b=lum(bg),ratio=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);
        if(ratio<4.5)failures.push({color,bg,ratio});
      }
      return failures;
    });
    assert.deepEqual(contrast,[],'code highlighting must remain readable at small sizes');
    for(const flow of codeFlows) {
      await page.goto(new URL(`code.html#${flow.id}`,url).href);
      await page.waitForFunction(id=>!document.getElementById(id).hidden&&document.getElementById(id).checkVisibility(),flow.id);
      assert.equal(await page.locator('[role=tab][aria-selected=true]').getAttribute('data-role-tab'),flow.role);
      assert.equal(await page.locator('[data-flow]:visible').count(),1);
      for(const [i,step] of flow.steps.entries()) {
        const stage=page.locator(`#${flow.id} > .cf-step`).nth(i);
        assert.equal(await stage.getAttribute('open'),i===0?'':null,'only the starting step opens initially');
        assert.ok((await stage.locator(':scope > summary').textContent()).includes(step.title));
        if(i>0) {
          await stage.locator(':scope > summary').click();
          assert.equal(await stage.getAttribute('open'),'');
        }
        assert.equal(await stage.locator('.cf-why').textContent(),step.why);
        const source=await readFile(new URL(`../../${step.file}`,import.meta.url),'utf8');
        const parts=step.example?example(source,step.example):implementation(source,step.scope);
        const panel=page.locator(`#${flow.id}-${i}`);
        assert.equal((await panel.locator('[data-example-part]').allTextContents()).join(''),parts.full);
      }
    }
    await page.goto(new URL('code.html#app-start',url).href);
    const panel=page.locator('#app-start-0');
    await panel.locator('[data-direction=above] summary').click();
    assert.equal(await panel.locator('[data-direction=above]').getAttribute('open'),'');
    assert.equal(await panel.locator('[data-direction=below]').getAttribute('open'),null);
    await panel.locator('[data-direction=below] summary').click();
    await panel.locator('[data-copy-full]').click();
    await page.waitForFunction(()=>document.querySelector('#app-start-0 [data-copy-full]').textContent==='Copied');
    const registry=await readFile(new URL('../examples/checked_operations.rs',import.meta.url),'utf8');
    assert.equal(await page.evaluate(()=>navigator.clipboard.readText()),example(registry,'checked_registry').full);
    await page.locator('#tab-consumer').focus();await page.keyboard.press('ArrowRight');
    assert.equal(await page.locator('[role=tab][aria-selected=true]').getAttribute('id'),'tab-adapter');
    await page.keyboard.press('End');
    assert.equal(await page.locator('[role=tab][aria-selected=true]').getAttribute('id'),'tab-implementor');
    await page.locator('#cf-search').fill('OutputFinish validate');
    await page.waitForFunction(()=>document.querySelector('#cf-results').textContent.includes('validate'));
    assert.ok(await page.locator('.cf-signature').count()>0);
    await page.selectOption('#cf-package','zencodec-render');await page.locator('#cf-search').fill('Renderer render_first');
    await page.waitForFunction(()=>document.querySelector('#cf-results').textContent.includes('first top-level')||document.querySelector('#cf-results').textContent.includes('render_first'));
    assert.ok((await page.locator('#cf-results').textContent()).includes('render_first'));
    await page.selectOption('#cf-package','zencodec');await page.locator('#cf-search').fill('codec_details');
    await page.waitForFunction(()=>[...document.querySelectorAll('.cf-signature')].filter(card=>card.textContent.includes('Source-backed supplement')).length===3);
    const supplemented=page.locator('.cf-signature').filter({hasText:'Source-backed supplement'});
    assert.equal(await supplemented.count(),3);
    const detect=await readFile(new URL('../../src/detect.rs',import.meta.url),'utf8');
    const scopes=['impl dyn SourceEncodingDetails {','impl dyn SourceEncodingDetails + Send {','impl dyn SourceEncodingDetails + Send + Sync {'];
    for(const [i,scope] of scopes.entries()) {
      const declaration=declarations(detect,{scope,methods:['codec_details'],associated:false});
      const card=supplemented.nth(i);
      assert.equal(await card.locator(':scope > pre code').textContent(),declaration.code);
      assert.ok((await card.locator('header').textContent()).includes('Source-backed supplement'));
      assert.ok((await card.locator('header a').getAttribute('href')).endsWith(`/src/detect.rs#L${declaration.line}`));
      assert.ok((await card.locator(':scope > details').first().textContent()).includes('Downcast to a concrete codec probe type.'));
    }
    await page.locator('#cf-search').fill('codec_details Send Sync');
    await page.waitForFunction(()=>document.getElementById('cf-count').textContent.startsWith('1 matching'));
    assert.ok((await page.locator('.cf-signature > pre code').textContent()).includes('+ Send + Sync'));
    await page.selectOption('#cf-package','zencodec-render');await page.locator('#cf-search').fill('codec_details');
    await page.waitForFunction(()=>document.getElementById('cf-count').textContent.startsWith('0 matching'));
    await page.selectOption('#cf-package','zencodec');await page.locator('#cf-search').fill('');
    await page.waitForFunction(()=>document.querySelectorAll('.cf-signature').length===40);
    const before=parseInt(await page.locator('#cf-count').textContent());await page.locator('#cf-more').click();
    assert.equal(await page.locator('.cf-signature').count(),80);
    await page.locator('#cf-legacy').uncheck();await page.waitForFunction(previous=>parseInt(document.getElementById('cf-count').textContent)<previous,before);
    assert.equal(await page.locator('.cf-signature .deprecated').count(),0);
    await page.locator('#cf-search').fill('no_such_signature_expected_zero');
    await page.waitForFunction(()=>document.getElementById('cf-count').textContent.startsWith('0 matching'));
    assert.equal(await page.locator('.cf-signature').count(),0);
    await page.locator('#cf-search').fill('');
    await page.goto(new URL('code.html#app-start',url).href);
    await page.locator('#tab-consumer').click();
    await page.evaluate(()=>scrollTo(0,0));
    for(const width of [1440,768,390]) {
      await page.setViewportSize({width,height:1000});
      assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1),`no page overflow at ${width}`);
      if(output)await page.screenshot({path:`${output}/code-first-${width}.png`});
    }
    assert.deepEqual(errors,[]);
    console.log('Code-first checks passed: all source scopes, role tabs, context, copy, search, pagination, deprecations and responsive layouts.');
  }finally{await page.close();}
}
