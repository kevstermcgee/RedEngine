// Prism Fall: deterministic falling-block rules, sprites and interface for RedEngine.
// node examples/2d/prism-fall/build.js [output.game2d.json]
const fs = require('fs');
const path = require('path');
const OUT = process.argv[2] || path.join(__dirname, '../prism-fall.game2d.json');
const W=300, H=644, BX=0, BY=44, CELL=30;
const colors=['#000000','#51d9ed','#f4ce62','#ba94ef','#75d6a4','#ef8394','#799ef1','#efa66b'];
const names=['','I','O','T','S','Z','J','L'];
const shapes={
  1:[[[0,1],[1,1],[2,1],[3,1]],[[2,0],[2,1],[2,2],[2,3]],[[0,2],[1,2],[2,2],[3,2]],[[1,0],[1,1],[1,2],[1,3]]],
  2:Array(4).fill([[1,0],[2,0],[1,1],[2,1]]),
  3:[[[1,0],[0,1],[1,1],[2,1]],[[1,0],[1,1],[2,1],[1,2]],[[0,1],[1,1],[2,1],[1,2]],[[1,0],[0,1],[1,1],[1,2]]],
  4:[[[1,0],[2,0],[0,1],[1,1]],[[1,0],[1,1],[2,1],[2,2]],[[1,1],[2,1],[0,2],[1,2]],[[0,0],[0,1],[1,1],[1,2]]],
  5:[[[0,0],[1,0],[1,1],[2,1]],[[2,0],[1,1],[2,1],[1,2]],[[0,1],[1,1],[1,2],[2,2]],[[1,0],[0,1],[1,1],[0,2]]],
  6:[[[0,0],[0,1],[1,1],[2,1]],[[1,0],[2,0],[1,1],[1,2]],[[0,1],[1,1],[2,1],[2,2]],[[1,0],[1,1],[0,2],[1,2]]],
  7:[[[2,0],[0,1],[1,1],[2,1]],[[1,0],[1,1],[1,2],[2,2]],[[0,1],[1,1],[2,1],[0,2]],[[0,0],[1,0],[1,1],[1,2]]]
};
const g={game2d:1,id:'prism-fall',title:'Prism Fall',description:'A polished Tetris-style arcade: seven-bag pieces, SRS wall kicks, hold, ghost, two-piece preview, combos and three modes.',
  capabilities:{presentation:'2d',platforms:['windows','linux'],networking:'offline',input:['keyboard','mouse','gamepad'],persistence:['progress','settings']},
  view:{width:W,height:H,background:'#0d0d0d',scale:'fit'},icon:'block3',sprites:{},sounds:{},vars:{},persist:['best','sprintbest','zenbest'],prefabs:{},scene:[],ui:[],effects:{},rules:[],pause_if:'phase != 1 || paused',checks:{scenarios:[]}};
const S=(v,n)=>({set:[v,n]}), A=(v,n)=>({add:[v,n]}), E=n=>({emit:n}), FX=n=>({apply:n}), play=n=>({play:n});
function rule(id,when,cond,actions){g.rules.push({id,when,...(cond?{if:cond}:{}),do:actions});}
const ev=(id,event,cond,actions)=>rule(id,{event},cond,actions);
const tick=(id,cond,actions)=>rule(id,{every:1/60},'phase==1 && !paused'+(cond?' && ('+cond+')':''),actions);
const press=(id,action,actions)=>rule(id,{press:action},'phase == 1 && !paused',actions);
function effect(id,actions){g.effects[id]={do:actions};}
const vars={phase:0,mode:0,paused:0,x:3,y:1,rot:0,piece:3,held:0,holdused:0,score:0,lines:0,stage:1,best:0,sprintbest:0,zenbest:0,
  elapsed:0,minutes:0,seconds:0,hundredths:0,gravity:0,locktime:0,resets:0,grounded:0,dirty:1,ghosty:1,cx:0,cy:0,hit:0,oldrot:0,turn:1,rotok:0,
  lastrotate:0,spin:0,cleared:0,combo:-1,b2b:0,difficult:0,award:0,perfect:0,popup:0,danger:0,flash:0,bagleft:0,ticket:0,drawn:0,
  lrepeat:0,rrepeat:0,moveok:0,oldy:0,pieces:0,clear_timer:0,keep:0,runbest:0,lockpercent:0,pps:0};
