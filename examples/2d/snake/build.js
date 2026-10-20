// Builds snake.game2d.json. Run: node build.js [OUT] (default: next to this script; in RedEngine: node examples/2d/snake/build.js examples/2d/snake.game2d.json)
const fs = require('fs');

// ---------- pixel art ----------
// The board is 200x150 px. Coarse art is 40x30 at scale 5 (one grid cell = 2x2 art pixels); fine art is 100x75 at scale 2, cut into 50-wide tiles
// because a sprite is at most 64 px a side.
const grid = (w, h, fill) => Array.from({ length: h }, (_, y) => Array.from({ length: w }, (_, x) => fill(x, y)));
const rows = (g) => g.map((r) => r.join(''));
let seed = 1;
const rnd = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
const ri = (n) => Math.floor(rnd() * n);

const checker = () => grid(40, 30, (x, y) => ((x >> 1) + (y >> 1)) % 2 ? 'b' : 'a');

function garden() {
  seed = 7;
  const W = 100, H = 75, g = grid(W, H, (x, y) => (Math.floor(x / 5) + Math.floor(y / 5)) % 2 ? 'b' : 'a');
  const set = (x, y, c) => { if (x >= 0 && y >= 0 && x < W && y < H) g[y][x] = c; };
  for (let i = 0; i < 90; i++) {
    const x = ri(W), y = ri(H), h = 2 + ri(2);
    for (let k = 0; k < h; k++) set(x, y - k, 'd');
    if (rnd() < 0.5) set(x + 1, y - h + 1, 'd');
  }
  for (let i = 0; i < 8; i++) { const x = ri(W), y = ri(H); set(x, y, 'p'); set(x + 1, y, 'q'); }
  for (let i = 0; i < 16; i++) {
    const x = 1 + ri(W - 2), y = 1 + ri(H - 2), petal = rnd() < 0.5 ? 'f' : 'w';
    set(x, y - 1, petal); set(x - 1, y, petal); set(x + 1, y, petal); set(x, y + 1, petal); set(x, y, 'y');
    set(x, y + 2, 'd');
  }
  return g;
}

function night() {
  seed = 11;
  const g = grid(40, 30, (x, y) => {
    const band = y < 10 ? 'a' : y < 20 ? 'b' : 'c';
    if (y === 9 || y === 19) return (x + y) % 2 ? band : (y < 10 ? 'b' : 'c');
    return band;
  });
  for (let i = 0; i < 40; i++) { const x = ri(40), y = ri(24); g[y][x] = rnd() < 0.3 ? 'w' : 's'; }
  for (let y = 1; y < 9; y++) for (let x = 29; x < 37; x++) {
    const d = (x - 32.5) ** 2 + (y - 4.5) ** 2, s = (x - 34.5) ** 2 + (y - 3.5) ** 2;
    if (d <= 10 && s > 7) g[y][x] = 'y';
  }
  for (let x = 0; x < 40; x++) {
    const h = 3 + Math.round(2 * Math.sin(x * 0.45) + 1.5 * Math.sin(x * 0.17 + 1));
    for (let y = 30 - h; y < 30; y++) g[y][x] = 'k';
  }
  return g;
}

// art -> board tiles: [{ rows, x, y }] in screen px (centres), each tile at most 64 art px a side
function tiles(art, scale) {
  const out = [], H = art.length, W = art[0].length, tw = Math.ceil(W / Math.ceil(W / 64)), th = Math.ceil(H / Math.ceil(H / 64));
  for (let y0 = 0; y0 < H; y0 += th) for (let x0 = 0; x0 < W; x0 += tw) {
    const part = art.slice(y0, y0 + th).map((r) => r.slice(x0, x0 + tw));
    const w = part[0].length, h = part.length;
    out.push({ rows: rows(part), x: (x0 + w / 2) * scale, y: 12 + (y0 + h / 2) * scale });
  }
  return out;
}

