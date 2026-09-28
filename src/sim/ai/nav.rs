//! The waypoint graph a scene may carry in its top-level `nav` block: where bots can go and how they get from one place to the next.
//!
//! ```json
//! "nav": {
//!   "nodes": [ { "id": "hall", "pos": [0, 0, 0] }, { "id": "ramp_foot", "pos": [6, 0, 0] }, { "id": "deck", "pos": [10, 3, 0] } ],
//!   "edges": [ ["hall", "ramp_foot"], ["ramp_foot", "deck"], ["deck", "hall", "drop"] ]
//! }
//! ```
//!
//! A node is a spot on a floor (`pos.y` is the height of the feet there). An edge is `[from, to]` or `[from, to, kind]`:
//! - `walk` (default): walk in a straight line; usable both ways. Stairs and ramps are walk edges.
//! - `jump`: the bot jumps near `from` to reach `to` (a ledge, a gap); one way.
//! - `pad`: `from` is on a jump pad; step on it, then steer through the air to `to`; one way.
//! - `drop`: step off a ledge and fall to `to`; one way.
//!
//! The graph is *data the engine checks*: `red_engine2 nav check` replays every edge with the real movement code (and the real jump pads)
//! and reports the ones a player could not actually travel, so a bot never follows a route the physics does not allow.

use crate::strict::check_keys;
use glam::{Vec2, Vec3};
use serde_json::{Map, Value};
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Most nodes a graph may have.
pub const MAX_NODES: usize = 512;
/// Most edges a graph may have.
pub const MAX_EDGES: usize = 4096;

/// How a bot gets along an edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// A straight walk; usable in both directions.
    Walk,
    /// Jump near the start; one way.
    Jump,
    /// Ride the jump pad at the start, steer to the end; one way.
    Pad,
    /// Walk off the edge and land at the end; one way.
    Drop,
}

impl EdgeKind {
    fn parse(s: &str) -> Option<EdgeKind> {
        match s {
            "walk" => Some(EdgeKind::Walk),
            "jump" => Some(EdgeKind::Jump),
            "pad" => Some(EdgeKind::Pad),
            "drop" => Some(EdgeKind::Drop),
            _ => None,
        }
    }

    /// The word used in scenes and reports.
    pub fn name(self) -> &'static str {
        match self {
            EdgeKind::Walk => "walk",
            EdgeKind::Jump => "jump",
            EdgeKind::Pad => "pad",
            EdgeKind::Drop => "drop",
        }
    }
}

/// A place bots can stand and route through.
#[derive(Debug, Clone, PartialEq)]
pub struct NavNode {
    /// Its name in the scene.
    pub id: String,
    /// Feet position: `x`, floor height, `z`.
    pub pos: Vec3,
}

/// A way between two nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NavEdge {
    /// Node index it starts at.
    pub from: usize,
    /// Node index it ends at.
    pub to: usize,
    /// How it is travelled.
    pub kind: EdgeKind,
}

/// The parsed graph with adjacency for routing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Nav {
    /// Every node, in scene order.
    pub nodes: Vec<NavNode>,
    /// Every edge as written (a walk edge is one entry but is usable both ways).
    pub edges: Vec<NavEdge>,
    /// For each node: `(node reached, edge index)` for every directed way out of it.
    adj: Vec<Vec<(usize, usize)>>,
}

/// Keys of the `nav` block.
pub const NAV_KEYS: &[&str] = &["nodes", "edges"];
/// Keys of one node.
pub const NODE_KEYS: &[&str] = &["id", "pos"];

fn vec3(v: &Value) -> Option<Vec3> {
    let a = v.as_array().filter(|a| a.len() == 3)?;
    let f = |i: usize| a[i].as_f64().filter(|n| n.is_finite() && n.abs() < 10_000.0).map(|n| n as f32);
    Some(Vec3::new(f(0)?, f(1)?, f(2)?))
}

