//! `red_engine2 nav`: prove a scene's `nav` waypoint graph with the game's own movement.
//!
//! A bot only follows a route the physics allows if every edge of its graph was *travelled*, not eyeballed. So each edge is replayed with the
//! real per-tick movement function (`step_player_tuned`: the scene's speeds, momentum, jump pads and colliders), steering straight at the
//! far node the way a bot does:
//!
//! - `walk` edges are travelled in both directions; `jump` edges jump when close to the far node; `pad` edges must start on a jump pad and are
//!   steered through the air; `drop` edges walk off and land.
//! - Every node must stand on a floor (within 0.25 m of the authored height) and outside solid geometry.
//! - Every node must be reachable from the first one, and able to get back to it.
//!
//! `nav scene.json` prints one line per edge and exits 1 if any failed; `nav scene.json --route hall deck` prints the route bots would take.
//! A scene can also carry `"checks": {"nav": {}}`, so `verify` (and a game project's `check`) runs it with everything else.

use crate::collide::ground_height_at;
use crate::player::Character;
use crate::sim::ai::nav::{EdgeKind, Nav};
use crate::sim::match_sim::MatchSim;
use crate::sim::player::{step_player_tuned, PlayerInput, PlayerState};
use crate::sim::spawns::parse_spawns;
use glam::{Vec2, Vec3};
use serde_json::{json, Value};

/// Ticks an edge may take before it counts as failed (12 s).
const MAX_TICKS: u32 = 60 * 12;
/// How close to the far node, horizontally, counts as arrived, metres.
const ARRIVE_M: f32 = 1.0;

/// What one successful trial cost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trial {
    /// Ticks it took.
    pub ticks: u32,
    /// Metres actually travelled along the ground track.
    pub metres: f32,
}

/// One edge in one direction.
#[derive(Debug, Clone)]
pub struct EdgeResult {
    /// Start node id.
    pub from: String,
    /// End node id.
    pub to: String,
    /// How the edge is travelled.
    pub kind: EdgeKind,
    /// The trial, or why it failed.
    pub result: Result<Trial, String>,
}

/// Everything `nav` found out.
#[derive(Debug, Clone, Default)]
pub struct NavReport {
    /// Node count.
    pub nodes: usize,
    /// Edge count (a `walk` edge is tried in both directions).
    pub edges: usize,
    /// One entry per edge direction tried.
    pub results: Vec<EdgeResult>,
    /// Nodes that float, are sunk or stand inside geometry.
    pub node_problems: Vec<String>,
    /// Nodes no route from the first node reaches.
    pub unreachable: Vec<String>,
    /// Nodes from which the first node cannot be reached again (a bot that lands there is stuck for good).
    pub traps: Vec<String>,
}

impl NavReport {
    /// Number of things wrong: failed trials, bad nodes, unreachable nodes and traps.
    pub fn failures(&self) -> usize {
        self.results.iter().filter(|r| r.result.is_err()).count() + self.node_problems.len() + self.unreachable.len() + self.traps.len()
    }

    /// One line per edge direction, then the node and connectivity problems, then a summary.
    pub fn render(&self, verbose: bool) -> String {
        let mut s = String::new();
        for r in &self.results {
            match &r.result {
                Ok(t) if verbose => {
                    s.push_str(&format!("  ok    {:<14} -> {:<14} {:<5} {:>4.1} s, {:>5.1} m\n", r.from, r.to, r.kind.name(), t.ticks as f32 / 60.0, t.metres))
                }
                Ok(_) => {}
                Err(e) => s.push_str(&format!("  FAIL  {:<14} -> {:<14} {:<5} {e}\n", r.from, r.to, r.kind.name())),
            }
        }
        for p in &self.node_problems {
            s.push_str(&format!("  FAIL  node {p}\n"));
        }
        for n in &self.unreachable {
            s.push_str(&format!("  FAIL  node {n} cannot be reached from the first node\n"));
        }
        for n in &self.traps {
            s.push_str(&format!("  FAIL  node {n} is a trap: nothing leads back to the first node\n"));
        }
        let bad = self.failures();
        s.push_str(&format!(
            "nav: {} node(s), {} edge(s), {} trial(s): {}\n",
            self.nodes,
            self.edges,
            self.results.len(),
            if bad == 0 { "every edge can be travelled".to_string() } else { format!("{bad} problem(s)") }
        ));
        s
    }

    /// The report as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "nodes": self.nodes, "edges": self.edges, "problems": self.failures(),
            "trials": self.results.iter().map(|r| json!({
                "from": r.from, "to": r.to, "kind": r.kind.name(),
                "ok": r.result.is_ok(),
                "ticks": r.result.as_ref().ok().map(|t| t.ticks), "metres": r.result.as_ref().ok().map(|t| t.metres),
                "error": r.result.as_ref().err(),
            })).collect::<Vec<_>>(),
            "node_problems": self.node_problems, "unreachable": self.unreachable, "traps": self.traps,
        })
    }
}

