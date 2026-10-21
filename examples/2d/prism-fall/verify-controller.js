// Exercise the engine's actual D-pad mapping and controller module, without a device.
// node examples/2d/prism-fall/verify-controller.js /path/to/engine/build/debug/deps
// Requires rustc and an already-built glam rlib. No engine source is changed.
const fs=require('fs'),path=require('path'),os=require('os'),cp=require('child_process');
const deps=process.argv[2];
if(!deps)throw Error('Pass the engine build/debug/deps directory.');
const root=path.resolve(__dirname,'../../..');
const source=fs.readFileSync(path.join(root,'src/play2d.rs'),'utf8');
const a=source.indexOf('const PAD_ACTIONS:');
const b=source.indexOf('\n}',source.indexOf('fn pad_actions(',a))+2;
if(a<0||b<2)throw Error('The engine pad_actions entry point moved; inspect its current source.');
const program=`#![allow(dead_code)]
#[path=${JSON.stringify(path.join(root,'src/controller.rs'))}] mod controller;
use controller::button;
${source.slice(a,b)}
#[test] fn all_four_dpad_directions_are_independent() {
 for (bit,index) in [(button::LEFT,0),(button::RIGHT,1),(button::UP,2),(button::DOWN,3)] {
  let a=pad_actions(&controller::Sample{held:bit,..Default::default()});
  for (i,held) in a.into_iter().enumerate(){assert_eq!(held,i==index,"D-pad {index}, action {i}");}
 }
}
#[test] fn dpad_diagonals_and_face_buttons_work_together() {
 let a=pad_actions(&controller::Sample{held:button::LEFT|button::DOWN|button::JUMP|button::INTERACT|button::PAUSE,..Default::default()});
 assert_eq!(a,[true,false,false,true,true,true,true]);
}
#[test] fn dpad_neutral_and_released_directions_stop_immediately(){
 assert_eq!(pad_actions(&controller::Sample::default()),[false;7]);
 let mut gate=controller::Gate::default();gate.sample(Default::default(),true);
 let held=controller::Sample{held:button::RIGHT,..Default::default()};
 assert!(pad_actions(&gate.sample(held,true))[1]);
 assert!(!pad_actions(&gate.sample(Default::default(),true))[1]);
 gate.reset();assert!(!pad_actions(&gate.sample(held,true))[1]);
}
`;
const temp=fs.mkdtempSync(path.join(os.tmpdir(),'prism-controller-'));
const src=path.join(temp,'check.rs'),exe=path.join(temp,'check'+(process.platform==='win32'?'.exe':''));
fs.writeFileSync(src,program);
let built=false,details='';
for(const lib of fs.readdirSync(deps).filter(n=>/^libglam-.*\.rlib$/.test(n))){
 const c=cp.spawnSync('rustc',['--edition','2021','--test',src,'--extern','glam='+path.join(deps,lib),'-L','dependency='+deps,'-o',exe],{encoding:'utf8'});
 if(c.status===0){built=true;break;}details=c.stderr||String(c.error);
}
if(!built)throw Error('No compatible built glam library: '+details);
const result=cp.spawnSync(exe,[],{stdio:'inherit'});
process.exitCode=result.status||0;
