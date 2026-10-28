import { createHighlighter } from 'shiki';

// Build-time only: no grammar, highlighter or WASM is shipped to readers.
const rust = await createHighlighter({ themes: ['github-light'], langs: ['rust'] });
const escape = value => value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');

export function highlight(code) {
  const { tokens } = rust.codeToTokens(code, { lang: 'rust', theme: 'github-light' });
  return tokens.map(line => line.map(token => {
    const styles = [`color:${token.color}`];
    if (token.fontStyle & 1) styles.push('font-style:italic');
    if (token.fontStyle & 2) styles.push('font-weight:600');
    if (token.fontStyle & 4) styles.push('text-decoration:underline');
    return `<span style="${styles.join(';')}">${escape(token.content)}</span>`;
  }).join('')).join('\n');
}

export function documentation(body) {
  const parts=[];let end=0;
  for(const block of body.matchAll(/```([^\n]*)\n([\s\S]*?)```/g)) {
    if(block.index>end)parts.push({text:body.slice(end,block.index)});
    const rust=!block[1]||/^(rust|ignore|no_run|should_panic|compile_fail|edition)/.test(block[1]);
    parts.push(rust?{code:block[2],html:highlight(block[2])}:{text:block[0]});end=block.index+block[0].length;
  }
  if(end<body.length)parts.push({text:body.slice(end)});
  return parts;
}
