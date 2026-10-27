// Read source declarations without duplicating signatures in editorial content.
// This scanner handles the named Rust items used by the guide; it is not a Rust parser.
export function maskRust(source) {
  const chars=source.split(''); // Keep UTF-16 offsets aligned, including emoji in docs.
  let i=0;
  const blank=(start,end)=>{for(let n=start;n<end;n++) if(chars[n]!=='\n') chars[n]=' ';};
  while(i<source.length) {
    if(source.startsWith('//',i)) {const end=source.indexOf('\n',i);const n=end<0?source.length:end;blank(i,n);i=n;continue;}
    if(source.startsWith('/*',i)) {
      const start=i;i+=2;let depth=1;
      while(depth&&i<source.length) {
        if(source.startsWith('/*',i)){depth++;i+=2;}
        else if(source.startsWith('*/',i)){depth--;i+=2;} else i++;
      }
      if(depth) throw Error('Unterminated Rust comment');blank(start,i);continue;
    }
    const raw=source.slice(i).match(/^(?:b|c)?r(#{0,32})"/);
    if(raw) {const end=source.indexOf('"'+raw[1],i+raw[0].length);if(end<0)throw Error('Unterminated raw string');const n=end+1+raw[1].length;blank(i,n);i=n;continue;}
    if(source[i]==='"') {
      const start=i++;while(i<source.length){if(source[i]==='\\'){i+=2;continue;}if(source[i++]==='"')break;}blank(start,i);continue;
    }
    const char=source.slice(i).match(/^'(?:\\(?:u\{[\da-fA-F]+\}|x[\da-fA-F]{2}|.)|[^'\\\n])'/);
    if(char){blank(i,i+char[0].length);i+=char[0].length;continue;}
    i++;
  }
  return chars.join('');
}
const deindent=text=>{
  const lines=text.replace(/^\n/,'').trimEnd().split('\n');
  const indent=Math.min(...lines.filter(x=>x.trim()).map(x=>x.match(/^ */)[0].length));
  return lines.map(x=>x.slice(indent)).join('\n');
};
function closeBrace(mask,open) {
  let depth=1;
  for(let i=open+1;i<mask.length;i++){if(mask[i]==='{')depth++;if(mask[i]==='}'&&!--depth)return i;}
  throw Error('Unclosed source scope');
}
export function declarations(source,{scope,methods,associated=true,item=false}) {
  if(item&&(!scope||methods.length))throw Error('A complete source item needs a scope and no method selection');
  const mask=maskRust(source);
  let start=0,end=source.length,header='';
  if(scope) {
    const at=mask.indexOf(scope);
    if(at<0||mask.indexOf(scope,at+scope.length)>=0)throw Error(`Scope must match once: ${scope}`);
    const open=mask.indexOf('{',at);end=closeBrace(mask,open);start=open+1;
    if(item)return {code:deindent(source.slice(at,end+1)),line:source.slice(0,at).split('\n').length};
    header=source.slice(at,open).trim();
  }
  const part=mask.slice(start,end);
  const depth=new Int32Array(part.length+1);
  for(let i=0;i<part.length;i++)depth[i+1]=depth[i]+(part[i]==='{'?1:part[i]==='}'?-1:0);
  const items=[],available=[];
  for(const m of part.matchAll(/^[ \t]*(?:(?:pub(?:\([^\n)]*\))?\s+)?(?:(?:async|const)\s+)*fn\s+(\w+)\b|type\s+(\w+)\b)/gm)) {
    if(depth[m.index]!==0)continue;
    const name=m[1]||m[2];const type=Boolean(m[2]);
    const offset=start+m.index;
    let stop=offset;
    while(stop<end&&mask[stop]!=='{'&&mask[stop]!==';')stop++;
    if(stop===end)throw Error(`Missing declaration terminator: ${name}`);
    if(!type)available.push({name,required:mask[stop]===';'});
    if(type?(!scope||!associated):!methods.includes(name))continue;
    const code=deindent(source.slice(offset,stop)).trimEnd().replace(/,$/,'')+';';
    items.push({name,type,code,line:source.slice(0,offset).split('\n').length});
  }
  for(const name of methods)if(items.filter(x=>!x.type&&x.name===name).length!==1)throw Error(`Expected exactly one ${scope||'free function'}::${name}`);
  const ordered=[...items.filter(x=>x.type),...methods.map(name=>items.find(x=>!x.type&&x.name===name))];
  return {code:scope?`${header} {\n${ordered.map(x=>x.code.split('\n').map(l=>'    '+l).join('\n')).join('\n\n')}\n}`:ordered.map(x=>x.code).join('\n\n'),line:ordered.find(x=>!x.type).line,omittedMethods:available.filter(x=>!methods.includes(x.name)).map(x=>x.name),requiredMethods:available.filter(x=>x.required).map(x=>x.name)};
}
export function example(source,name) {
  if(!/^[a-z0-9_-]+$/.test(name))throw Error(`Invalid example name: ${name}`);
  const starts=[...source.matchAll(new RegExp(`^[ \\t]*// EXAMPLE: ${name}\\n`,'gm'))];
  const ends=[...source.matchAll(new RegExp(`^[ \\t]*// END EXAMPLE: ${name}(?:\\n|$)`,'gm'))];
  if(starts.length!==1||ends.length!==1||ends[0].index<starts[0].index)throw Error(`Example scope must be unique: ${name}`);
  const start=starts[0].index+starts[0][0].length;
  const body=deindent(source.slice(start,ends[0].index));
  const opening=new RegExp(`^[ \\t]*// ANCHOR: ${name}\\n`,'m');
  const closing=new RegExp(`^[ \\t]*// END: ${name}(?:\\n|$)`,'m');
  const a=opening.exec(body),b=closing.exec(body);
  if(!a||!b||b.index<a.index)throw Error(`Missing example focus: ${name}`);
  const above=body.slice(0,a.index),focus=body.slice(a.index+a[0].length,b.index),below=body.slice(b.index+b[0].length);
  return {above,focus,below,full:above+focus+below,line:source.slice(0,start).split('\n').length};
}

// Public-API snapshots put method generics between the name and parameters.
export function snapshotMethods(snapshot,path) {
  const prefix=`${path}::`;
  return snapshot.split('\n').flatMap(line=>{
    const declaration=line.replace(/^pub (?:(?:async|const) )*fn /,'');
    if(declaration===line||!declaration.startsWith(prefix))return [];
    const name=declaration.slice(prefix.length).match(/^(\w+)(?=[<(])/);
    return name?[name[1]]:[];
  });
}