// ---------- snake looks ----------
const SEG = ['lllllllll', 'lhhhhhhgl', 'lhggggggl', 'lhggggggl', 'lhggggggl', 'lhggggggl', 'lhggggggl', 'lgggggggl', 'lllllllll'];
const HEAD = ['lllllllll', 'ldddddddl', 'ldwwdwwdl', 'ldwkdwkdl', 'ldddddddl', 'ldddddddl', 'ldddddddl', 'ldddddddl', 'lllllllll'];
const APPLE = ['....bgg..', '..rrbrr..', '.rrrrrrr.', 'rwrrrrrrr', 'rwrrrrrrr', 'rrrrrrrrr', 'rrrrrrrrd', '.rrrrrdd.', '..rr.rr..'];
const APPLE_PAL = { r: '#d02828', w: '#ffb0a0', d: '#901818', b: '#5a3a1a', g: '#48a838' };
const pixel = {
  seg: { palette: { l: '#c4f0c2', g: '#306230', h: '#4f8f3f' }, rows: SEG },
  head: { palette: { l: '#c4f0c2', d: '#0f380f', w: '#ffffff', k: '#000000' }, rows: HEAD },
  apple: { palette: APPLE_PAL, rows: APPLE },
};
const inverted = {
  seg: { palette: { l: '#306230', g: '#c4f0c2', h: '#f0fff0' }, rows: SEG },
  head: { palette: { l: '#306230', d: '#f0fff0', w: '#9bbc0f', k: '#0f380f' }, rows: HEAD },
  apple: { palette: APPLE_PAL, rows: APPLE },
};

// ---------- themes: background + snake + apple ----------
const THEMES = [
  { id: 'classic', name: 'CLASSIC', need: 0, board: { rect: '#9bbc0f' },
    look: { seg: { rect: [9, 9], color: '#306230' }, head: { rect: [9, 9], color: '#0f380f' }, apple: { rect: [7, 7], color: '#b02020' } } },
  { id: 'garden', name: 'GARDEN', need: 150, board: { art: garden(), scale: 2, palette: { a: '#7fb046', b: '#7aab43', d: '#5e8c30', f: '#e88ab0', w: '#f4f4e8', y: '#f8d040', p: '#a8a890', q: '#8a8a74' } },
    look: pixel },
  { id: 'night', name: 'NIGHT SKY', need: 450, board: { art: night(), scale: 5, palette: { a: '#1a1640', b: '#22205a', c: '#2c2c70', s: '#8888c0', w: '#f8f8d0', y: '#f0e8a0', k: '#100e28' } },
    look: inverted },
  { id: 'checker', name: 'CHECKER', need: 1000, board: { art: checker(), scale: 5, palette: { a: '#9bbc0f', b: '#93b40f' } },
    look: pixel },
];

const sprites = {}, prefabs = {};
const lookShape = (t, part) => {
  const l = t.look[part];
  if (l.rect) return { rect: l.rect, color: l.color };
  sprites[`${part}_${t.id}`] = l;
  return { sprite: `${part}_${t.id}` };
};
const boardTiles = {};
for (const t of THEMES) {
  if (t.board.rect) {
    prefabs[`bg_${t.id}_0`] = { tag: 'bg', shape: { rect: [200, 150], color: t.board.rect }, size: [200, 150], layer: 0 };
    boardTiles[t.id] = [{ prefab: `bg_${t.id}_0`, x: 100, y: 87 }];
  } else {
    boardTiles[t.id] = tiles(t.board.art, t.board.scale).map((tile, i) => {
      sprites[`bg_${t.id}_${i}`] = { palette: t.board.palette, rows: tile.rows };
      prefabs[`bg_${t.id}_${i}`] = { tag: 'bg', shape: { sprite: `bg_${t.id}_${i}`, scale: t.board.scale }, layer: 0 };
      return { prefab: `bg_${t.id}_${i}`, x: tile.x, y: tile.y };
    });
  }
  prefabs[`head_${t.id}`] = { tag: 'head', shape: lookShape(t, 'head'), size: [9, 9], layer: 3 };
  for (const d of 'rlud') prefabs[`seg_${t.id}_${d}`] = { tag: ['body', d], shape: lookShape(t, 'seg'), size: [9, 9], layer: 2 };
  prefabs[`apple_${t.id}`] = { tag: 'apple', shape: lookShape(t, 'apple'), size: [9, 9], layer: 1 };
}
prefabs.tail = { tag: 'tail', shape: { rect: [4, 4], color: '#000' }, size: [4, 4], layer: 1, hidden: true };

