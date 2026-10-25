// Capture only rendered guide text, then the same section with details opened.
// Usage: node docs/evaluation/capture.mjs URL OUTPUT_DIR REVISION
import {chromium} from 'playwright';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
const [url,out,revision]=process.argv.slice(2);
if(!url||!out||!revision) throw Error('URL, output directory and revision required');
const tasks=JSON.parse(await readFile(new URL('./tasks.json',import.meta.url),'utf8'));
await mkdir(out,{recursive:true});
const browser=await chromium.launch({headless:true,...(process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE?{executablePath:process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE}:{})});
try {
 const page=await browser.newPage({viewport:{width:1440,height:1100},reducedMotion:'reduce'});
 for(const task of tasks) {
  const anchor=task.selector.startsWith('#policy-')?'#policy':task.selector==='.ownership'?'#ownership-title':task.selector;
  await page.goto(url+anchor);
  await page.waitForSelector('#lab-table tr');
  for(const expanded of [false,true]) {
   const capture=await page.evaluate(({selector,expanded})=>{
    let elements;
    const actual=document.querySelector(selector);
    if(actual) elements=[actual];
    else {
      if(!['#policy-jpeg-scenario','#policy-stream-scenario'].includes(selector)) throw Error(`Missing section: ${selector}`);
      const policy=[...document.querySelector('#policy').children];
      const start=policy.findIndex(e=>e.tagName==='H3'&&e.textContent.startsWith(selector==='#policy-jpeg-scenario'?'3.':'5.'));
      const end=selector==='#policy-jpeg-scenario'?policy.findIndex(e=>e.tagName==='H3'&&e.textContent.startsWith('5.')):policy.length;
      if(start<0) throw Error(`Missing section: ${selector}`);
      elements=policy.slice(start,end);
    }
    for(const el of elements) for(const d of [...(el.matches('details')?[el]:[]),...el.querySelectorAll('details')]) if(expanded) d.open=true;
    return {text:elements.map(el=>el.innerText).join('\n\n'),links:elements.flatMap(el=>[...el.querySelectorAll('a')].map(a=>({text:a.textContent.trim(),href:a.href}))),height:elements.reduce((n,el)=>n+el.getBoundingClientRect().height,0)};
   },{selector:task.selector,expanded});
   const stage=expanded?'expanded':'visible';
   const text=`# ${task.id} / ${stage}\n\nSource: ${url}${anchor} (revision ${revision}).\n\n${capture.text}\n\nLinks printed on this section (do not follow them during this test):\n${capture.links.map(l=>`- [${l.text}](${l.href})`).join('\n')}\n`;
   await writeFile(path.join(out,`${task.id}-${stage}.md`),text);
   await writeFile(path.join(out,`${task.id}-${stage}.json`),JSON.stringify({task:task.id,stage,revision,words:text.split(/\s+/).length,rendered_height:capture.height,links:capture.links},null,2)+'\n');
  }
 }
} finally {await browser.close()}
