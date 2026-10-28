import { createHash } from 'node:crypto';
import { declarations, maskRust } from './source.mjs';
import { highlight, documentation } from './highlight.mjs';

// rustdoc JSON omits these inherent dyn-trait impls at the pinned revision.
// This is an explicit source supplement, not a second public-API parser.
const scopes = [
  'impl dyn SourceEncodingDetails {',
  'impl dyn SourceEncodingDetails + Send {',
  'impl dyn SourceEncodingDetails + Send + Sync {',
];
export function sourceCatalog(source, surface, details) {
  const actual = [...maskRust(source).matchAll(/\bimpl\s+dyn\s+SourceEncodingDetails\b[^{}]*\{/g)]
    .map(match => match[0].replace(/\s+/g, ' '));
  if (JSON.stringify(actual) !== JSON.stringify(scopes))
    throw Error('SourceEncodingDetails dyn impls changed; review the explicit source supplement');
  if (/\bcodec_details\s*[<(]/.test(surface) || details.some(detail => /::codec_details$/.test(detail.path)))
    throw Error('Compiler catalog now includes codec_details; review/retire the source supplement');
  return scopes.map(scope => {
    const declaration = declarations(source, {scope, methods:['codec_details'], associated:false});
    if (!/^\s*pub fn codec_details\b/m.test(declaration.code))
      throw Error('codec_details is no longer public; review the explicit source supplement');
    if (declaration.omittedMethods.length)
      throw Error('SourceEncodingDetails gained methods; review the explicit source supplement');
    // Only the contiguous /// documentation attached to this selected method.
    const lines = source.split('\n'), docs = [];
    for (let i = declaration.line - 2; i >= 0 && /^\s*\/\/\//.test(lines[i]); i--)
      docs.unshift(lines[i].replace(/^\s*\/\/\/ ?/, ''));
    if (!docs.length) throw Error('Missing source documentation for codec_details');
    const signature = declaration.code;
    return {
      id:createHash('sha256').update('source:src/detect.rs:'+scope+':codec_details').digest('hex').slice(0,16),
      package:'zencodec', signature, context:signature.split('\n')[0], kind:'function',
      provenance:'Source-backed supplement',
      detail:{file:'src/detect.rs', line:declaration.line, docs:docs.join('\n'), deprecated:null},
      flows:[], documentation:documentation(docs.join('\n')), html:highlight(signature),
    };
  });
}