// ---------- helpers ----------
const CELL = (v, n) => `${v} * 10 + ${n}`;
const RX = 'random(0, 19) * 10 + 5', RY = 'random(0, 14) * 10 + 17';
const PLAYING = 'ended == 0 && paused == 0 && menu == 0';
const NT = THEMES.length, NU = 2 + NT - 1;
const TOASTS = ['', 'SIDE PORTALS OPEN', 'SKY PORTALS OPEN', ...THEMES.slice(1).map((t) => 'NEW BACKGROUND: ' + t.name)];
const toast = (i) => [{ set: ['toast', i] }, { set: ['toast_until', 'time + 3'] }, { play: 'unlock' }];

const dirs = { r: [1, 0], l: [-1, 0], u: [0, -1], d: [0, 1] };
const axisFree = (d) => (dirs[d][0] ? 'dx == 0' : 'dy == 0');
const pendFree = (d) => (dirs[d][0] ? 'ndx == 0' : 'ndy == 0');
const keyName = { r: 'right', l: 'left', u: 'up', d: 'down' };
const perTheme = (fn) => THEMES.map((t, i) => fn(t, i));

const rules = [];
for (const d of 'lrud') rules.push({ id: keyName[d], when: { press: keyName[d] }, if: `pend == 0 && ${axisFree(d)} && ${PLAYING}`,
  do: [{ set: ['ndx', dirs[d][0]] }, { set: ['ndy', dirs[d][1]] }, { set: ['pend', 1] }, { set: ['started', 1] }] });
for (const d of 'lrud') rules.push({ id: 'queue_' + keyName[d], when: { press: keyName[d] }, if: `pend == 1 && ${pendFree(d)} && ${PLAYING}`,
  do: [{ set: ['qdx', dirs[d][0]] }, { set: ['qdy', dirs[d][1]] }, { set: ['hasq', 1] }, { set: ['pend', 2] }] });