for(let i=0;i<22;i++)vars['r'+i]=0;
for(let i=0;i<5;i++)vars['q'+i]=i+1;
for(let i=1;i<=7;i++)vars['bag'+i]=0;
for(let i=0;i<4;i++) for(const p of ['dx','dy','tx','ty']) vars[p+i]=0;
Object.assign(g.vars,vars);
// Each row is ten base-eight digits: 0 empty, 1..7 colour. All integers fit in 30 bits.
const pow=x=>'('+Array.from({length:10},(_,i)=>`${8**i}*((${x})==${i})`).join('+')+')';
const row=y=>'('+Array.from({length:22},(_,i)=>`r${i}*((${y})==${i})`).join('+')+')';
const occupied=(x,y)=>`((${x})<0 || (${x})>9 || (${y})>=22 || ((${y})>=0 && floor(${row(y)}/${pow(x)})%8>0))`;
effect('shape',Array.from({length:4},(_,b)=>[0,1].map(axis=>S((axis?'dy':'dx')+b,
  Object.keys(shapes).flatMap(p=>shapes[p].map((s,r)=>`${s[b][axis]}*(piece==${p} && rot==${r})`)).join('+')))).flat());
effect('probe',[...Array.from({length:4},(_,i)=>[S('tx'+i,`cx+dx${i}`),S('ty'+i,`cy+dy${i}`)]).flat(),
  S('hit',Array.from({length:4},(_,i)=>occupied('tx'+i,'ty'+i)).join(' || '))]);
effect('reset_lock',[S('locktime','locktime*(resets>=15)'),A('resets','grounded*(resets<15)'),S('dirty',1)]);
effect('spawn_position',[S('x',3),S('y',1),S('rot',0),S('lastrotate',0),S('resets',0),S('locktime',0),S('gravity',0),S('dirty',1),FX('shape'),S('cx','x'),S('cy','y'),FX('probe')]);
effect('newgame',[...Object.entries(vars).filter(([k])=>!['best','sprintbest','zenbest','mode'].includes(k)).map(([k,v])=>S(k,v)),S('phase',1),E('fill')]);
// A true shuffled seven-bag. Each draw removes one of the remaining pieces uniformly.
function drawRules(event,label,target){
  ev(label+'-refill',event,'bagleft == 0',[S('bagleft',7),...Array.from({length:7},(_,i)=>S('bag'+(i+1),0))]);
  ev(label+'-ticket',event,null,[S('ticket','random(1,bagleft)'),S('drawn',0)]);
  for(let i=1;i<=7;i++){
    ev(label+'-scan'+i,event,`drawn==0 && bag${i}==0`,[A('ticket',-1)]);
    ev(label+'-take'+i,event,`drawn==0 && ticket==0 && bag${i}==0`,[S('drawn',i),S('bag'+i,1),A('bagleft',-1)]);
  }
  ev(label+'-store',event,null,[S(target,'drawn')]);
}
for(let i=0;i<5;i++)drawRules('fill','initial'+i,'q'+i);
ev('filled','fill',null,[E('spawn')]);
ev('queue-pop','spawn',null,[S('piece','q0'),...Array.from({length:4},(_,i)=>S('q'+i,'q'+(i+1))),S('holdused',0)]);
drawRules('spawn','queue','q4');
ev('spawn-place','spawn',null,[FX('spawn_position')]);
ev('spawn-blocked','spawn','hit',[E('gameover')]);
// Events are also the input API used by the regression fixtures.
press('left-key','left',[S('lrepeat',.15),E('left')]);
press('right-key','right',[S('rrepeat',.15),E('right')]);
press('rotate-key','up',[S('turn',1),E('rotate')]);
press('drop-key','action',[E('drop')]);
press('hold-key','secondary',[E('hold')]);
rule('pause-key',{press:'pause'},'phase==1',[S('paused','1-paused')]);
rule('menu-up',{press:'up'},'phase==0',[S('mode','(mode+2)%3')]);
rule('menu-down',{press:'down'},'phase==0',[S('mode','(mode+1)%3')]);
rule('menu-select',{press:'action'},'phase==0',[FX('newgame')]);
rule('pad-resume',{press:'action'},'phase==1 && paused',[S('paused',0)]);
rule('pad-retry',{press:'action'},'phase>=2',[FX('newgame')]);
tick('timers','phase==1 && !paused',[A('elapsed',1/60),A('gravity',1/60),A('lrepeat',-1/60),A('rrepeat',-1/60),S('popup','max(0,popup-1/60)'),S('flash','max(0,flash-1/60)'),
  S('minutes','floor(elapsed/60)'),S('seconds','floor(elapsed)%60'),S('hundredths','floor(elapsed*100)%100'),S('pps','pieces/max(1,elapsed)')]);
