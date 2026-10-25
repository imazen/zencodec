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