rules.push(
  { id: 'go', when: { press: 'right' }, if: 'started == 0 && menu == 0', do: [{ set: ['started', 1] }] },
  { id: 'go_action', when: { press: 'action' }, if: 'started == 0 && ended == 0 && menu == 0', do: [{ set: ['started', 1] }] },
  { id: 'pause', when: { press: 'pause' }, if: 'menu == 0 && started == 1 && ended == 0', do: [{ set: ['paused', '1 - paused'] }] },
  { id: 'close_menu', when: { press: 'pause' }, if: 'menu == 1', do: [{ set: ['menu', 0] }] },
  { id: 'again', when: { press: 'action' }, if: 'ended != 0 && menu == 0', do: [{ restart: true }] },

  { id: 'begin', when: { start: true }, do: [{ emit: 'new_apple' }, { set: ['fresh', 1] }, { emit: 'check_best' }, { emit: 'apply_theme' }, { emit: 'recount' }] },
  { id: 'restore', when: { event: 'loaded' }, do: [{ emit: 'check_best' }, { emit: 'apply_theme' }, { emit: 'recount' }] },

  // theme: swap the board tiles, then turn every snake part and the apple into this theme's look
  { id: 'theme_range', when: { event: 'apply_theme' }, if: `bg < 0 || bg > ${NT - 1}`, do: [{ set: ['bg', 0] }] },
  ...perTheme((t, i) => ({ id: 'theme_' + t.id, when: { event: 'apply_theme' }, if: `bg == ${i}`, do: [
    { destroy: 'tag:bg' },
    ...boardTiles[t.id].map((b) => ({ spawn: { prefab: b.prefab, at: [b.x, b.y] } })),
    { become: { target: 'id:head', prefab: 'head_' + t.id } },
    ...Object.keys(dirs).map((d) => ({ become: { target: 'tag:' + d, prefab: `seg_${t.id}_${d}` } })),
    { become: { target: 'tag:apple', prefab: 'apple_' + t.id } }] })),
  ...perTheme((t, i) => ({ id: 'apple_' + t.id, when: { event: 'new_apple' }, if: `bg == ${i}`, do: [{ spawn: { prefab: 'apple_' + t.id, at: [RX, RY] } }] })),

  { id: 'tick', when: { every: 0.1 }, if: 'started == 1 && dead == 0 && won == 0', do: [{ emit: 'step' }] },
  { id: 'turn', when: { event: 'step' }, do: [{ set: ['dx', 'ndx'] }, { set: ['dy', 'ndy'] }, { set: ['pend', 0] }, { set: ['fresh', 0] }] },
  { id: 'next_turn', when: { event: 'step' }, if: 'hasq == 1', do: [{ set: ['ndx', 'qdx'] }, { set: ['ndy', 'qdy'] }, { set: ['hasq', 0] }, { set: ['pend', 1] }] },
  { id: 'wall', when: { event: 'step' }, if: '(wrapx == 0 && (hx + dx < 0 || hx + dx > 19)) || (wrapy == 0 && (hy + dy < 0 || hy + dy > 14))',
    do: [{ set: ['dead', 1] }, { play: 'crash' }, { shake: 3 }, { end: 'lose' }] },
  { id: 'portal', when: { event: 'step' }, if: 'dead == 0 && (hx + dx < 0 || hx + dx > 19)', do: [{ add: ['portals', 1] }, { emit: 'check_sky' }] },
  ...Object.entries(dirs).flatMap(([d, [x, y]]) => perTheme((t, i) => ({ id: `leave_${t.id}_${d}`, when: { event: 'step' },
    if: `dead == 0 && bg == ${i} && ${x ? `dx == ${x}` : `dy == ${y}`}`, do: [{ spawn: { prefab: `seg_${t.id}_${d}`, at: [CELL('hx', 5), CELL('hy', 17)] } }] }))),
  { id: 'advance', when: { event: 'step' }, if: 'dead == 0', do: [
    { add: ['hx', 'dx'] }, { set: ['hx', 'hx - 20 * floor(hx / 20)'] }, { add: ['hy', 'dy'] }, { set: ['hy', 'hy - 15 * floor(hy / 15)'] },
    { teleport: { target: 'id:head', to: [CELL('hx', 5), CELL('hy', 17)] } }] },
  { id: 'shrink', when: { event: 'step' }, if: 'dead == 0 && grow == 0', do: [{ set: ['tailgo', 1] }] },
  { id: 'grow', when: { event: 'step' }, if: 'dead == 0 && grow > 0', do: [{ add: ['grow', -1] }] },

  { id: 'tail_r', when: { touching: ['tail', 'r'] }, if: 'tailgo == 1', do: [{ set: ['tailgo', 0] }, { destroy: 'other' }, { teleport: { target: 'self', to: ['self_x + 10 - 200 * floor((self_x + 10) / 200)', 'self_y'] } }] },
  { id: 'tail_l', when: { touching: ['tail', 'l'] }, if: 'tailgo == 1', do: [{ set: ['tailgo', 0] }, { destroy: 'other' }, { teleport: { target: 'self', to: ['self_x - 10 - 200 * floor((self_x - 10) / 200)', 'self_y'] } }] },
  { id: 'tail_u', when: { touching: ['tail', 'u'] }, if: 'tailgo == 1', do: [{ set: ['tailgo', 0] }, { destroy: 'other' }, { teleport: { target: 'self', to: ['self_x', 'self_y - 10 - 150 * floor((self_y - 27) / 150)'] } }] },
  { id: 'tail_d', when: { touching: ['tail', 'd'] }, if: 'tailgo == 1', do: [{ set: ['tailgo', 0] }, { destroy: 'other' }, { teleport: { target: 'self', to: ['self_x', 'self_y + 10 - 150 * floor((self_y - 7) / 150)'] } }] },

  { id: 'bite', when: { touch: ['head', 'body'] }, if: 'dead == 0', do: [{ set: ['dead', 1] }, { play: 'crash' }, { shake: 3 }, { end: 'lose' }] },

  { id: 'eat', when: { touch: ['head', 'apple'] }, if: 'fresh == 0 && dead == 0',
    do: [{ destroy: 'other' }, { add: ['len', 1] }, { add: ['grow', 1] }, { add: ['score', 10] }, { play: 'eat' }, { set: ['fresh', 1] }, { emit: 'ate' }] },
  { id: 'next_apple', when: { event: 'ate' }, if: 'len < 300', do: [{ emit: 'new_apple' }] },
  { id: 'full', when: { event: 'ate' }, if: 'len >= 300', do: [{ set: ['won', 1] }, { play: 'win' }, { end: 'win' }] },
  { id: 'apple_off_body', when: { touching: ['apple', 'body'] }, do: [{ set: ['fresh', 1] }, { teleport: { target: 'self', to: [RX, RY] } }] },
  { id: 'apple_off_head', when: { touching: ['apple', 'head'] }, if: 'fresh == 1', do: [{ teleport: { target: 'self', to: [RX, RY] } }] },

  // unlocks
  { id: 'unlock_side', when: { event: 'ate' }, if: 'u_side == 0 && len >= 25', do: [{ set: ['u_side', 1] }, { set: ['wrapx', 1] }, ...toast(1), { save: true }, { emit: 'recount' }] },
  { id: 'unlock_sky', when: { event: 'check_sky' }, if: 'u_sky == 0 && portals >= 15', do: [{ set: ['u_sky', 1] }, { set: ['wrapy', 1] }, ...toast(2), { save: true }, { emit: 'recount' }] },
  ...THEMES.slice(1).flatMap((t, i) => [
    { id: 'unlock_' + t.id, when: { event: 'ate' }, if: `u_${t.id} == 0 && score >= ${t.need}`, do: [{ set: ['u_' + t.id, 1] }, ...toast(3 + i), { save: true }, { emit: 'recount' }] },
    { id: 'restore_' + t.id, when: { event: 'check_best' }, if: `u_${t.id} == 0 && best >= ${t.need}`, do: [{ set: ['u_' + t.id, 1] }] },
  ]),
  { id: 'recount', when: { event: 'recount' }, do: [{ set: ['ucount', ['u_side', 'u_sky', ...THEMES.slice(1).map((t) => 'u_' + t.id)].join(' + ')] }] },
  { id: 'record', when: { end: 'any' }, if: 'score > best', do: [{ set: ['best', 'score'] }, { save: true }] },
);

