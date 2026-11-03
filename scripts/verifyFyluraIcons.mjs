import fs from 'node:fs';
import path from 'node:path';
const build=process.argv[2]?path.resolve(process.argv[2]):path.resolve('build');
const ico=fs.readFileSync(path.join(build,'icon.ico'));
if(ico.readUInt16LE(0)!==0||ico.readUInt16LE(2)!==1||ico.readUInt16LE(4)!==7)throw Error('ICO header');
const expected=[16,24,32,48,64,128,256];
for(let i=0;i<expected.length;i++){
 const o=6+i*16,s=ico[o]||256,n=ico.readUInt32LE(o+8),pos=ico.readUInt32LE(o+12);
 if(s!==expected[i]||ico[o+1]!==ico[o]||pos+n>ico.length)throw Error(`ICO frame ${i}`);
 const p=ico.subarray(pos,pos+n);
 if(p.subarray(1,4).toString('ascii')!=='PNG'||p.readUInt32BE(16)!==s||p.readUInt32BE(20)!==s)throw Error(`ICO ${s} PNG`);
 if(s<=64&&!fs.readFileSync(path.join(build,'brand','small-sizes',`Fylura_${s}.png`)).equals(p))throw Error(`ICO ${s} not native`);
}
const icns=fs.readFileSync(path.join(build,'icon.icns'));
if(icns.toString('ascii',0,4)!=='icns'||icns.readUInt32BE(4)!==icns.length)throw Error('ICNS header');
let pos=8,count=0;
while(pos<icns.length){
 const n=icns.readUInt32BE(pos+4);
 if(n<=8||pos+n>icns.length||icns.toString('ascii',pos+9,pos+12)!=='PNG')throw Error('ICNS slot');
 pos+=n;count++;
}
if(count!==11||pos!==icns.length)throw Error('ICNS count');
const png=fs.readFileSync(path.join(build,'icon.png'));
if(png.readUInt32BE(16)!==1024||png.readUInt32BE(20)!==1024)throw Error('Master PNG size');
console.log('Fylura PNG/ICO/ICNS content and native small frames: PASS');