impl Nav {
    /// Builds a graph from parts (tests and generators); no validation beyond index bounds, which are checked.
    pub fn from_parts(nodes: Vec<NavNode>, edges: Vec<NavEdge>) -> Result<Nav, String> {
        if let Some(e) = edges.iter().find(|e| e.from >= nodes.len() || e.to >= nodes.len()) {
            return Err(format!("edge {} -> {} names a node that does not exist", e.from, e.to));
        }
        let mut adj = vec![Vec::new(); nodes.len()];
        for (i, e) in edges.iter().enumerate() {
            adj[e.from].push((e.to, i));
            if e.kind == EdgeKind::Walk {
                adj[e.to].push((e.from, i));
            }
        }
        Ok(Nav { nodes, edges, adj })
    }

    /// Whether the scene has no graph.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The index of node `id`.
    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The edge from `a` to `b` (either direction for a walk edge), if there is one.
    pub fn edge_between(&self, a: usize, b: usize) -> Option<&NavEdge> {
        self.adj.get(a)?.iter().find(|(to, _)| *to == b).map(|(_, e)| &self.edges[*e])
    }

    /// Every directed way out of `node` as `(node reached, edge)`.
    pub fn neighbours(&self, node: usize) -> impl Iterator<Item = (usize, &NavEdge)> {
        self.adj.get(node).into_iter().flatten().map(|(to, e)| (*to, &self.edges[*e]))
    }

    /// The node closest to `pos` among those within `max_dy` metres of its height (horizontal distance, a floor apart counting triple).
    pub fn nearest(&self, pos: Vec3, max_dy: f32) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| (n.pos.y - pos.y).abs() <= max_dy)
            .map(|(i, n)| (i, Vec2::new(n.pos.x - pos.x, n.pos.z - pos.z).length() + (n.pos.y - pos.y).abs() * 3.0))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// What it costs to travel `edge` (metres-equivalent): pads are quick, jumps and drops cost a little extra, floors apart add up.
    pub fn cost(&self, edge: &NavEdge) -> f32 {
        let (a, b) = (self.nodes[edge.from].pos, self.nodes[edge.to].pos);
        let flat = Vec2::new(a.x - b.x, a.z - b.z).length();
        let dy = (a.y - b.y).abs();
        match edge.kind {
            EdgeKind::Walk => flat + dy * 1.5,
            EdgeKind::Jump => flat * 1.3 + dy * 2.0 + 1.5,
            EdgeKind::Pad => 3.0 + flat * 0.4,
            EdgeKind::Drop => flat + dy * 0.5 + 1.0,
        }
    }

    /// The cheapest route `from` -> `to` as node indices including both ends (A*, straight-line heuristic), or `None` if there is none.
    pub fn path(&self, from: usize, to: usize) -> Option<Vec<usize>> {
        let n = self.nodes.len();
        if from >= n || to >= n {
            return None;
        }
        if from == to {
            return Some(vec![from]);
        }
        let goal = self.nodes[to].pos;
        let h = |i: usize| {
            let p = self.nodes[i].pos;
            Vec2::new(p.x - goal.x, p.z - goal.z).length() * 0.35
        };
        let mut best = vec![f32::INFINITY; n];
        let mut came: Vec<usize> = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        best[from] = 0.0;
        open.push(Reverse((h(from).to_bits(), from)));
        while let Some(Reverse((_, i))) = open.pop() {
            if i == to {
                let mut path = vec![to];
                let mut at = to;
                while came[at] != usize::MAX {
                    at = came[at];
                    path.push(at);
                }
                path.reverse();
                return Some(path);
            }
            for (j, e) in self.neighbours(i) {
                let g = best[i] + self.cost(e);
                if g < best[j] {
                    best[j] = g;
                    came[j] = i;
                    open.push(Reverse(((g + h(j)).to_bits(), j)));
                }
            }
        }
        None
    }
}