// ---------- HUD ----------
const LIGHT = '#e0f8d0', DIM = '#8a9a7a', DARK = '#0f380f';
const OVER = '(started == 0 || paused == 1 || ended != 0) && menu == 0';
const ui = [
  { text: 'SCORE {score}  LEN {len}', at: [3, 3], color: '#9bbc0f' },
  { text: 'HI {best}', at: [197, 3], align: 'right', color: '#9bbc0f' },
  { text: 'P PAUSE  M MUSIC  U UNLOCKS', at: [100, 164], align: 'center', color: '#5a7a2a' },
  { button: { id: 'music', label: '', at: [0, 0], size: [1, 1], key: 'KeyM', do: [{ music: 'toggle' }] } },
  { button: { id: 'unlocks_key', label: '', at: [1, 0], size: [1, 1], key: 'KeyU', do: [{ set: ['menu', '(1 - menu) * (started == 0 || paused == 1 || ended != 0)'] }] } },
  // portal edges
  { panel: { at: [0, 12], size: [2, 150], color: '#b060ff' }, show: 'wrapx == 1' },
  { panel: { at: [198, 12], size: [2, 150], color: '#b060ff' }, show: 'wrapx == 1' },
  { panel: { at: [0, 12], size: [200, 2], color: '#40c0ff' }, show: 'wrapy == 1' },
  { panel: { at: [0, 160], size: [200, 2], color: '#40c0ff' }, show: 'wrapy == 1' },
  // toast
  { panel: { at: [0, 14], size: [200, 11], color: '#0f380fdd' }, show: 'time < toast_until && menu == 0' },
  { text: { pick: 'toasts', by: 'toast' }, at: [100, 16], align: 'center', color: '#f8e060', show: 'time < toast_until && menu == 0' },
  // center panel
  { panel: { at: [30, 24], size: [140, 54], color: '#c8dc78' }, show: OVER },
  { text: 'ARROWS TO MOVE', at: [100, 32], align: 'center', color: DARK, show: 'started == 0 && menu == 0' },
  { text: 'PAUSED', at: [100, 32], align: 'center', color: DARK, show: 'paused == 1 && menu == 0' },
  { text: 'P TO RESUME', at: [100, 46], align: 'center', color: DARK, show: 'paused == 1 && menu == 0' },
  { text: 'GAME OVER', at: [100, 32], align: 'center', color: DARK, show: 'dead == 1 && menu == 0' },
  { text: 'YOU FILLED THE BOARD', at: [100, 32], align: 'center', color: DARK, show: 'won == 1 && menu == 0' },
  { text: 'SPACE TO PLAY AGAIN', at: [100, 46], align: 'center', color: DARK, show: 'ended != 0 && menu == 0' },
  { button: { id: 'open_unlocks', label: 'UNLOCKS', at: [55, 60], size: [90, 13], color: '#306230', do: [{ set: ['menu', 1] }] }, show: OVER },
];

