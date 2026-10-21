// Targeted regressions against the real RedEngine simulation; no parallel rules model.
// node examples/2d/prism-fall/verify.js [path/to/red_engine2]
const fs=require('fs'),path=require('path'),os=require('os'),cp=require('child_process');
const engine=process.argv[2]||process.env.RED_ENGINE_BIN;
if(!engine)throw Error('Pass the current red_engine2 executable as the first argument.');
const original=JSON.parse(fs.readFileSync(path.join(__dirname,'../prism-fall.game2d.json'),'utf8'));
const dir=fs.mkdtempSync(path.join(os.tmpdir(),'prism-fall-check-'));
const S=(v,n)=>({set:[v,n]}),P=a=>({press:a}),B=id=>({button:id}),V=(v,n,op='eq')=>({var:v,[op]:n});
const encode=a=>a.reduce((n,c,x)=>n+c*8**x,0);
const gap=(xs,c=6)=>encode(Array.from({length:10},(_,x)=>xs.includes(x)?0:c));
let passed=0;
function test(name,setup,script,expect,extra=[]){
 const g=structuredClone(original);g.vars.phase=1;g.rules.unshift({id:'fixture',when:{start:true},do:[...Object.entries(setup).map(([k,v])=>S(k,v)),{apply:'shape'},S('dirty',1),{emit:'ghost'},...extra]});
 g.checks.scenarios=[{name,seed:41,max_seconds:60,script,expect}];
 const p=path.join(dir,'fixture.game2d.json');fs.writeFileSync(p,JSON.stringify(g));
 const r=cp.spawnSync(engine,['sim',p],{encoding:'utf8',maxBuffer:4e6});
 if(r.status!==0){console.error('FAIL '+name+'\n'+r.stdout+'\n'+r.stderr);process.exitCode=1;return;}
 passed++;console.log('PASS '+name);
}
test('four adjacent rows clear and earn a perfect clear',{
 piece:1,rot:1,x:2,y:1,...Object.fromEntries([18,19,20,21].map(r=>['r'+r,gap([4])]))},[P('action')],
 [V('cleared',4),V('lines',4),V('perfect',1),V('r21',0),V('score',2800,'gte'),V('b2b',1)]);
