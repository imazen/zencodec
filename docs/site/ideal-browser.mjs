import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { example } from './source.mjs';

export async function checkIdeal(browser,url,output) {
  const source=await readFile(new URL('../examples/checked_operations.rs',import.meta.url),'utf8');
  const video=await readFile(new URL('../examples/checked_video.rs',import.meta.url),'utf8');
  const positional=await readFile(new URL('../examples/checked_source.rs',import.meta.url),'utf8');
  const selection=await readFile(new URL('../../perf/spatial-validation/src/collection.rs',import.meta.url),'utf8');
  const storage=await readFile(new URL('../../perf/spatial-validation/src/jxl_packaged_storage.rs',import.meta.url),'utf8');
  const color=await readFile(new URL('../../perf/spatial-validation/src/jxl_color_oracle_tests.rs',import.meta.url),'utf8');
  const layout=await readFile(new URL('../../perf/spatial-validation/src/jxl_planned_color_tests.rs',import.meta.url),'utf8');
  const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce',permissions:['clipboard-read','clipboard-write']});
  const errors=[];
  page.on('pageerror',error=>errors.push(error.message));
  page.on('console',message=>{if(message.type()==='error')errors.push(message.text());});
  try {
    const response=await page.goto(new URL('ideal.html',url).href);
    assert.equal(response.status(),200);
    assert.ok((await page.locator('h1').textContent()).includes('executes today'));
    assert.equal(await page.locator('.ideal-step').count(),5);
    const changes=page.locator('.api-change');
    assert.equal(await page.locator('.api-change:not(#change-operation-storage):not(#change-float-color):not(#change-output-descriptor):not(#change-strided-plane):not(#change-auxiliary-grid):not(#change-frame-route):not(#change-mux-finish)').count(),12);
    assert.equal(await page.locator('.api-change:not(#change-float-color):not(#change-output-descriptor):not(#change-strided-plane):not(#change-auxiliary-grid):not(#change-frame-route):not(#change-mux-finish)').count(),13);
    assert.equal(await page.locator('#change-auxiliary-grid').count(),1);
    assert.equal(await page.locator('#change-frame-route').count(),1);
    assert.equal(await page.locator('#change-mux-finish').count(),1);
    assert.equal(await page.locator('#change-strided-plane').count(),1);
    assert.equal(await page.locator('#change-output-descriptor').count(),1);
    assert.equal(await page.locator('#change-float-color').count(),1);
    assert.equal(await page.locator('#change-mapping').count(),1);
    assert.ok((await page.locator('#change-images .code-toolbar').textContent()).includes('Private prototype'));
    assert.equal(await page.locator('#change-images [data-example="checked_jxl_first"]').count(),1);
    for(const change of await changes.all()) assert.ok(!await change.evaluate(el=>el.open));
    const registration=page.locator('#change-auxiliary-grid');
    await registration.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await registration.evaluate(el=>el.open));
    for(const required of ['pub enum AuxiliaryGrid','ColorSamples','Unknown','with_grid','dim_shift','before admitting output storage']) assert.ok((await registration.textContent()).includes(required),required);
    assert.equal(await registration.locator('svg .connector').count(),3);
    const colorChange=page.locator('#change-float-color');
    await colorChange.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await colorChange.evaluate(el=>el.open));
    // Four-stage routes must not clip the last owner or omit its connector.
    const route = colorChange.locator('svg');
    assert.equal(await route.locator('.connector').count(), 3);
    assert.ok(await route.evaluate(svg => [...svg.querySelectorAll('g > rect')].every(rect =>
      rect.x.baseVal.value + rect.width.baseVal.value <= svg.viewBox.baseVal.width)));
    assert.equal(await colorChange.locator('[data-example="native_float_color"]').count(),1);
    for(const required of ['OutputPlan','with_clip_out_of_gamut(false)','Little CMS','auxiliary-plane','0.0001']) assert.ok((await colorChange.textContent()).includes(required),required);
    for(const required of ['retain_auxiliaries','AuxiliaryImageOwner','9,455,616']) assert.ok((await colorChange.textContent()).includes(required),required);
    const owner=page.locator('#change-packed-owner');
    await owner.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await owner.evaluate(el=>el.open));
    for(const required of ['PackedImageOwner','from_packed_owner','ValueSlice','last image drops','Native workspace','NeedsCms']) assert.ok((await owner.textContent()).includes(required),required);
    const storageChange=page.locator('#change-operation-storage');
    await storageChange.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await storageChange.evaluate(el=>el.open));
    assert.equal(await storageChange.locator('[data-example="checked_output_storage"]').count(),1);
    for(const required of ['with_output_storage','CollectionOpenContext','open_with','UnsupportedOutputStorage','max_memory_bytes']) assert.ok((await storageChange.textContent()).includes(required),required);
    const inspection=page.locator('#change-inspection');
    await inspection.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await inspection.evaluate(el=>el.open));
    assert.equal(await inspection.locator('[data-example="checked_first_description"]').count(),1);
    assert.ok((await inspection.textContent()).includes('without reconstructing pixels'));
    assert.ok((await inspection.textContent()).includes('Unknown orientation remains unknown'));
    const factories=page.locator('#change-factories');
    await factories.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await factories.evaluate(el=>el.open));
    assert.ok((await factories.textContent()).includes('requires an already-resident slice'));
    assert.equal(await factories.locator('[data-example="checked_registered_first"]').count(),1);
    assert.ok((await factories.textContent()).includes('open_collection'));
    assert.ok((await factories.textContent()).includes('CollectionFactory'));
    assert.ok((await factories.textContent()).includes('refuse unqualified legacy'));
    assert.ok((await factories.textContent()).includes('stored width, height and pixel caps'));
    assert.ok((await factories.textContent()).includes('OpenRequest'));
    assert.ok((await factories.textContent()).includes('CollectionOpenError'));
    assert.ok((await factories.textContent()).includes('unsupported: ResourceLimits'));
    const placement=page.locator('#change-placement');
    await placement.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await placement.evaluate(el=>el.open));
    assert.ok((await placement.textContent()).includes('188×202'));
    assert.ok((await placement.textContent()).includes('CmykF32Inverted'));
    assert.ok((await placement.textContent()).includes('native raw output is implemented'));
    assert.ok((await placement.textContent()).includes('not a lossless re-encoding contract'));
    assert.equal(await placement.locator('[data-example="checked_jxl_layer_first"]').count(),1);
    const videoChange=page.locator('#change-video');
    await videoChange.locator(':scope>summary').focus();
    await page.keyboard.press('Enter');
    assert.ok(await videoChange.evaluate(el=>el.open));
    assert.ok((await videoChange.textContent()).includes('after native reconstruction'));
    assert.ok((await videoChange.textContent()).includes('byte budget'));
    for(const role of ['consumer','adapter','implementor']) {
      assert.equal(await page.locator(`section#${role}.${role}`).count(),1);
    }
    for(const panel of await page.locator('.code-panel').all()) {
      const exampleSource=await panel.getAttribute('data-example-source');
      const sources={'docs/examples/checked_operations.rs':source,'docs/examples/checked_video.rs':video,'docs/examples/checked_source.rs':positional,'perf/spatial-validation/src/collection.rs':selection,'perf/spatial-validation/src/jxl_packaged_storage.rs':storage,'perf/spatial-validation/src/jxl_color_oracle_tests.rs':color,'perf/spatial-validation/src/jxl_planned_color_tests.rs':layout};
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
      await details.evaluate(el=>{for(let p=el.parentElement;p;p=p.parentElement)if(p.tagName==='DETAILS')p.open=true;});
      if(!await details.evaluate(el=>el.open)) await details.locator(':scope>summary').click();
    }
    const signatures=(await page.locator('.ideal-signatures,.api-panel').allTextContents()).join('\n');
    for(const required of ['with_send_encoder','PolicyResolver','At<Self::Error>','step_controlled','transcode_async','prepare_send','resolve_frame','is_failed','ReadAtReader','stream_position','try_set_resolution','try_integer_density','with_resolution','describe','ImageDescription','with_orientation','with_placement']) assert.ok(signatures.includes(required),required);
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