tick('left-release','!held_left',[S('lrepeat',0)]);
tick('right-release','!held_right',[S('rrepeat',0)]);
tick('left-repeat','held_left && !held_right && lrepeat<=0',[S('lrepeat',.045),E('left')]);
tick('right-repeat','held_right && !held_left && rrepeat<=0',[S('rrepeat',.045),E('right')]);
for(const [name,dx]of [['left',-1],['right',1]]){
  ev(name+'-probe',name,'phase==1 && !paused',[S('cx',`x+${dx}`),S('cy','y'),FX('probe'),S('moveok','!hit')]);
  ev(name+'-move',name,'moveok && phase==1 && !paused',[S('x','cx'),S('lastrotate',0),FX('reset_lock')]);
}
// SRS kick tests. Official y-up offsets are inverted for the engine's y-down screen.
const normal={ '0,1':[[0,0],[-1,0],[-1,1],[0,-2],[-1,-2]],'1,0':[[0,0],[1,0],[1,-1],[0,2],[1,2]],
 '1,2':[[0,0],[1,0],[1,-1],[0,2],[1,2]],'2,1':[[0,0],[-1,0],[-1,1],[0,-2],[-1,-2]],
 '2,3':[[0,0],[1,0],[1,1],[0,-2],[1,-2]],'3,2':[[0,0],[-1,0],[-1,-1],[0,2],[-1,2]],
 '3,0':[[0,0],[-1,0],[-1,-1],[0,2],[-1,2]],'0,3':[[0,0],[1,0],[1,1],[0,-2],[1,-2]]};
const ikicks={'0,1':[[0,0],[-2,0],[1,0],[-2,-1],[1,2]],'1,0':[[0,0],[2,0],[-1,0],[2,1],[-1,-2]],
 '1,2':[[0,0],[-1,0],[2,0],[-1,2],[2,-1]],'2,1':[[0,0],[1,0],[-2,0],[1,-2],[-2,1]],
 '2,3':[[0,0],[2,0],[-1,0],[2,1],[-1,-2]],'3,2':[[0,0],[-2,0],[1,0],[-2,-1],[1,2]],
 '3,0':[[0,0],[1,0],[-2,0],[1,-2],[-2,1]],'0,3':[[0,0],[-1,0],[2,0],[-1,2],[2,-1]]};
