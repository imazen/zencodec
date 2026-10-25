const cases=[...document.querySelectorAll('.migration-case')];
const audience=document.getElementById('migration-audience');
function filter() {
  for(const item of cases) item.hidden=audience.value!=='all'&&!item.dataset.audience.split(' ').includes(audience.value);
  document.getElementById('migration-count').textContent=`${cases.filter(item=>!item.hidden).length} migrations`;
}
audience.addEventListener('change',filter);
function revealHash() {
  const target=document.getElementById(location.hash.slice(1));
  const item=target?.closest('.migration-case');
  if(item?.hidden) {audience.value='all';filter();target.scrollIntoView();}
}
window.addEventListener('hashchange',revealHash);
document.querySelector('.migration-controls').addEventListener('click',event=>{
  const link=event.target.closest('a[href^="#"]');
  if(link&&document.getElementById(link.hash.slice(1))?.hidden) {audience.value='all';filter();}
});
filter();revealHash();
document.addEventListener('click',async event=>{
  const button=event.target.closest('[data-copy-full],[data-copy]');
  if(!button)return;
  const panel=button.closest('.code-panel');
  const text=button.dataset.copyFull
    ?[...panel.querySelectorAll('[data-example-part]')].map(part=>part.textContent).join('')
    :document.getElementById(button.dataset.copy).textContent;
  try {
    await navigator.clipboard.writeText(text);
    button.textContent='Copied';panel.querySelector('.copy-status').textContent='Example copied.';
  } catch {
    let field=panel.querySelector('.copy-fallback');
    if(!field){field=document.createElement('textarea');field.className='copy-fallback';field.readOnly=true;field.setAttribute('aria-label','Code selected for manual copy');panel.append(field);}
    field.value=text;field.focus();field.select();
    panel.querySelector('.copy-status').textContent='Use your system copy command.';
  }
});
