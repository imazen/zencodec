// Shared by the browser and source-rendering controls.
export function filterCatalog(items, {search='', kind='all', legacy=true}={}) {
  const words=search.toLowerCase().trim().split(/\s+/).filter(Boolean);
  return items.filter(item=>(kind==='all'||kind===item.kind)&&(legacy||!item.detail?.deprecated)&&words.every(word=>`${item.signature} ${item.context} ${item.detail?.docs||''} ${item.detail?.deprecated?.note||''}`.toLowerCase().includes(word)));
}