/// Parses a scene's optional `nav` block; every problem is `nav.path: message`.
pub fn parse_nav(root: &Map<String, Value>) -> Result<Option<Nav>, Vec<String>> {
    let Some(value) = root.get("nav") else { return Ok(None) };
    let Some(o) = value.as_object() else {
        return Err(vec!["nav: must be an object like {\"nodes\": [{\"id\": \"a\", \"pos\": [0, 0, 0]}], \"edges\": [[\"a\", \"b\"]]}".to_string()]);
    };
    let mut errs = Vec::new();
    check_keys(&mut errs, "nav", o, NAV_KEYS);
    let mut nodes: Vec<NavNode> = Vec::new();
    match o.get("nodes").and_then(Value::as_array) {
        None => errs.push("nav.nodes: missing (a list of {id, pos: [x, y, z]})".to_string()),
        Some(list) => {
            if list.len() > MAX_NODES {
                errs.push(format!("nav.nodes: at most {MAX_NODES} nodes"));
            }
            for (i, item) in list.iter().enumerate() {
                let path = format!("nav.nodes[{i}]");
                let Some(n) = item.as_object() else {
                    errs.push(format!("{path}: must be an object like {{\"id\": \"a\", \"pos\": [0, 0, 0]}}"));
                    continue;
                };
                check_keys(&mut errs, &path, n, NODE_KEYS);
                let id = n.get("id").and_then(Value::as_str).filter(|s| !s.is_empty());
                let pos = n.get("pos").and_then(vec3);
                match (id, pos) {
                    (Some(id), Some(pos)) => {
                        if nodes.iter().any(|x| x.id == id) {
                            errs.push(format!("{path}.id: `{id}` is used twice"));
                        }
                        nodes.push(NavNode { id: id.to_string(), pos });
                    }
                    (None, _) => errs.push(format!("{path}.id: missing (a non-empty name)")),
                    (_, None) => errs.push(format!("{path}.pos: must be [x, y, z] in metres (y = the floor height)")),
                }
            }
        }
    }
    let mut edges = Vec::new();
    match o.get("edges") {
        None => {}
        Some(v) => match v.as_array() {
            None => errs.push("nav.edges: must be a list of [from, to] or [from, to, kind]".to_string()),
            Some(list) => {
                if list.len() > MAX_EDGES {
                    errs.push(format!("nav.edges: at most {MAX_EDGES} edges"));
                }
                for (i, item) in list.iter().enumerate() {
                    let path = format!("nav.edges[{i}]");
                    let Some(a) = item.as_array().filter(|a| a.len() == 2 || a.len() == 3) else {
                        errs.push(format!("{path}: must be [from, to] or [from, to, kind]"));
                        continue;
                    };
                    let find = |v: &Value| v.as_str().and_then(|id| nodes.iter().position(|n| n.id == id));
                    let (Some(from), Some(to)) = (find(&a[0]), find(&a[1])) else {
                        let unknown: Vec<String> = a[..2].iter().filter(|v| find(v).is_none()).map(|v| v.to_string()).collect();
                        errs.push(format!("{path}: unknown node {} (ids come from nav.nodes)", unknown.join(" and ")));
                        continue;
                    };
                    let kind = match a.get(2) {
                        None => EdgeKind::Walk,
                        Some(k) => match k.as_str().and_then(EdgeKind::parse) {
                            Some(k) => k,
                            None => {
                                errs.push(format!("{path}: kind must be walk, jump, pad or drop"));
                                continue;
                            }
                        },
                    };
                    if from == to {
                        errs.push(format!("{path}: an edge cannot start and end at the same node"));
                        continue;
                    }
                    edges.push(NavEdge { from, to, kind });
                }
            }
        },
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    Nav::from_parts(nodes, edges).map(Some).map_err(|e| vec![e])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(s: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(s).unwrap().as_object().unwrap().clone()
    }

    const GRAPH: &str = r#"{"nav":{"nodes":[
        {"id":"a","pos":[0,0,0]},{"id":"b","pos":[10,0,0]},{"id":"c","pos":[10,0,10]},{"id":"deck","pos":[0,4,10]},{"id":"pad","pos":[0,0,5]}],
        "edges":[["a","b"],["b","c"],["c","deck","jump"],["a","pad"],["pad","deck","pad"],["deck","a","drop"]]}}"#;

    #[test]
    fn no_block_means_no_graph_and_a_good_block_parses() {
        assert_eq!(parse_nav(&root("{}")).unwrap(), None);
        let nav = parse_nav(&root(GRAPH)).unwrap().unwrap();
        assert_eq!((nav.nodes.len(), nav.edges.len()), (5, 6));
        assert_eq!(nav.edge_between(1, 0).map(|e| e.kind), Some(EdgeKind::Walk), "walk edges work both ways");
        assert_eq!(nav.edge_between(2, 3).map(|e| e.kind), Some(EdgeKind::Jump));
        assert!(nav.edge_between(3, 2).is_none(), "a jump is one way");
        assert_eq!(nav.index_of("deck"), Some(3));
    }

    #[test]
    fn routes_take_the_cheapest_way_and_respect_one_way_edges() {
        let nav = parse_nav(&root(GRAPH)).unwrap().unwrap();
        let name = |p: Vec<usize>| p.iter().map(|i| nav.nodes[*i].id.as_str()).collect::<Vec<_>>().join(">");
        assert_eq!(name(nav.path(0, 3).unwrap()), "a>pad>deck", "the pad beats walking round and jumping");
        assert_eq!(name(nav.path(3, 2).unwrap()), "deck>a>b>c", "down by the drop, then along the floor");
        assert_eq!(name(nav.path(0, 0).unwrap()), "a");
        let lonely = Nav::from_parts(vec![NavNode { id: "x".into(), pos: Vec3::ZERO }, NavNode { id: "y".into(), pos: Vec3::X }], vec![]).unwrap();
        assert_eq!(lonely.path(0, 1), None);
    }

    #[test]
    fn nearest_prefers_the_same_floor() {
        let nav = parse_nav(&root(GRAPH)).unwrap().unwrap();
        assert_eq!(nav.nodes[nav.nearest(Vec3::new(1.0, 0.0, 9.0), 2.5).unwrap()].id, "pad", "the deck is right above but a floor away");
        assert_eq!(nav.nodes[nav.nearest(Vec3::new(1.0, 4.0, 9.0), 2.5).unwrap()].id, "deck");
        assert_eq!(nav.nearest(Vec3::new(0.0, 40.0, 0.0), 2.5), None);
    }

    #[test]
    fn mistakes_name_the_field_and_the_fix() {
        let e = parse_nav(&root(
            r#"{"nav":{"node":[],"nodes":[{"id":"a","pos":[0,0,0]},{"id":"a","pos":[0,0,0]},{"id":"b","pos":[0,0]},{"pos":[0,0,0]}],"edges":[["a","zzz"],["a"],["a","a"]]}}"#,
        ))
        .unwrap_err()
        .join("\n");
        assert!(e.contains("nav.node: unknown field — did you mean `nodes`"), "{e}");
        assert!(e.contains("nav.nodes[1].id: `a` is used twice"), "{e}");
        assert!(e.contains("nav.nodes[2].pos: must be [x, y, z]"), "{e}");
        assert!(e.contains("nav.nodes[3].id: missing"), "{e}");
        assert!(parse_nav(&root(r#"{"nav":{"nodes":[{"id":"a","pos":[0,0,0]}],"edges":[["a","zzz"]]}}"#)).unwrap_err()[0].contains("unknown node \"zzz\""));
        assert!(parse_nav(&root(r#"{"nav":{"nodes":[{"id":"a","pos":[0,0,0]},{"id":"b","pos":[1,0,0]}],"edges":[["a","b","teleport"]]}}"#)).unwrap_err()[0]
            .contains("kind must be"));
        assert!(parse_nav(&root(r#"{"nav":5}"#)).is_err());
        assert!(parse_nav(&root(r#"{"nav":{}}"#)).is_err(), "a graph needs nodes");
    }
}
