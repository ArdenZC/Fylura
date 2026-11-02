import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';
// Fylura source of truth: orb at upper left, glass pane at lower right.
// Native small-size frames are optically redrawn, never 1024px downscales.
const repo=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const argv=process.argv.slice(2), idx=argv.indexOf('--output');
if(idx!==-1&&!argv[idx+1])throw Error('--output requires a path');
const build=idx===-1?path.join(repo,'build'):path.resolve(argv[idx+1]);
const icoSizes=[16,24,32,48,64,128,256];
const icnsSlots=[['icp4',16],['icp5',32],['icp6',64],['ic07',128],['ic08',256],['ic09',512],['ic10',1024],['ic11',32],['ic12',64],['ic13',256],['ic14',512]];
const sizes=[...new Set([...icoSizes,...icnsSlots.map(([,s])=>s),1024])];
const clamp=(x,a=0,b=1)=>Math.min(b,Math.max(a,x));
const mix=(a,b,t)=>a+(b-a)*clamp(t);
const between=(a,b,t)=>[0,1,2].map(i=>mix(a[i],b[i],t));
function over(dst,src,alpha){
 const sa=clamp(alpha),da=dst[3]/255,outA=sa+da*(1-sa);
 if(!outA)return [0,0,0,0];
 const k=sa/outA,j=da*(1-sa)/outA;
 return [src[0]*k+dst[0]*j,src[1]*k+dst[1]*j,src[2]*k+dst[2]*j,255*outA];
}
function pixel(u,v,size){
 const small=size<=64,edge=1024/size,aa=Math.max(1,edge),cover=d=>clamp(.5-d/aa);
 let p=[0,0,0,0];
 const r=224,w=956,h=956,qx=Math.abs(u-512)-(w/2-r),qy=Math.abs(v-512)-(h/2-r);
 const d=Math.hypot(Math.max(qx,0),Math.max(qy,0))+Math.min(Math.max(qx,qy),0)-r;
 const tile=cover(d);
 if(!tile)return p;
 p=over(p,between([255,255,255],[224,236,254],clamp((u*.45+v*.85)/1024)),tile);
 const dx=u-440.5,dy=v-454.5,dist=Math.hypot(dx,dy),sphere=cover(dist-272.5)*tile;
 if(sphere){
  const g=clamp((.40*dx-.88*dy+270)/520);
  let c=g<.52?between([10,85,247],[20,139,249],g/.52):between([20,139,249],[69,222,224],(g-.52)/.48);
  if(!small)c=c.map(x=>mix(x,255,clamp((300-Math.hypot(dx+135,dy+195))/300)*.07));
  p=over(p,c,sphere);
 }
 const pw=508,ph=508,px=392,py=386,pr=116;
 const qrx=Math.abs(u-(px+pw/2))-(pw/2-pr),qry=Math.abs(v-(py+ph/2))-(ph/2-pr);
 const pd=Math.hypot(Math.max(qrx,0),Math.max(qry,0))+Math.min(Math.max(qrx,qry),0)-pr;
 const pane=cover(pd)*tile;
 if(pane){
  const slant=clamp(((u-px)+(v-py)*.52)/(pw*1.6));
  p=over(p,between([229,248,255],[217,234,255],slant),pane*(small?(size<=24?.93:.86):.78));
  const border=small?(size<=24?1.25:1.0)*edge:Math.max(3.8,edge*.7);
  const borderCov=clamp((pd+border)/aa+.5)-clamp(pd/aa+.5);
  p=over(p,[255,255,255],borderCov*(small?.97:.86)*tile);
  if(!small)p=over(p,[255,255,255],pane*(1-clamp((u-px)/pw)*clamp((v-py)/ph))*.075);
 }
 return p;
}
function render(size){
 const raw=Buffer.alloc((size*4+1)*size),scale=1024/size,n=size<=64?2:1;let k=0;
 for(let y=0;y<size;y++){
  raw[k++]=0;
  for(let x=0;x<size;x++){
   const sum=[0,0,0,0];
   for(let j=0;j<n;j++)for(let i=0;i<n;i++){
    const p=pixel((x+(i+.5)/n)*scale,(y+(j+.5)/n)*scale,size);
    for(let t=0;t<4;t++)sum[t]+=p[t];
   }
   for(let t=0;t<4;t++)raw[k++]=Math.round(sum[t]/(n*n));
  }
 }
 return png(size,raw);
}
const ct=Array.from({length:256},(_,i)=>{let c=i;for(let k=0;k<8;k++)c=(c&1)?0xedb88320^(c>>>1):c>>>1;return c>>>0;});
function crc(b){let c=0xffffffff;for(const x of b)c=ct[(c^x)&255]^(c>>>8);return(c^0xffffffff)>>>0;}
function chunk(tag,payload){
 const len=Buffer.alloc(4),sum=Buffer.alloc(4),t=Buffer.from(tag,'ascii');
 len.writeUInt32BE(payload.length);sum.writeUInt32BE(crc(Buffer.concat([t,payload])));
 return Buffer.concat([len,t,payload,sum]);
}
function png(s,raw){
 const h=Buffer.alloc(13);h.writeUInt32BE(s,0);h.writeUInt32BE(s,4);h[8]=8;h[9]=6;
 return Buffer.concat([Buffer.from([137,80,78,71,13,10,26,10]),chunk('IHDR',h),chunk('IDAT',zlib.deflateSync(raw,{level:9})),chunk('IEND',Buffer.alloc(0))]);
}
function ico(m){
 const h=Buffer.alloc(6+16*icoSizes.length);h.writeUInt16LE(1,2);h.writeUInt16LE(icoSizes.length,4);
 let off=h.length;const payload=[];
 for(const [i,s] of icoSizes.entries()){
  const b=m.get(s),o=6+i*16;h[o]=s===256?0:s;h[o+1]=s===256?0:s;
  h.writeUInt16LE(1,o+4);h.writeUInt16LE(32,o+6);h.writeUInt32LE(b.length,o+8);h.writeUInt32LE(off,o+12);
  off+=b.length;payload.push(b);
 }
 return Buffer.concat([h,...payload]);
}
function icns(m){
 const blocks=icnsSlots.map(([type,s])=>{
  const b=m.get(s),h=Buffer.alloc(8);h.write(type,0,4,'ascii');h.writeUInt32BE(b.length+8,4);return Buffer.concat([h,b]);
 });
 const h=Buffer.alloc(8);h.write('icns',0,4,'ascii');h.writeUInt32BE(8+blocks.reduce((n,b)=>n+b.length,0),4);
 return Buffer.concat([h,...blocks]);
}
fs.mkdirSync(build,{recursive:true});
const frames=new Map(sizes.map(s=>[s,render(s)]));
fs.writeFileSync(path.join(build,'icon.png'),frames.get(1024));
const icoData=ico(frames);
for(const name of ['icon.ico','installerIcon.ico','uninstallerIcon.ico','installerHeaderIcon.ico'])fs.writeFileSync(path.join(build,name),icoData);
fs.writeFileSync(path.join(build,'icon.icns'),icns(frames));
const qa=path.join(build,'brand','small-sizes');fs.mkdirSync(qa,{recursive:true});
for(const s of [16,24,32,48,64])fs.writeFileSync(path.join(qa,`Fylura_${s}.png`),frames.get(s));
console.log('Fylura PNG, ICO and ICNS generated; 16–64 px are optical native-size rasters');
