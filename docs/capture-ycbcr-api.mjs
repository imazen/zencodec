// Capture the shared borrowed converter used by native AV1 qualification.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { declarations } from './site/source.mjs';
const [checkout,revision]=process.argv.slice(2);
if(!checkout||!revision?.match(/^[0-9a-f]{40}$/))throw Error('usage: capture-ycbcr-api.mjs CHECKOUT FULL_COMMIT');
const specifications=[
  {path:'zenpixels/src/sample/plane.rs',scope:'pub enum ChromaSubsampling',methods:[],item:true},
  {path:'zenpixels/src/sample/plane.rs',scope:'pub enum ChromaSiting',methods:[],item:true},
  {path:'zenpixels/src/sample/plane.rs',scope:"impl<'a> CodePlane<'a>",methods:['from_u8','from_u16','width','height','encoding','code']},
  {path:'zenpixels-convert/src/ycbcr.rs',scope:"impl<'a> YcbcrToRgbF32<'a>",methods:['new','output_descriptor','convert_rows']},
  {path:'zenpixels-convert/src/ycbcr.rs',scope:'pub enum YcbcrError',methods:[],item:true},
];
const sources=specifications.map(spec=>{
  const source=execFileSync('git',['-C',checkout,'show',`${revision}:${spec.path}`],{encoding:'utf8'});
  const {code,line}=declarations(source,spec);
  return {path:spec.path,line,source_sha256:createHash('sha256').update(source).digest('hex'),code};
});
const surface=`use zenpixels::{Cicp, PixelDescriptor, PixelSliceMut, sample::{CodePlane, CodePlaneError, SampleEncoding, ChromaSubsampling, ChromaSiting}};
use zenpixels_convert::ycbcr::{YcbcrToRgbF32, YcbcrError};
use whereat::At;

${sources.map(s=>s.code).join('\n\n')}\n`;
await writeFile(new URL('ycbcr-api.json',import.meta.url),JSON.stringify({repository:'imazen/zenpixels',revision,sources,surface,sha256:createHash('sha256').update(surface).digest('hex')},null,2)+'\n');