ev('rotation-begin','rotate','phase==1 && !paused',[S('oldrot','rot'),S('rot','(rot+turn+4)%4'),S('rotok',0),FX('shape')]);
for(let k=0;k<5;k++){
  const kick=axis=>Object.entries(normal).flatMap(([key,a])=>{
    const [o,n]=key.split(','); const q=ikicks[key];
    return [`${(axis?-1:1)*a[k][axis]}*(piece!=1 && piece!=2 && oldrot==${o} && rot==${n})`,`${(axis?-1:1)*q[k][axis]}*(piece==1 && oldrot==${o} && rot==${n})`];
  }).join('+');
  ev('kick-probe'+k,'rotate','!rotok && phase==1 && !paused',[S('cx','x+('+kick(0)+')'),S('cy','y+('+kick(1)+')'),FX('probe')]);
  ev('kick-accept'+k,'rotate','!rotok && !hit && phase==1 && !paused',[S('x','cx'),S('y','cy'),S('rotok',1),S('lastrotate',1),FX('reset_lock')]);
}
ev('rotation-rollback','rotate','!rotok && phase==1 && !paused',[S('rot','oldrot'),FX('shape')]);
ev('rotation-sound','rotate','rotok && phase==1 && !paused',[play('rotate')]);
ev('hold-begin','hold','!holdused && phase==1 && !paused',[S('keep','held'),S('held','piece'),S('holdused',1),play('hold')]);
ev('hold-empty','hold','holdused==1 && keep==0 && phase==1 && !paused',[S('keep',-1),E('spawn-held')]);
ev('hold-swap','hold','holdused==1 && keep>0 && phase==1 && !paused',[S('piece','keep'),S('keep',-1),FX('spawn_position'),E('hold-check')]);
ev('hold-blocked','hold-check','hit && phase==1 && !paused',[E('gameover')]);
// Mark the request consumed: a second hold must not reuse an earlier `keep` or `hit`.
ev('hold-finish','hold',null,[S('keep',-1)]);
ev('held-queue-pop','spawn-held',null,[S('piece','q0'),...Array.from({length:4},(_,i)=>S('q'+i,'q'+(i+1)))]);
drawRules('spawn-held','held-queue','q4');
ev('held-queue-place','spawn-held',null,[FX('spawn_position')]);
ev('held-queue-blocked','spawn-held','hit',[E('gameover')]);
// Fall, lock delay and a ghost computed with the exact same probe as movement.
tick('fall-request','gravity>=(max(0.05,0.8-(stage-1)*0.055)*(mode!=2)+0.8*(mode==2)) || (held_down && gravity>=0.035)',[S('gravity',0),E('fall')]);
ev('fall-probe','fall','phase==1 && !paused',[S('cx','x'),S('cy','y+1'),FX('probe')]);
ev('fall-move','fall','!hit && phase==1 && !paused',[A('y',1),A('score','held_down'),S('dirty',1)]);
tick('ghost-request','dirty',[S('dirty',0),E('ghost')]);
ev('ghost-begin','ghost',null,[S('ghosty','y'),S('cx','x'),S('cy','y+1'),FX('probe'),S('grounded','hit')]);
for(let j=0;j<22;j++){
  ev('ghost-probe'+j,'ghost',null,[S('cx','x'),S('cy','ghosty+1'),FX('probe')]);
  ev('ghost-advance'+j,'ghost','!hit',[A('ghosty',1)]);
}
tick('lock-clock','grounded',[A('locktime',1/60)]);
tick('lock-unset','!grounded',[S('locktime',0)]);
tick('lock-ready','locktime>=0.5',[E('lock')]);
ev('drop-place','drop','phase==1 && !paused',[S('oldy','y'),E('ghost'),E('drop-finish')]);
ev('drop-finish','drop-finish','phase==1 && !paused',[S('y','ghosty'),A('score','2*(y-oldy)'),play('drop'),E('lock')]);
ev('lock-begin','lock','phase==1 && !paused',[S('cleared',0),S('perfect',0),S('spin',`piece==3 && lastrotate && (${[[0,0],[2,0],[0,2],[2,2]].map(([x,y])=>occupied('x+'+x,'y+'+y)).join('+')})>=3`),A('pieces',1),play('lock')]);
ev('lock-out','lock','phase==1 && (y+dy0<2 || y+dy1<2 || y+dy2<2 || y+dy3<2)',[E('gameover')]);
for(let i=0;i<4;i++)for(let r=0;r<22;r++)
  ev('write'+i+'-'+r,'lock',`phase==1 && y+dy${i}==${r}`,[A('r'+r,`piece*${pow('x+dx'+i)}`)]);
