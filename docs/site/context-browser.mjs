import assert from 'node:assert/strict';
import { inlineContext } from './code-first.mjs';

// Preserve independent keyboard/mouse expansion for substantial context;
// short tails and example-local initialization must require no interaction.
export async function checkContext(page,panel) {
  const contexts=['above','below'].map(key=>panel.locator(`[data-direction=${key}]`));
  const fold=[];
  for(const context of contexts) {
    const text=await context.locator('code').textContent();
    const inline=inlineContext(text,true);
    assert.equal(await context.evaluate(el=>el.tagName),inline?'DIV':'DETAILS');
    if(inline) assert.ok(await context.locator('pre').isVisible());
    else {assert.ok(!await context.evaluate(el=>el.open));fold.push(context);}
  }
  for(const [i,context] of fold.entries()) {
    await context.locator('summary').focus();await page.keyboard.press('Enter');
    assert.ok(await context.locator('pre').isVisible());
    for(const other of fold.slice(i+1))assert.ok(!await other.evaluate(el=>el.open));
  }
  if(fold.length===2) {
    await fold[0].locator('summary').click();
    assert.ok(!await fold[0].evaluate(el=>el.open));
    assert.ok(await fold[1].evaluate(el=>el.open));
  }
}
