// Capture named source declarations from the same immutable integration used by examples.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { declarations, snapshotMethods } from './site/source.mjs';
const [checkout,revision]=process.argv.slice(2);
if(!checkout||!revision?.match(/^[0-9a-f]{40}$/))throw Error('usage: capture-integration-api.mjs CHECKOUT FULL_COMMIT');
const read=file=>execFileSync('git',['-C',checkout,'show',`${revision}:${file}`],{encoding:'utf8'});
const file='zencodecs/src/checked_transcode.rs';
const asyncFile='zencodecs/src/checked_transcode/asynchronous.rs';
const snapshot=read('docs/public-api/zencodecs.features.txt');
const methods=snapshotMethods(snapshot,"checked_transcode::TranscodeRequest<'a>");
if(!methods.length)throw Error('Missing integration API');
const imports=`use zencodec::{
    At, CodecSet, ImageFormat,
    encode::EncodeOutput,
    operation::{self, OperationReport, Outcome, PolicyResolver, PolicySet, MetadataTransfer},
    storage::StorageBudget,
};
use zencodec_io::{DriveOptions, tokio_io::AsyncSource};
use zenpixels::{ConvertOptions, PixelFormat};
use zenpixels_convert::{OutputProfile, cms::PluggableCms};
use zencodecs::checked_transcode::{Error, Stage, TranscodeRequest};`;
const surface=[imports,
  declarations(read(file),{scope:"impl<'a> TranscodeRequest<'a> {",methods}).code,
  declarations(read(asyncFile),{scope:"impl TranscodeRequest<'_> {",methods:['transcode_async','transcode_async_with']}).code,
  declarations(read(file),{scope:'impl Error {',methods:['stage','report']}).code,
].join('\n\n')+'\n';
await writeFile(new URL('integration-api.json',import.meta.url),JSON.stringify({
  repository:'imazen/zenpipe',revision,path:file,related_paths:[asyncFile],
  features:'checked-transcode-async',surface,sha256:createHash('sha256').update(surface).digest('hex'),
},null,2)+'\n');