// Scan from the top. Shifting the rows above cannot skip consecutive full rows below.
for(let r=0;r<22;r++){
 const full=Array.from({length:10},(_,x)=>`floor(r${r}/${8**x})%8>0`).join(' && ');
 ev('clear-row'+r,'lock',`phase==1 && ${full}`,[A('cleared',1),S('flash',.22),...Array.from({length:r},(_,i)=>S('r'+(r-i),'r'+(r-i-1))),S('r0',0)]);
}
ev('score-clear','lock','phase==1',[S('combo','(combo+1)*(cleared>0)-(cleared==0)'),S('difficult','cleared==4 || (spin && cleared>0)'),
 S('award','(100*(cleared==1)+300*(cleared==2)+500*(cleared==3)+800*(cleared==4))*(1-spin)+spin*(400+400*(cleared==1)+800*(cleared==2)+1200*(cleared==3))'),
 S('award','floor(award*(1+0.5*(difficult && b2b)))*stage+max(0,combo)*50*stage'),A('score','award'),A('lines','cleared'),
 S('popup','1.8*(cleared>0 || spin)'),S('b2b','difficult+(cleared==0)*b2b')]);
ev('perfect-clear','lock','cleared>0 && '+Array.from({length:22},(_,r)=>`r${r}==0`).join(' && '),[S('perfect',1),A('score','2000*stage')]);
ev('record-lock','lock','phase==1',[S('best','max(best,score*(mode==0))'),S('zenbest','max(zenbest,score*(mode==2))'),S('stage','1+floor(lines/10)')]);
ev('clear-sound','lock','cleared>0',[play('clear')]);
ev('tetris-sound','lock','cleared==4',[play('tetris')]);
ev('sprint-win','lock','phase==1 && mode==1 && lines>=40',[S('phase',3),S('sprintbest','elapsed*(sprintbest==0 || elapsed<sprintbest)+sprintbest*(sprintbest>0 && elapsed>=sprintbest)'),play('win')]);
ev('next-piece','lock','phase==1',[E('spawn')]);
ev('finish-over','gameover','phase==1',[S('phase',2),S('paused',0),play('over')]);
tick('records',null,[S('best','max(best,score*(mode==0))'),S('zenbest','max(zenbest,score*(mode==2))'),S('runbest','best*(mode==0)+zenbest*(mode==2)'),
 S('danger','r2>0 || r3>0 || r4>0 || r5>0'),S('lockpercent','min(100,locktime*200)')]);
