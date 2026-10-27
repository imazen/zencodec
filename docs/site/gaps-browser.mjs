import assert from 'node:assert/strict';

export async function checkGaps(browser,url,output) {
  const page=await browser.newPage({viewport:{width:1440,height:1000},reducedMotion:'reduce'});
  try {
    const response=await page.goto(new URL('gaps.html',url).href);
    assert.equal(response.status(),200);
    assert.equal(await page.locator('.gap-table tbody tr').filter({hasNotText:'Video output transitions'}).count(),12);
    assert.equal(await page.locator('.gap-table tbody tr').filter({hasText:'Video output transitions'}).count(),1);
    assert.match(await page.locator('#canvas').textContent(),/not implemented yet/);
    assert.match(await page.locator('#canvas').textContent(),/1024×576/);
    assert.match(await page.locator('#compile-time').textContent(),/Not a confirmed performance comparison/);
    assert.match(await page.locator('#compile-time').textContent(),/all 60/);
    const svg=page.locator('#canvas svg');
    assert.equal(await svg.locator('.connector').count(),3);
    assert.ok(await svg.evaluate(svg=>[...svg.querySelectorAll('g > rect')].every(rect=>rect.x.baseVal.value+rect.width.baseVal.value<=svg.viewBox.baseVal.width)));
    for(const width of [390,768,1440]) {
      await page.setViewportSize({width,height:1000});
      const detail=page.locator('.gap-table details').first();
      if(!await detail.evaluate(el=>el.open)) {await detail.locator('summary').focus();await page.keyboard.press('Enter');}
      assert.ok(await detail.locator('p').isVisible());
      const bounds=await page.evaluate(()=>({width:innerWidth,scroll:document.documentElement.scrollWidth,
        over:[...document.querySelectorAll('.gap-table,.gap-table th,.gap-table td,header,footer')].filter(el=>el.getBoundingClientRect().right>innerWidth).map(el=>({tag:el.tagName,width:el.getBoundingClientRect().width,text:el.textContent.slice(0,60)}))}));
      assert.ok(bounds.scroll<=bounds.width,`gaps page overflow at ${width}: ${JSON.stringify(bounds)}`);
      if(output) await page.screenshot({path:`${output}/gaps-${width}.png`,fullPage:true});
    }
  } finally {await page.close();}
}