// directory
const ROWS = [
  { name: 'SIDE PORTALS', need: 'REACH LENGTH 25 IN ONE GAME', u: 'u_side', act: [{ set: ['wrapx', '1 - wrapx'] }],
    status: [['ON', 'wrapx == 1'], ['OFF', 'wrapx == 0']] },
  { name: 'SKY PORTALS', need: '15 PORTAL TRIPS IN ONE GAME', u: 'u_sky', act: [{ set: ['wrapy', '1 - wrapy'] }],
    status: [['ON', 'wrapy == 1'], ['OFF', 'wrapy == 0']] },
  ...THEMES.map((t, i) => ({ name: t.name, need: i ? `HIGH SCORE: ${t.need}` : 'STARTER BACKGROUND', u: i ? 'u_' + t.id : '1',
    act: [{ set: ['bg', i] }, { emit: 'apply_theme' }],
    status: [['IN USE', `bg == ${i}`], ['SELECT', `bg != ${i}`]] })),
];
ui.push(
  { panel: { at: [0, 0], size: [200, 172], color: '#0f1a0a' }, show: 'menu == 1' },
  { text: `UNLOCKS {ucount}/${NU}`, at: [100, 4], align: 'center', color: '#f8e060', show: 'menu == 1' },
);
ROWS.forEach((r, i) => {
  const y = 18 + i * 22, n = i + 1;
  const open = `menu == 1 && ${r.u} == 1`, shut = `menu == 1 && ${r.u} == 0`;
  ui.push(
    { button: { id: 'use_' + n, label: `${n} ${r.name}`, at: [4, y], size: [104, 11], color: '#306230', key: 'Digit' + n, do: r.act }, show: open },
    { button: { id: 'locked_' + n, label: `${n} ${r.name}`, at: [4, y], size: [104, 11], color: '#3a3a3a', key: 'Digit' + n, do: [{ play: 'nope' }] }, show: shut },
    { text: r.need, at: [6, y + 12], color: DIM, show: 'menu == 1' },
    { text: 'LOCKED', at: [196, y + 2], align: 'right', color: '#7a7a7a', show: shut },
    ...r.status.map(([t, c]) => ({ text: t, at: [196, y + 2], align: 'right', color: t === 'ON' || t === 'IN USE' ? '#f8e060' : LIGHT, show: `${open} && ${c}` })),
  );
});
ui.push(
  { text: `PRESS 1-${ROWS.length} OR CLICK`, at: [6, 163], color: DIM, show: 'menu == 1' },
  { button: { id: 'close_unlocks', label: 'BACK', at: [150, 160], size: [46, 11], color: '#306230', do: [{ set: ['menu', 0] }] }, show: 'menu == 1' },
);

// ---------- checks ----------
const scenarios = [
  { name: 'wall_up_kills', max_seconds: 5, script: [{ press: 'up' }, { wait: 1.5 }], expect: [{ var: 'dead', eq: 1 }, { ended: 'lose' }] },
  { name: 'side_wall_kills_by_default', max_seconds: 5, script: [{ press: 'right' }, { wait: 2.0 }], expect: [{ var: 'dead', eq: 1 }, { ended: 'lose' }] },
  { name: 'no_instant_reverse', max_seconds: 5, script: [{ press: 'right' }, { wait: 0.15 }, { press: 'left' }, { wait: 0.5 }], expect: [{ var: 'dx', eq: 1 }, { var: 'dead', eq: 0 }] },
  { name: 'two_quick_turns_both_count', max_seconds: 5, script: [{ press: 'up' }, { press: 'left' }, { wait: 0.5 }], expect: [{ var: 'dx', eq: -1 }, { var: 'dy', eq: 0 }, { var: 'dead', eq: 0 }] },
  { name: 'pause_freezes', max_seconds: 5, script: [{ press: 'right' }, { wait: 0.3 }, { press: 'pause' }, { wait: 1.0 }], expect: [{ var: 'paused', eq: 1 }, { var: 'hx', lt: 9 }] },
  { name: 'locked_portal_does_nothing', max_seconds: 3, script: [{ button: 'open_unlocks' }, { button: 'locked_1' }, { wait: 0.2 }], expect: [{ var: 'wrapx', eq: 0 }, { var: 'menu', eq: 1 }] },
  { name: 'classic_board_is_one_piece', max_seconds: 3, script: [{ wait: 0.2 }], expect: [{ count: 'bg', eq: 1 }, { count: 'apple', eq: 1 }] },
];

