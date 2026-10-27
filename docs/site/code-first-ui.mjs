import './ideal-ui.mjs';
const tabs=[...document.querySelectorAll('[data-role-tab]')];
const flows=[...document.querySelectorAll('[data-flow]')];
const remembered=new Map();
function selectRole(role,flowId) {
  for(const tab of tabs){const active=tab.dataset.roleTab===role;tab.setAttribute('aria-selected',active);tab.tabIndex=active?0:-1;document.getElementById(tab.getAttribute('aria-controls')).hidden=!active;}
  const panel=document.getElementById(`role-${role}`);
  const selected=flowId||remembered.get(role)||panel.querySelector('[data-flow]').id;
  remembered.set(role,selected);
  for(const flow of panel.querySelectorAll('[data-flow]'))flow.hidden=flow.id!==selected;
  for(const link of panel.querySelectorAll('.cf-menu a'))link.setAttribute('aria-current',link.hash===`#${selected}`?'true':'false');
}
for(const tab of tabs) {
  tab.addEventListener('click',()=>{selectRole(tab.dataset.roleTab);history.replaceState(null,'',`#${remembered.get(tab.dataset.roleTab)}`);});
  tab.addEventListener('keydown',event=>{
    let i=tabs.indexOf(tab);
    if(event.key==='ArrowRight')i=(i+1)%tabs.length;else if(event.key==='ArrowLeft')i=(i+tabs.length-1)%tabs.length;else if(event.key==='Home')i=0;else if(event.key==='End')i=tabs.length-1;else return;
    event.preventDefault();tabs[i].focus();tabs[i].click();
  });
}
function followHash(){const flow=flows.find(f=>`#${f.id}`===location.hash);if(flow)selectRole(flow.closest('[role=tabpanel]').id.replace('role-',''),flow.id);}
for(const role of tabs.map(t=>t.dataset.roleTab)){const panel=document.getElementById(`role-${role}`);for(const flow of [...panel.querySelectorAll('[data-flow]')].slice(1))flow.hidden=true;}
selectRole('consumer');followHash();addEventListener('hashchange',followHash);

const packageSelect=document.getElementById('cf-package'),search=document.getElementById('cf-search'),kind=document.getElementById('cf-kind'),legacy=document.getElementById('cf-legacy');
const results=document.getElementById('cf-results'),status=document.getElementById('cf-count'),more=document.getElementById('cf-more');
let manifest,filtered=[],limit=40,version=0;
const cache=new Map();
async function json(url){const response=await fetch(url);if(!response.ok)throw Error(`${url}: HTTP ${response.status}`);return response.json();}
function element(tag,text,className){const el=document.createElement(tag);if(text!==undefined)el.textContent=text;if(className)el.className=className;return el;}
function draw(){
  results.replaceChildren();
  for(const item of filtered.slice(0,limit)) {
    const article=element('article',undefined,'cf-signature');article.id=`signature-${item.id}`;
    const header=element('header'),label=element('span',`${item.package} / ${item.kind}`);header.append(label);
    if(item.detail?.deprecated)header.append(element('span','Deprecated','deprecated'));
    const link=element('a','Source ↗');link.href=`https://github.com/imazen/zencodec/blob/${manifest.sha}/${item.detail?.file||manifest.packages.find(p=>p.name===item.package).directory+'/src/lib.rs'}${item.detail?'#L'+item.detail.line:''}`;header.append(link);article.append(header);
    if(item.context!==item.signature)article.append(element('div',item.context,'cf-context'));
    const pre=element('pre'),code=element('code');code.innerHTML=item.html;pre.append(code);article.append(pre);
    const contract=element('details'),summary=element('summary','Contract & deprecation notes');contract.append(summary);
    if(item.detail?.deprecated)contract.append(element('div',`Deprecated: ${item.detail.deprecated.note||'See the source for the replacement.'}`,'cf-docs'));
    if(!item.documentation.length)contract.append(element('div','No local documentation was attached to this exported declaration. Inspect its source and surrounding type contract.','cf-docs'));
    for(const part of item.documentation) {
      if(part.html){const pre=element('pre'),code=element('code');code.innerHTML=part.html;pre.append(code);contract.append(pre);}
      else contract.append(element('div',part.text,'cf-docs'));
    }
    article.append(contract);
    if(item.flows.length) {
      const related=element('details');related.append(element('summary','Related usage flows'));
      for(const flow of item.flows){const link=element('a',flow.title);link.href=`#${flow.id}`;related.append(link,element('br'));}
      article.append(related);
    }
    results.append(article);
  }
  status.textContent=`${filtered.length} matching declarations · showing ${Math.min(limit,filtered.length)}`;
  more.hidden=limit>=filtered.length;
}
async function update(){
  const request=++version;status.textContent='Loading signatures…';
  try {
    manifest ||= await json('code-api/manifest.json');
    const names=packageSelect.value==='all'?manifest.packages.map(p=>p.name):[packageSelect.value];
    const arrays=await Promise.all(names.map(async name=>{if(!cache.has(name))cache.set(name,json(`code-api/${name}.json`).then(items=>items.map(item=>({...item,package:name}))));return cache.get(name);}));
    if(request!==version)return;
    const words=search.value.toLowerCase().trim().split(/\s+/).filter(Boolean);
    filtered=arrays.flat().filter(item=>(kind.value==='all'||kind.value===item.kind)&&(legacy.checked||!item.detail?.deprecated)&&words.every(word=>`${item.signature} ${item.context} ${item.detail?.docs||''} ${item.detail?.deprecated?.note||''}`.toLowerCase().includes(word)));
    limit=40;draw();
  }catch(error){if(request===version){status.textContent=`Could not load the catalog: ${error.message}`;results.replaceChildren();more.hidden=true;}}
}
for(const control of [packageSelect,search,kind,legacy])control.addEventListener('input',update);
more.addEventListener('click',()=>{limit+=40;draw();});
update();