fn yaw_of(d: Vec2) -> f32 {
    libm::atan2f(d.x, -d.y)
}

/// Travels from `a` to `b` the way a bot would: heading straight at `b`, full speed, jumping for a jump edge.
fn travel(sim: &MatchSim, kind: EdgeKind, a: Vec3, b: Vec3) -> Result<Trial, String> {
    let (colliders, ground) = sim.static_world();
    let (tuning, pads) = sim.movement();
    if kind == EdgeKind::Pad && !pads.iter().any(|p| p.touches(Vec2::new(a.x, a.z), a.y)) {
        return Err(format!("a pad edge must start on a jump pad, but there is none at ({:.1}, {:.1}) at height {:.1}", a.x, a.z, a.y));
    }
    let mut st = PlayerState::spawn(a.x, a.z, a.y, 0.0, Character::Human);
    let (mut metres, mut jumped_at) = (0.0f32, None::<u32>);
    let (mut last_check, mut last_pos) = (0u32, st.pos);
    for tick in 1..=MAX_TICKS {
        let to = Vec2::new(b.x - st.pos.x, b.z - st.pos.y);
        let dist = to.length();
        let floor = ground_height_at(ground, st.pos, st.foot_y);
        let grounded = st.vy <= 0.0 && (st.foot_y - floor).abs() < 0.05;
        if tick > 2 && grounded && dist < ARRIVE_M && (st.foot_y - b.y).abs() < 0.7 {
            return Ok(Trial { ticks: tick, metres });
        }
        let jump = kind == EdgeKind::Jump && grounded && dist < 3.5 && jumped_at.is_none_or(|t| tick > t + 30);
        if jump {
            jumped_at = Some(tick);
        }
        let input = PlayerInput { forward: 127, analog: true, sprint: true, jump, yaw: if dist > 0.05 { yaw_of(to) } else { st.yaw }, ..Default::default() };
        let before = st.pos;
        step_player_tuned(&mut st, &input, colliders, ground, tuning, pads);
        metres += (st.pos - before).length();
        if tick - last_check >= 90 {
            if grounded && (st.pos - last_pos).length() < 0.5 {
                return Err(format!(
                    "stuck at ({:.1}, {:.1}) on floor y {:.1}, {:.1} m short of ({:.1}, {:.1}) y {:.1}: a {} edge is a straight line, add a node to steer round what is in the way",
                    st.pos.x, st.pos.y, st.foot_y, dist, b.x, b.z, b.y, kind.name()
                ));
            }
            (last_check, last_pos) = (tick, st.pos);
        }
    }
    let to = Vec2::new(b.x - st.pos.x, b.z - st.pos.y);
    Err(format!("gave up after 12 s at ({:.1}, {:.1}) y {:.1}, {:.1} m from ({:.1}, {:.1}) y {:.1}", st.pos.x, st.pos.y, st.foot_y, to.length(), b.x, b.z, b.y))
}

/// Checks the graph of a scene given as JSON text.
pub fn check_text(text: &str) -> Result<NavReport, String> {
    let scene = crate::schema::parse_scene(text).map_err(|e| e.join("; "))?;
    let Some(nav) = scene.nav.clone() else { return Err("the scene has no `nav` block (nodes and edges bots route along)".to_string()) };
    let spawns = parse_spawns(text)?;
    let sim = MatchSim::try_new(&scene, spawns)?;
    Ok(check(&sim, &nav))
}

