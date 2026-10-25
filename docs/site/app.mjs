const $ = (q,el=document) => el.querySelector(q);
const $$ = q => [...document.querySelectorAll(q)];
function activate() {
  const hash=location.hash.slice(1);
  const selected=$$('.chapter').find(el=>el.id===hash) || $('.chapter:not([hidden])') || $('.chapter');
  $$('.chapter').forEach(el=>{el.hidden=el!==selected;});
  $$('.steps-group').forEach(el=>{el.hidden=el.dataset.lane!==selected.dataset.lane;});
  $$('[data-lane-link]').forEach(el=>{
    const active=el.dataset.laneLink===selected.dataset.lane;
    el.classList.toggle('active',active);
    if(active) el.setAttribute('aria-current','true'); else el.removeAttribute('aria-current');
  });
  $$('.step-link').forEach(el=>{
    const active=el.hash===`#${selected.id}`;
    if(active) el.setAttribute('aria-current','step'); else el.removeAttribute('aria-current');
  });
  if(hash===selected.id) selected.scrollIntoView({block:'start'});
}
activate();
window.addEventListener('hashchange',activate);
document.addEventListener('click', async event=>{
  const button=event.target.closest('[data-copy]');
  if(!button) return;
  const text=document.getElementById(button.dataset.copy).textContent;
  try {
    if(navigator.clipboard?.writeText) await navigator.clipboard.writeText(text);
    else throw Error('Clipboard unavailable');
    button.textContent='Copied';
  } catch {
    const selection=window.getSelection();
    const range=document.createRange(); range.selectNodeContents(document.getElementById(button.dataset.copy));
    selection.removeAllRanges(); selection.addRange(range);button.textContent='Selected — copy';
  }
  setTimeout(()=>{button.textContent='Copy';},2000);
});
const names={serial:'Serial reads',batch:'Batched ranges',coalesced:'Merge gaps',budgeted:'Budgeted plan'};
try {
  const response=await fetch('transport.csv');
  if(!response.ok) throw Error('Cannot load recorded results');
  const text=await response.text();
  const [header,...lines]=text.trim().split('\n');const keys=header.split(',');
  const data=lines.map(line=>Object.fromEntries(line.split(',').map((v,i)=>[keys[i],i?Number(v):v])));
  function draw() {
    const rows=data.filter(r=>r.rtt_ms===Number($('#rtt').value)&&r.cache_bytes===Number($('#cache').value)&&r.concurrency===Number($('#concurrency').value));
    const max=Math.max(...rows.map(r=>r.simulated_ms));
    $('#bars').replaceChildren(...rows.map(row=>{
      const el=document.createElement('div');el.className=`bar-row ${row.policy}`;
      const label=document.createElement('span');label.textContent=names[row.policy];
      const track=document.createElement('div');track.className='bar-track';
      const bar=document.createElement('div');bar.className='bar';bar.style.width=`${row.simulated_ms/max*100}%`;
      const value=document.createElement('b');value.textContent=`${row.simulated_ms.toFixed(2)} ms`;
      track.append(bar);el.append(label,track,value);return el;
    }));
    $('#lab-table').replaceChildren(...rows.map(row=>{
      const tr=document.createElement('tr');
      [names[row.policy],row.simulated_ms.toFixed(5),row.transferred_bytes.toLocaleString('en-US'),row.dependency_rounds].forEach((v,i)=>{const cell=document.createElement(i?'td':'th');if(!i)cell.scope='row';cell.textContent=v;tr.append(cell);});return tr;
    }));
    $('#lab-takeaway').textContent=Number($('#rtt').value)===0?'With no round-trip cost, merging gaps spends bandwidth without a latency benefit in this model.':Number($('#concurrency').value)===1?'One request at a time removes the overlap benefit of batching. Merging still reduces round trips, at the cost of gap bytes.':'Batching overlaps round trips without fetching extra bytes. Merging is faster here, but the admitted byte budget may rule it out.';
  }
  ['rtt','cache','concurrency'].forEach(id=>document.getElementById(id).addEventListener('change',draw));draw();
} catch(error) { $('#lab-takeaway').textContent=`Recorded results unavailable: ${error.message}. Use the CSV download or linked methodology.`; }
