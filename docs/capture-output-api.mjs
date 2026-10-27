// Capture the exact conversion methods used by the native spatial validation.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { declarations } from './site/source.mjs';
const [checkout,revision]=process.argv.slice(2);
if(!checkout||!revision?.match(/^[0-9a-f]{40}$/))throw Error('usage: capture-output-api.mjs CHECKOUT FULL_COMMIT');
const file='zenpixels-convert/src/output/plan.rs';
const source=execFileSync('git',['-C',checkout,'show',`${revision}:${file}`],{encoding:'utf8'});
const {code,line}=declarations(source,{scope:'impl OutputPlan {',methods:[
  'new','new_with_descriptor','new_preserving_samples','output_descriptor','metadata','prepare','borrow_rows','convert_rows',
]});
const policyPath='zenpixels/src/policy.rs';
const policySource=execFileSync('git',['-C',checkout,'show',`${revision}:${policyPath}`],{encoding:'utf8'});
const policy=declarations(policySource,{scope:'impl ConvertOptions {',methods:['retain_channels','with_clip_out_of_gamut','forbid_lossy']});
const surface=`use alloc::sync::Arc;
use zenpixels::{ColorContext, ColorOrigin, PixelDescriptor, PixelFormat, PixelSlice, PixelSliceMut};
use zenpixels_convert::{ConvertError, ConvertOptions, OutputPlan, output::{OutputMetadata, OutputProfile}};
use whereat::At;

${code.replaceAll('crate::cms::PluggableCms','zenpixels_convert::cms::PluggableCms')}\n\n${policy.code}\n`;
await writeFile(new URL('output-plan-api.json',import.meta.url),JSON.stringify({
  repository:'imazen/zenpixels',revision,path:file,line,
  policy:{path:policyPath,line:policy.line,source_sha256:createHash('sha256').update(policySource).digest('hex')},
  source_sha256:createHash('sha256').update(source).digest('hex'),
  surface,sha256:createHash('sha256').update(surface).digest('hex'),
},null,2)+'\n');