/// Checks `nav` against the world of `sim`.
pub fn check(sim: &MatchSim, nav: &Nav) -> NavReport {
    let mut report = NavReport { nodes: nav.nodes.len(), edges: nav.edges.len(), ..Default::default() };
    let (colliders, ground) = sim.static_world();
    let (tuning, pads) = sim.movement();
    for n in &nav.nodes {
        let p = Vec2::new(n.pos.x, n.pos.z);
        let floor = ground_height_at(ground, p, n.pos.y + 0.35);
        if (floor - n.pos.y).abs() > 0.25 {
            report.node_problems.push(format!("`{}` is at height {:.2} but the floor there is at {:.2}", n.id, n.pos.y, floor));
            continue;
        }
        let mut st = PlayerState::spawn(p.x, p.y, n.pos.y, 0.0, Character::Human);
        // Collisions are resolved when the body moves, so give it a nudge: a body standing in a wall is pushed out by more than a step.
        step_player_tuned(&mut st, &PlayerInput { forward: 1, ..Default::default() }, colliders, ground, tuning, pads);
        if (st.pos - p).length() > 0.3 {
            report.node_problems.push(format!(
                "`{}` at ({:.1}, {:.1}) is inside solid geometry (pushed out by {:.2} m)",
                n.id,
                p.x,
                p.y,
                (st.pos - p).length()
            ));
        }
    }
    for e in &nav.edges {
        let (a, b) = (&nav.nodes[e.from], &nav.nodes[e.to]);
        report.results.push(EdgeResult { from: a.id.clone(), to: b.id.clone(), kind: e.kind, result: travel(sim, e.kind, a.pos, b.pos) });
        if e.kind == EdgeKind::Walk {
            report.results.push(EdgeResult { from: b.id.clone(), to: a.id.clone(), kind: e.kind, result: travel(sim, e.kind, b.pos, a.pos) });
        }
    }
    // Connectivity over the edges that actually work.
    let n = nav.nodes.len();
    let ok_between = |from: usize, to: usize| report.results.iter().any(|r| r.result.is_ok() && r.from == nav.nodes[from].id && r.to == nav.nodes[to].id);
    let (mut forward, mut backward) = (vec![false; n], vec![false; n]);
    if n > 0 {
        forward[0] = true;
        backward[0] = true;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for e in &nav.edges {
            for (a, b) in [(e.from, e.to), (e.to, e.from)] {
                if ok_between(a, b) {
                    if forward[a] && !forward[b] {
                        forward[b] = true;
                        changed = true;
                    }
                    if backward[b] && !backward[a] {
                        backward[a] = true;
                        changed = true;
                    }
                }
            }
        }
    }
    report.unreachable = (0..n).filter(|i| !forward[*i]).map(|i| nav.nodes[i].id.clone()).collect();
    report.traps = (0..n).filter(|i| forward[*i] && !backward[*i]).map(|i| nav.nodes[i].id.clone()).collect();
    report
}

/// The route a bot would take between two nodes, as text: each hop with its kind and the total cost.
pub fn route_text(text: &str, from: &str, to: &str) -> Result<String, String> {
    let scene = crate::schema::parse_scene(text).map_err(|e| e.join("; "))?;
    let nav = scene.nav.ok_or("the scene has no `nav` block")?;
    let find = |id: &str| {
        nav.index_of(id).ok_or_else(|| format!("no node `{id}` (nodes: {})", nav.nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>().join(", ")))
    };
    let (a, b) = (find(from)?, find(to)?);
    let path = nav.path(a, b).ok_or_else(|| format!("no route from `{from}` to `{to}`"))?;
    let mut s = String::new();
    let mut cost = 0.0;
    for w in path.windows(2) {
        let kind = nav.edge_between(w[0], w[1]).map_or(EdgeKind::Walk, |e| e.kind);
        let c = nav.edge_between(w[0], w[1]).map_or(0.0, |e| nav.cost(e));
        cost += c;
        s.push_str(&format!("  {:<14} -> {:<14} {:<5} cost {:>5.1}\n", nav.nodes[w[0]].id, nav.nodes[w[1]].id, kind.name(), c));
    }
    s.push_str(&format!("route {from} -> {to}: {} hop(s), cost {cost:.1}\n", path.len().saturating_sub(1)));
    Ok(s)
}

