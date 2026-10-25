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