// ---------- the game ----------
const game = {
  game2d: 1,
  id: 'snake',
  title: 'Snake',
  description: 'The original: steer a snake of squares, eat apples to grow, and keep going until the snake fills the whole board. Unlock portals and pixel-art backgrounds along the way.',
  capabilities: { presentation: '2d', platforms: ['windows', 'linux'], networking: 'offline', input: ['keyboard', 'mouse'], persistence: ['settings', 'progress'] },
  view: { width: 200, height: 172, background: '#1c2b12', scale: 'integer' },
  sprites,
  sounds: {
    eat: { seconds: 0.5, level: 0.3, layers: [{ sine: 659, harmonics: [[2, 0.15]], attack: 0.01, decay: 7 }, { sine: 988, delay: 0.06, attack: 0.01, decay: 8, gain: 0.5 }] },
    crash: { seconds: 0.9, level: 0.3, layers: [{ glide: [330, 165, 3], attack: 0.02, decay: 3.5 }, { sine: 220, attack: 0.03, decay: 4, gain: 0.4 }] },
    win: { seconds: 2.0, level: 0.35, layers: [{ sine: 523, attack: 0.02, decay: 1.5 }, { sine: 659, delay: 0.15, attack: 0.02, decay: 1.5 }, { sine: 784, delay: 0.3, attack: 0.02, decay: 1.3 }, { sine: 1047, delay: 0.45, attack: 0.02, decay: 1.2, gain: 0.6 }] },
    unlock: { seconds: 1.2, level: 0.3, layers: [{ sine: 784, attack: 0.01, decay: 3 }, { sine: 988, delay: 0.1, attack: 0.01, decay: 3 }, { sine: 1319, delay: 0.2, attack: 0.01, decay: 2.5, gain: 0.7 }] },
    nope: { seconds: 0.3, level: 0.2, layers: [{ sine: 196, attack: 0.01, decay: 10 }] },
  },
  music: {
    theme: { title: 'Garden', bpm: 60, beats: 4, bars: 8, key: 'D', scale: 'pentatonic', seed: 11, lufs: -30,
      reverb: { decay: 6, mix: 0.45 },
      instruments: {
        pad: { seconds: 10, level: 0.4, layers: [{ sine: 1, attack: 2.5, release: 3 }, { sine: 1.004, attack: 3, release: 3, gain: 0.5 }] },
        bell: { seconds: 4, level: 0.3, layers: [{ sine: 1, harmonics: [[2.76, 0.15]], attack: 0.01, decay: 1.2 }] },
      },
      tracks: [
        { inst: 'pad', play: 'chords', chords: '1 4 6 5', every: '2 bars', octave: 3, gain: 0.7 },
        { inst: 'bell', play: 'walk', every: '1 beat', density: 0.25, range: [1, 8], octave: 5, gain: 0.4, pan: 'spread', send: 1, vel: [0.3, 0.6] },
      ] },
  },
  tables: { toasts: TOASTS },
  vars: { hx: 4, hy: 7, dx: 1, dy: 0, ndx: 1, ndy: 0, pend: 0, qdx: 0, qdy: 0, hasq: 0, len: 3, grow: 0, tailgo: 0, fresh: 1, started: 0, paused: 0, menu: 0,
    dead: 0, won: 0, score: 0, best: 0, portals: 0, toast: 0, toast_until: 0, ucount: 0,
    u_side: 0, u_sky: 0, ...Object.fromEntries(THEMES.slice(1).map((t) => ['u_' + t.id, 0])), wrapx: 0, wrapy: 0, bg: 0 },
  persist: ['best', 'u_side', 'u_sky', ...THEMES.slice(1).map((t) => 'u_' + t.id), 'wrapx', 'wrapy', 'bg'],
  pause_if: 'paused == 1 || menu == 1',
  prefabs,
  scene: [
    { prefab: 'bg_classic_0', at: [100, 87] },
    { prefab: 'seg_classic_r', at: [25, 87] },
    { prefab: 'seg_classic_r', at: [35, 87] },
    { prefab: 'tail', at: [25, 87] },
    { prefab: 'head_classic', at: [45, 87], id: 'head' },
  ],
  ui,
  rules,
  checks: { scenarios },
};

const out = process.argv[2] || __dirname + '/snake.game2d.json';
fs.writeFileSync(out, JSON.stringify(game, null, 1) + '\n');
console.log('wrote ' + out);