// Zen gives a fresh well after a top-out and keeps the score; marathon/sprint end.
ev('zen-rescue','gameover','phase==2 && mode==2',[S('phase',1),...Array.from({length:22},(_,r)=>S('r'+r,0)),E('spawn')]);
// Teleport the four active blocks and their hollow landing indicators.
for(let p=1;p<=7;p++)ev('active-colour'+p,'spawn',`piece==${p}`,Array.from({length:4},(_,i)=>({become:{target:'id:a'+i,prefab:'block'+p}})));
for(const event of ['hold','spawn-held'])for(let p=1;p<=7;p++)ev(event+'-colour'+p,event,`piece==${p}`,Array.from({length:4},(_,i)=>({become:{target:'id:a'+i,prefab:'block'+p}})));
for(let i=0;i<4;i++){
 const vis=`(phase==1 && y+dy${i}>=2)`,gv=`(phase==1 && ghosty+dy${i}>=2)`;
 tick('draw-active'+i,null,[{teleport:{target:'id:a'+i,to:[`(${BX}+CELL*(x+dx${i})+15)*${vis}-500*!${vis}`.replaceAll('CELL',CELL),`${BY}+${CELL}*(y+dy${i}-2)+15`]}},
  {teleport:{target:'id:g'+i,to:[`(${BX}+${CELL}*(x+dx${i})+15)*${gv}-500*!${gv}`,`${BY}+${CELL}*(ghosty+dy${i}-2)+15`]}}]);
}
// Sound design: quiet mechanical taps, glassy rotations, warm rising clear chords.
function sound(id,hz,secs=.16,stage=.22){g.sounds[id]={seconds:secs,level:stage,layers:[{sine:hz,decay:22,release:.04},{sine:hz*2,decay:32,gain:.22,release:.04}]};}
sound('rotate',660,.10,.12);sound('hold',392,.20,.2);sound('lock',130,.13,.16);sound('drop',196,.20,.22);
sound('clear',523.25,.5,.26);g.sounds.clear.layers.push({sine:659.25,delay:.07,decay:14,release:.07},{sine:783.99,delay:.14,decay:12,release:.08});
sound('tetris',1046.5,.65,.24);sound('win',783.99,.8,.26);sound('over',146.83,.65,.22);
// Bevelled glass blocks and restrained outlines: every pixel ships inside the scene.
function tint(hex,k){return '#'+hex.slice(1).match(/../g).map(c=>Math.round(Math.min(255,parseInt(c,16)*k)).toString(16).padStart(2,'0')).join('');}
for(let p=1;p<=7;p++){
 const rows=Array.from({length:26},(_,y)=>Array.from({length:26},(_,x)=>{
   if(x===0||y===0||x===25||y===25)return '.';
   if((x===1||x===24)&&(y===1||y===24))return '.';
   if(y<=2||x<=2)return 'h';if(y>=23||x>=23)return 's';
   if(y<=6)return 'l';if(y>=19)return 'd';if(x===6&&y===8||x===7&&y===8)return 'h';return 'b';
 }).join(''));
 g.sprites['block'+p]={rows,palette:{h:tint(colors[p],1.22),s:tint(colors[p],.43),l:tint(colors[p],1.08),d:tint(colors[p],.82),b:colors[p]}};
 g.sprites['mini'+p]={rows:Array.from({length:8},(_,y)=>Array.from({length:8},(_,x)=>x===0||y===0||x===7||y===7?'.':y<2?'h':y>5?'s':'b').join('')),palette:{h:tint(colors[p],1.2),s:tint(colors[p],.55),b:colors[p]}};
 g.prefabs['block'+p]={tag:'active',shape:{sprite:'block'+p},size:[26,26],layer:3};
}
g.sprites.ghost={rows:Array.from({length:26},(_,y)=>Array.from({length:26},(_,x)=>(x===2||x===23)&&y>=2&&y<=23||(y===2||y===23)&&x>=2&&x<=23?'o':'.').join('')),palette:{o:'#6c6c6c'}};
g.prefabs.ghost={tag:'ghost',shape:{sprite:'ghost'},size:[26,26],layer:2};
for(let i=0;i<4;i++){g.scene.push({prefab:'block3',at:[-500,-500],id:'a'+i},{prefab:'ghost',at:[-500,-500],id:'g'+i});}
let rectid=0;
function worldRect(x,y,w,h,color,layer=-10){const id='deco'+rectid++;g.prefabs[id]={shape:{rect:[w,h],color},layer};g.scene.push({prefab:id,at:[x+w/2,y+h/2]});}
function panel(x,y,w,h,color='#1c1c1c',show){g.ui.push({panel:{at:[x,y],size:[w,h],color},...(show?{show}:{})});}
function text(t,x,y,color='#eeeeee',scale=2,show,align='left'){g.ui.push({text:t,at:[x,y],color,scale,align,...(show?{show}:{})});}
function button(id,label,x,y,w,h,actions,key,show){g.ui.push({button:{id,label,at:[x,y],size:[w,h],color:'#252525',...(key?{key}:{}),do:actions},...(show?{show}:{})});}
// The entire width is the board. Only a 44px information strip sits above it.
worldRect(BX,BY,300,600,'#151515');
for(let x=1;x<10;x++)worldRect(BX+x*CELL,BY,1,600,'#202020');
for(let y=1;y<20;y++)worldRect(BX,BY+y*CELL,300,1,'#202020');
text('{score:6}',8,6,'#eeeeee',2);
text('{lines} L / L{stage}',8,29,'#929292',1,'mode!=1');
text('{lines}/40 {minutes:2}:{seconds:2}',8,29,'#929292',1,'mode==1');
function preview(v,x,y){for(let p=1;p<=7;p++){
 const s=shapes[p][0],minx=Math.min(...s.map(a=>a[0])),maxx=Math.max(...s.map(a=>a[0])),miny=Math.min(...s.map(a=>a[1]));
 for(const[a,b]of s)g.ui.push({image:'mini'+p,at:[x+(a-minx)*8-(maxx-minx+1)*4,y+(b-miny)*8],show:`${v}==${p}`});
}}
preview('held',128,4);text('HOLD',116,29,'#686868',1);
preview('q0',210,4);preview('q1',266,4);text('NEXT',226,29,'#686868',1);
for(let r=2;r<22;r++)for(let x=0;x<10;x++)for(let p=1;p<=7;p++)g.ui.push({image:'block'+p,at:[BX+x*CELL+2,BY+(r-2)*CELL+2],show:`floor(r${r}/${8**x})%8==${p}`});
panel(BX,BY,300,1,'#a0a0a0','flash>0');
text('{combo}x',6,52,'#aaaaaa',1,'combo>0 && popup>0');
text('B2B',6,66,'#aaaaaa',1,'b2b>0 && popup>0');
button('rotate-cw','',-40,-40,1,1,[S('turn',1),E('rotate')],'KeyE','phase==1 && !paused');
button('rotate-ccw','',-40,-40,1,1,[S('turn',-1),E('rotate')],'KeyQ','phase==1 && !paused');
button('hold','',-40,-40,1,1,[E('hold')],'KeyC','phase==1 && !paused');
// Menus are plain charcoal. Instructions live here, away from the active board.
panel(0,0,W,H,'#000000e8','phase!=1 || paused');
text('PRISM FALL',150,176,'#eeeeee',3,'phase==0','center');
button('marathon','MARATHON  [1]',20,238,260,34,[S('mode',0),FX('newgame')],'Digit1','phase==0');
button('sprint','40 LINE SPRINT  [2]',20,286,260,34,[S('mode',1),FX('newgame')],'Digit2','phase==0');
button('zen','ZEN / PRACTICE  [3]',20,334,260,34,[S('mode',2),FX('newgame')],'Digit3','phase==0');
button('start-selected','',-40,-40,1,1,[FX('newgame')],'Enter','phase==0');
for(let i=0;i<3;i++)text('>',8,251+48*i,'#aaaaaa',1,`phase==0 && mode==${i}`);
text('UP / DOWN SELECT / ENTER START',150,402,'#929292',1,'phase==0 && !gamepad','center');
text('D-PAD SELECT / A START',150,402,'#929292',1,'phase==0 && gamepad','center');
text('PAUSED',150,176,'#eeeeee',3,'paused','center');
button('resume','RESUME  [ENTER]',20,238,260,34,[S('paused',0)],'Enter','paused');
button('retry','RESTART  [R]',20,286,260,34,[FX('newgame')],'KeyR','paused || phase>=2');
button('menu','CHOOSE A MODE  [M]',20,334,260,34,[S('phase',0),S('paused',0)],'KeyM','paused || phase>=2');
text('ARROWS MOVE / DOWN SOFT DROP',150,466,'#929292',1,'(phase==0 || paused) && !gamepad','center');
text('UP/E ROTATE / Q REVERSE',150,484,'#929292',1,'(phase==0 || paused) && !gamepad','center');
text('SPACE DROP / C OR SHIFT HOLD',150,502,'#929292',1,'(phase==0 || paused) && !gamepad','center');
text('P PAUSE / F FULLSCREEN',150,520,'#686868',1,'(phase==0 || paused) && !gamepad','center');
text('D-PAD LEFT/RIGHT MOVE',150,466,'#929292',1,'(phase==0 || paused) && gamepad','center');
text('D-PAD DOWN SOFT DROP / UP ROTATE',150,484,'#929292',1,'(phase==0 || paused) && gamepad','center');
text('A HARD DROP / X OR B HOLD',150,502,'#929292',1,'(phase==0 || paused) && gamepad','center');
text('START PAUSE / A RESUME',150,520,'#686868',1,'(phase==0 || paused) && gamepad','center');
text('BEST {runbest}',150,558,'#686868',1,'paused && mode!=1','center');
text('BEST {sprintbest:.1} SECONDS',150,558,'#686868',1,'paused && mode==1','center');
text('WELL PLAYED',150,176,'#eeeeee',3,'phase==2','center');
text('SPRINT COMPLETE',150,176,'#eeeeee',3,'phase==3','center');
text('SCORE {score:6}',150,235,'#dddddd',2,'phase>=2','center');
text('{lines} LINES / {minutes:2}:{seconds:2}.{hundredths:2}',150,265,'#929292',1,'phase>=2','center');
button('result-enter','',-40,-40,1,1,[FX('newgame')],'Enter','phase>=2');
// Short checks ship with the game; verify.js adds board fixtures and deep rule checks.
const sc=(name,script,expect,extra={})=>({name,seed:41,max_seconds:45,script,expect,...extra});
g.checks.scenarios=[
 sc('menu waits for the player',[{wait:2}],[{var:'phase',eq:0},{var:'elapsed',eq:0}]),
 sc('start move rotate hold and hard drop',[{button:'marathon'},{press:'left'},{press:'up'},{button:'hold'},{press:'action'},{wait:.1}],[{var:'phase',eq:1},{var:'pieces',eq:1},{var:'held',gt:0},{var:'score',gt:0},{sound:'drop',min:1}],{smoke:true}),
 sc('pause freezes the game clock',[{button:'marathon'},{wait:.1},{press:'pause'},{wait:2}],[{var:'paused',eq:1},{var:'elapsed',lt:.2}]),
 sc('hold is limited to once per piece',[{button:'marathon'},{button:'hold'},{button:'hold'},{button:'hold'}],[{var:'holdused',eq:1},{event:'spawn-held',min:1,max:1}]),
 sc('held direction repeats but respects the wall',[{button:'marathon'},{hold:['left'],seconds:1}],[{var:'x',gte:-2},{var:'x',lte:0}]),
 sc('sprint and zen are selectable',[{button:'sprint'},{press:'pause'},{button:'menu'},{button:'zen'}],[{var:'phase',eq:1},{var:'mode',eq:2},{var:'pieces',eq:0}]),
 sc('resume and restart reset the board',[{button:'marathon'},{press:'action'},{press:'pause'},{button:'retry'}],[{var:'phase',eq:1},{var:'pieces',eq:0},{var:'score',eq:0}]),
 sc('D-pad and face actions drive a complete session',[{press:'down'},{press:'action'},{press:'left'},{press:'up'},{press:'secondary'},{press:'action'},{press:'pause'}],[{var:'mode',eq:1},{var:'phase',eq:1},{var:'pieces',eq:1},{var:'held',gt:0},{var:'paused',eq:1}])
];
const showcase=path.join(__dirname,'showcase.scenario.json');
if(fs.existsSync(showcase))g.checks.scenarios.push(JSON.parse(fs.readFileSync(showcase,'utf8')));
// Write a stable, reviewable artifact; no dependencies and no build output ships.
fs.writeFileSync(OUT,JSON.stringify(g,null,2)+'\n');
console.log(`${OUT}: ${g.rules.length} rules, ${g.ui.length} widgets, ${fs.statSync(OUT).size} bytes`);
module.exports={game:g,shapes,occupied};