test('a single row clears', {piece:1,rot:0,x:3,y:1,r21:gap([3,4,5,6])},[P('action')], [V('cleared',1),V('lines',1),V('perfect',1)]);
test('two adjacent rows clear', {piece:2,rot:0,x:3,y:1,r20:gap([4,5]),r21:gap([4,5])},[P('action')], [V('cleared',2),V('lines',2),V('perfect',1)]);
test('three rows clear while preserving the fourth mino', {piece:1,rot:1,x:2,y:1,...Object.fromEntries([19,20,21].map(r=>['r'+r,gap([4])]))},[P('action')], [V('cleared',3),V('r21',8**4),V('perfect',0)]);
test('row compaction preserves the colour and order of remaining cells', {piece:1,rot:1,x:2,y:1,r21:gap([4]),r20:6*8},[P('action')], [V('cleared',1),V('r21',6*8+8**4)]);
test('I piece uses its two-cell SRS wall kick', {piece:1,rot:1,x:-2,y:10},[P('up')], [V('rot',2),V('x',0),V('rotok',1)]);
test('T piece uses its one-cell SRS wall kick', {piece:3,rot:1,x:-1,y:10},[P('up')], [V('rot',2),V('x',0),V('rotok',1)]);
test('T piece kicks upward off the floor', {piece:3,rot:0,x:3,y:20},[P('up')], [V('rot',1),V('x',2),V('y',19)]);
const blocked={piece:3,rot:0,x:3,y:18};
for(let r=15;r<22;r++)blocked['r'+r]=gap([]);
blocked.r18-=6*8**4;blocked.r19-=6*(8**3+8**4+8**5);
test('rotation with all five kicks blocked rolls back',blocked,[P('up')],[V('rot',0),V('x',3),V('y',18),V('rotok',0)]);
test('reverse rotation follows the reverse kick path', {piece:3,rot:0,x:3,y:10},[B('rotate-ccw')], [V('rot',3),V('rotok',1)]);
test('a filled hold swaps once and rejects repeated swaps', {piece:3,held:1},[B('hold'),B('hold')], [V('piece',1),V('held',3),V('holdused',1),V('phase',1)]);
test('a grounded piece waits for its lock delay', {piece:2,x:3,y:20},[{wait:.3}], [V('pieces',0),V('phase',1)]);
test('pausing on the lock boundary freezes the piece', {piece:2,x:3,y:20,grounded:1,locktime:.49},[P('pause'),{wait:.3}], [V('pieces',0),V('locktime',.49),V('paused',1)]);
test('a grounded piece locks after half a second', {piece:2,x:3,y:20},[{wait:.6}], [V('pieces',1),V('r21',2*8**4+2*8**5)]);
test('15 resets cap lock-delay stalling', {piece:2,x:3,y:20,resets:15,locktime:.48},[P('left'),{wait:.1}], [V('pieces',1)]);
test('a soft drop scores and descends faster', {piece:2,x:3,y:1},[{hold:['down'],seconds:.25}], [V('y',5,'gte'),V('score',4,'gte')]);
test('a blocked spawn ends marathon', {piece:2,x:3,y:20,r2:gap([0])},[P('action')], [V('phase',2)]);
test('zen rescues a blocked well and keeps the score', {mode:2,piece:2,x:3,y:20,r2:gap([0]),score:1234},[P('action'),{wait:.1}], [V('phase',1),V('r2',0),V('score',1234,'gte'),V('zenbest',1234,'gte')]);
test('a sprint finishes at forty lines and records its time', {mode:1,piece:1,rot:0,x:3,y:1,lines:39,r21:gap([3,4,5,6]),elapsed:62},[P('action')], [V('phase',3),V('lines',40),V('sprintbest',62,'gte')]);
test('difficult clears receive the back-to-back bonus', {piece:1,rot:1,x:2,y:1,b2b:1,...Object.fromEntries([18,19,20,21].map(r=>['r'+r,gap([4])]))},[P('action')], [V('award',1200),V('b2b',1)]);
test('consecutive clears receive combo points', {piece:1,rot:0,x:3,y:1,combo:1,r21:gap([3,4,5,6])},[P('action')], [V('award',200),V('combo',2)]);
test('T-spin corner detection awards a zero-line spin', {piece:3,rot:0,x:3,y:19,lastrotate:1,r19:6*(8**3+8**5),r21:6*(8**3+8**5)},[P('action')], [V('spin',1),V('award',400)]);
// Test seven-bag boundaries by requesting successive draws without locking pieces.
{
 const g=structuredClone(original);g.vars.phase=1;g.vars.seen=0;g.vars.nseen=0;g.vars.duplicates=0;g.vars.sample=0;
 g.rules.unshift({id:'fixture-start',when:{start:true},do:[{emit:'fill'}]});
 g.ui.push({button:{id:'sample',label:'',at:[-10,-10],size:[1,1],do:[{emit:'spawn'}]}});
 // Observe every draw, including the initial five queued pieces; reset at each bag boundary.
 const take=/^(initial[0-4]|queue|held-queue)-store$/;
 const result=[];
 for(const r of g.rules){
  if(take.test(r.id))result.push({id:'watch-'+r.id,when:r.when,do:[S('sample','floor(seen/('+Array.from({length:7},(_,i)=>`${2**i}*(drawn==${i+1})`).join('+')+'))%2'),{add:['duplicates','sample']},{add:['seen',Array.from({length:7},(_,i)=>`${2**i}*(drawn==${i+1})`).join('+')]},{add:['nseen',1]},S('seen','seen*(nseen%7!=0)')]});
  result.push(r);
 }
 g.rules=result;g.checks.scenarios=Array.from({length:12},(_,i)=>({name:'seven-bag seed '+i,seed:i+1,max_seconds:60,script:Array.from({length:70},()=>B('sample')),expect:[V('duplicates',0),V('nseen',76)]}));
 const p=path.join(dir,'bag.game2d.json');fs.writeFileSync(p,JSON.stringify(g));const r=cp.spawnSync(engine,['sim',p],{encoding:'utf8',maxBuffer:4e6});
 if(r.status){console.error(r.stdout,r.stderr);process.exitCode=1;}else{passed+=12;console.log('PASS seven-bag: 912 draws across twelve seeds');}
}
console.log(`${passed} targeted regression scenarios passed; fixtures in ${dir}`);