/// `checks.nav` for `verify`: one row per failed trial or problem, or a single passing summary row. `block` may hold `max_failures`.
pub fn verify_checks(path: &std::path::Path, block: &Value) -> Result<Vec<(String, bool, String)>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let report = check_text(&text)?;
    let allowed = block.get("max_failures").and_then(Value::as_u64).unwrap_or(0) as usize;
    let bad = report.failures();
    let mut rows = Vec::new();
    if bad > allowed {
        for line in report.render(false).lines().filter(|l| l.trim_start().starts_with("FAIL")) {
            rows.push(("nav".to_string(), false, line.trim_start().trim_start_matches("FAIL").trim().to_string()));
        }
        if rows.is_empty() {
            rows.push(("nav".to_string(), false, format!("{bad} problem(s)")));
        }
    } else {
        let hours: f32 = report.results.iter().filter_map(|r| r.result.as_ref().ok()).map(|t| t.ticks as f32 / 60.0).sum();
        rows.push((
            "nav".to_string(),
            true,
            format!("{} node(s), {} edge(s): every edge travelled with the real movement ({hours:.0} s of walking in total)", report.nodes, report.edges),
        ));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(nav: &str, extra_objects: &str, jump_pads: &str) -> String {
        format!(
            r##"{{"camera":{{"position":[0,1.7,0],"target":[0,1,-5]}},"player":{{"jump_speed":8,"gravity":22}},{jump_pads}
            "spawns":[{{"id":"s","position":[0,0,0],"yaw_deg":0}}],
            "objects":[{{"id":"floor","type":"box","size":[60,0.4,60],"position":[0,-0.2,0]}}{extra_objects}],
            "nav":{nav}}}"##
        )
    }

    #[test]
    fn open_floor_edges_pass_and_a_wall_in_the_way_fails_with_advice() {
        let open = scene(r#"{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[10,0,0]}],"edges":[["a","b"]]}"#, "", "");
        let r = check_text(&open).unwrap();
        assert_eq!((r.failures(), r.results.len()), (0, 2), "{}", r.render(true));
        let walled = scene(
            r#"{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[10,0,0]}],"edges":[["a","b"]]}"#,
            r#",{"id":"wall","type":"box","size":[0.5,4,20],"position":[5,2,0]}"#,
            "",
        );
        let r = check_text(&walled).unwrap();
        assert_eq!(r.results.iter().filter(|x| x.result.is_err()).count(), 2, "both directions fail");
        assert!(r.render(false).contains("add a node to steer round"), "{}", r.render(false));
    }

    #[test]
    fn a_pad_edge_must_start_on_a_pad_and_lands_on_the_deck() {
        let pads = r#""jump_pads":[{"id":"p","position":[0,0,0],"size":[2,2],"launch_speed":16}],"#;
        let objs = r#",{"id":"deck","type":"box","size":[6,0.4,6],"position":[0,3.4,9.5]}"#;
        let ok = scene(r#"{"nodes":[{"id":"pad","pos":[0,0,0]},{"id":"deck","pos":[0,3.6,9.5]}],"edges":[["pad","deck","pad"]]}"#, objs, pads);
        let r = check_text(&ok).unwrap();
        assert!(r.results[0].result.is_ok(), "{}", r.render(true));
        let bad = scene(r#"{"nodes":[{"id":"nopad","pos":[5,0,0]},{"id":"deck","pos":[0,3.6,9.5]}],"edges":[["nopad","deck","pad"]]}"#, objs, pads);
        assert!(check_text(&bad).unwrap().render(false).contains("must start on a jump pad"));
    }

    #[test]
    fn nodes_must_stand_on_floors_outside_walls_and_be_connected() {
        let s = scene(
            r#"{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"high","pos":[3,2,0]},{"id":"inside","pos":[10,0,10]},{"id":"far","pos":[-8,0,0]}],"edges":[["a","inside"]]}"#,
            r#",{"id":"block","type":"box","size":[4,4,4],"position":[10,2,10]}"#,
            "",
        );
        let r = check_text(&s).unwrap();
        let text = r.render(false);
        assert!(text.contains("`high` is at height 2.00 but the floor there is at 0.00"), "{text}");
        assert!(text.contains("`inside` at (10.0, 10.0) is inside solid geometry"), "{text}");
        assert!(r.unreachable.contains(&"far".to_string()) && r.unreachable.contains(&"high".to_string()), "{:?}", r.unreachable);
    }

    #[test]
    fn a_one_way_drop_makes_the_landing_a_trap_unless_something_leads_back() {
        let objs = r#",{"id":"ledge","type":"box","size":[6,0.4,6],"position":[0,3.4,-8]},{"id":"stair","type":"stairs","position":[6,0,-8],"width":2,"run":8,"rise":3.6,"steps":18,"rotation":[0,90,0]}"#;
        let drop_only = scene(r#"{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"top","pos":[0,3.6,-8]}],"edges":[["top","a","drop"],["a","top","pad"]]}"#, objs, "");
        let r = check_text(&drop_only).unwrap();
        assert!(!r.render(false).is_empty());
        assert!(r.failures() > 0, "{}", r.render(true));
    }

    #[test]
    fn routes_print_their_hops_and_unknown_nodes_list_the_choices() {
        let s = scene(r#"{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[10,0,0]},{"id":"c","pos":[10,0,10]}],"edges":[["a","b"],["b","c"]]}"#, "", "");
        let t = route_text(&s, "a", "c").unwrap();
        assert!(t.contains("a") && t.contains("2 hop(s)"), "{t}");
        assert!(route_text(&s, "a", "zzz").unwrap_err().contains("nodes: a, b, c"));
        assert!(check_text(r#"{"camera":{"position":[0,1,0]},"objects":[]}"#).unwrap_err().contains("no `nav` block"));
    }

    #[test]
    fn verify_reports_one_summary_row_when_everything_works() {
        let dir = std::env::temp_dir().join(format!("nav_verify_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("s.json");
        std::fs::write(&p, scene(r#"{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[8,0,0]}],"edges":[["a","b"]]}"#, "", "")).unwrap();
        let rows = verify_checks(&p, &json!({})).unwrap();
        assert!(rows.len() == 1 && rows[0].1, "{rows:?}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
