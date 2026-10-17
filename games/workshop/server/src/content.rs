//! Workshop's content: definition files of resources, recipes, machine kinds, stores, sources, sinks, machine instances and switches, each
//! under a namespace, read in the order the scene's `workshop` block lists them.
//!
//! ```json
//! {"namespace": "ws",
//!  "resources": ["ore", "ingot", "gear"],
//!  "recipes":   [{"id": "smelt", "in": {"ore": 2}, "out": {"ingot": 1}, "seconds": 2}],
//!  "machines":  [{"id": "furnace", "recipes": ["smelt"], "speed": 1, "input": 6, "output": 4}],
//!  "stores":    [{"id": "storage", "capacity": 40, "start": {"ore": 6}}],
//!  "sources":   [{"id": "mine", "into": "storage", "seconds": 1.5, "gives": {"ore": [1, 2]}}],
//!  "sinks":     [{"id": "ship", "from": "dock", "takes": {"gear": 1}, "seconds": 0.5}],
//!  "instances": [{"id": "furnace_a", "machine": "furnace", "from": "storage", "to": "storage"}],
//!  "switches":  [{"id": "route", "var": "route", "instance": "press_a", "to": ["storage", "dock"]}]}
//! ```
//!
//! A reference without a namespace means the file's own (`"ore"` in namespace `ws` is `ws:ore`); another file's is written in full
//! (`"ws:gear"`). Every definition is registered before any reference is resolved, so files may refer to each other in any order; defining an
//! id twice is refused unless the later definition says `"replace": true`. A machine's `from` names a store or another instance (its output),
//! its `to` a store or another instance (its input). Every problem is a `sim::defs::Diagnostic` at `file#/pointer`.

use red_engine2::sim::clock::TICK_RATE_HZ;
use red_engine2::sim::defs::{closest, code, Defs, Diagnostic, Id, Provenance};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::Path;

/// A recipe: what one cycle consumes and produces, and how long it takes at speed 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Recipe {
    /// Consumed per cycle.
    pub inputs: BTreeMap<Id, u32>,
    /// Produced per cycle.
    pub outputs: BTreeMap<Id, u32>,
    /// Ticks per cycle at speed 1.
    pub ticks: u32,
}

/// A kind of machine: one behavior (`ws:machines`) with its parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct MachineKind {
    /// The recipes it can run (it runs the first).
    pub recipes: Vec<Id>,
    /// Cycles go this many times faster than the recipe's time.
    pub speed: f32,
    /// Its input store's capacity.
    pub input: u32,
    /// Its output store's capacity.
    pub output: u32,
}

/// A shared store.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreDef {
    /// Capacity in units.
    pub capacity: u32,
    /// What it holds at the start (produced by the content's `start` source).
    pub start: BTreeMap<Id, u32>,
}

/// A source: resources from outside the workshop (a mine, a delivery).
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The store it fills.
    pub into: Id,
    /// Ticks between deliveries.
    pub every: u32,
    /// Per delivery, each resource's `[least, most]` (a random amount between them).
    pub gives: BTreeMap<Id, (u32, u32)>,
}

/// A sink: resources leaving the workshop (shipping).
#[derive(Debug, Clone, PartialEq)]
pub struct Sink {
    /// The store it empties.
    pub from: Id,
    /// What one shipment takes.
    pub takes: BTreeMap<Id, u32>,
    /// Ticks between shipments.
    pub every: u32,
}

/// A machine on the floor: a kind and where it takes from and gives to.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Its kind.
    pub machine: Id,
    /// The store it takes inputs from.
    pub from: Id,
    /// The store its outputs go to (a switch may change it).
    pub to: Id,
}

/// A player's choice: a scene variable picks where an instance's output goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// The scene variable that holds the position (0, 1, ...).
    pub var: String,
    /// The instance it routes.
    pub instance: Id,
    /// The stores each position sends the output to.
    pub to: Vec<Id>,
}

/// Every definition of the workshop, resolved.
#[derive(Debug, Clone, Default)]
pub struct Catalog {
    /// Resources.
    pub resources: Defs<()>,
    /// Recipes.
    pub recipes: Defs<Recipe>,
    /// Machine kinds.
    pub machines: Defs<MachineKind>,
    /// Shared stores.
    pub stores: Defs<StoreDef>,
    /// Sources.
    pub sources: Defs<Source>,
    /// Sinks.
    pub sinks: Defs<Sink>,
    /// Machine instances.
    pub instances: Defs<Instance>,
    /// Switches.
    pub switches: Defs<Switch>,
    /// The files, in the order read.
    pub files: Vec<String>,
}

impl Catalog {
    /// The id of instance `inst`'s input store (`ws:furnace_a/in`).
    pub fn input_store(inst: &Id) -> Id {
        Id::parse(&format!("{inst}/in")).unwrap_or(*inst)
    }

    /// The id of instance `inst`'s output store.
    pub fn output_store(inst: &Id) -> Id {
        Id::parse(&format!("{inst}/out")).unwrap_or(*inst)
    }

    /// The id of the source that fills the stores' `start` contents.
    pub fn start_source() -> Id {
        Id::parse("workshop:start").unwrap_or_else(|_| unreachable!("a valid id"))
    }
}

/// The scene's `workshop` block.
#[derive(Debug, Clone)]
pub struct WorkshopSpec {
    /// The definitions.
    pub catalog: std::sync::Arc<Catalog>,
    /// The seed of the workshop's random stream (source amounts).
    pub seed: u64,
    /// Scene variable <- units a sink has taken (what the objective, the HUD and scenarios read).
    pub show: Vec<(String, Id)>,
}

const FILE_KEYS: &[&str] = &["namespace", "resources", "recipes", "machines", "stores", "sources", "sinks", "instances", "switches", "x-note"];

fn unknown_keys(o: &Map<String, Value>, allowed: &[&str], at: &Provenance, errs: &mut Vec<Diagnostic>) {
    for k in o.keys().filter(|k| !allowed.contains(&k.as_str()) && !k.starts_with("x-")) {
        let d = Diagnostic::new("unknown-field", at.at(k), format!("unknown field `{k}`"));
        errs.push(match closest(k, allowed.iter().copied()) {
            Some(n) => d.fix(format!("did you mean `{n}`?")),
            None => d.fix(format!("one of: {}", allowed.join(", "))),
        });
    }
}

fn number(o: &Map<String, Value>, key: &str, at: &Provenance, errs: &mut Vec<Diagnostic>) -> Option<f64> {
    match o.get(key) {
        Some(v) => match v.as_f64().filter(|n| *n > 0.0 && n.is_finite()) {
            Some(n) => Some(n),
            None => {
                errs.push(Diagnostic::new("out-of-range", at.at(key), format!("must be a number above 0, got {v}")));
                None
            }
        },
        None => {
            errs.push(Diagnostic::new("missing-field", at.at(key), format!("`{key}` is required")));
            None
        }
    }
}

fn ticks(secs: f64) -> u32 {
    ((secs * TICK_RATE_HZ as f64).round() as u32).max(1)
}

/// One file's definitions before references are resolved: `(id, provenance, raw object, replace)` per kind.
type Raw = (Id, Provenance, Map<String, Value>, bool);

/// Reads the block's files (relative to `base`), in order, into a resolved [`Catalog`]; or every problem found.
pub fn load(files: &[String], base: Option<&Path>) -> Result<Catalog, Vec<Diagnostic>> {
    let mut errs = Vec::new();
    let mut cat = Catalog { files: files.to_vec(), ..Default::default() };
    // Pass 1: every definition of every file, by kind, with its namespace.
    let mut raw: BTreeMap<&str, Vec<(String, Raw)>> = BTreeMap::new();
    for file in files {
        let path = base.map_or_else(|| Path::new(file).to_path_buf(), |b| b.join(file));
        let root_at = Provenance::new(file, "");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                errs.push(Diagnostic::new("io", &root_at, format!("cannot read {}: {e}", path.display())));
                continue;
            }
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                errs.push(Diagnostic::new("json-syntax", &root_at, e.to_string()));
                continue;
            }
        };
        let Some(o) = v.as_object() else {
            errs.push(Diagnostic::new("wrong-type", &root_at, "a definitions file is an object"));
            continue;
        };
        unknown_keys(o, FILE_KEYS, &root_at, &mut errs);
        let Some(ns) = o.get("namespace").and_then(Value::as_str) else {
            errs.push(Diagnostic::new("missing-field", root_at.at("namespace"), "every definitions file names its namespace (`\"namespace\": \"ws\"`)"));
            continue;
        };
        if let Some(list) = o.get("resources") {
            for (i, r) in list.as_array().into_iter().flatten().enumerate() {
                let at = root_at.at("resources").at(i);
                match r.as_str().map(|r| Id::resolve(ns, r)) {
                    Some(Ok(id)) => {
                        if let Err(e) = cat.resources.define(id, (), at) {
                            errs.push(e);
                        }
                    }
                    Some(Err(e)) => errs.push(Diagnostic::new(code::ID_FORM, at, e)),
                    None => errs.push(Diagnostic::new("wrong-type", at, "a resource is a name")),
                }
            }
        }
        for kind in ["recipes", "machines", "stores", "sources", "sinks", "instances", "switches"] {
            for (i, d) in o.get(kind).and_then(Value::as_array).into_iter().flatten().enumerate() {
                let at = root_at.at(kind).at(i);
                let Some(d) = d.as_object() else {
                    errs.push(Diagnostic::new("wrong-type", at, "a definition is an object with an `id`"));
                    continue;
                };
                match d.get("id").and_then(Value::as_str).map(|s| Id::resolve(ns, s)) {
                    Some(Ok(id)) => {
                        let replace = d.get("replace").and_then(Value::as_bool).unwrap_or(false);
                        raw.entry(kind).or_default().push((ns.to_string(), (id, at, d.clone(), replace)));
                    }
                    Some(Err(e)) => errs.push(Diagnostic::new(code::ID_FORM, at.at("id"), e)),
                    None => errs.push(Diagnostic::new("missing-field", at.at("id"), "every definition has an `id`")),
                }
            }
        }
    }
    // Pass 2, kind by kind (each only refers to kinds before it): parse, resolve, define (or replace on purpose).
    fn put<T>(defs: &mut Defs<T>, id: Id, def: T, at: Provenance, replace: bool, errs: &mut Vec<Diagnostic>) {
        if let Err(e) = if replace { defs.replace(id, def, at) } else { defs.define(id, def, at) } {
            errs.push(e);
        }
    }
    let resource_map = |cat: &Catalog, ns: &str, v: Option<&Value>, at: &Provenance, errs: &mut Vec<Diagnostic>| -> BTreeMap<Id, u32> {
        let mut out = BTreeMap::new();
        for (k, n) in v.and_then(Value::as_object).into_iter().flatten() {
            let kat = at.at(k);
            let Ok(id) = cat.resources.reference(ns, k, "resource", &kat).map_err(|e| errs.push(e)) else { continue };
            match n.as_u64().filter(|n| *n > 0 && *n <= 10_000) {
                Some(n) => {
                    out.insert(id, n as u32);
                }
                None => errs.push(Diagnostic::new("out-of-range", kat, format!("a count from 1 to 10000, got {n}"))),
            }
        }
        out
    };
    for (ns, (id, at, d, replace)) in raw.remove("recipes").unwrap_or_default() {
        unknown_keys(&d, &["id", "in", "out", "seconds", "replace"], &at, &mut errs);
        let inputs = resource_map(&cat, &ns, d.get("in"), &at.at("in"), &mut errs);
        let outputs = resource_map(&cat, &ns, d.get("out"), &at.at("out"), &mut errs);
        if inputs.is_empty() && outputs.is_empty() {
            errs.push(Diagnostic::new("missing-field", &at, "a recipe consumes or produces something (`in`, `out`)"));
        }
        let secs = number(&d, "seconds", &at, &mut errs).unwrap_or(1.0);
        put(&mut cat.recipes, id, Recipe { inputs, outputs, ticks: ticks(secs) }, at, replace, &mut errs);
    }
    for (ns, (id, at, d, replace)) in raw.remove("machines").unwrap_or_default() {
        unknown_keys(&d, &["id", "recipes", "speed", "input", "output", "replace"], &at, &mut errs);
        let recipes: Vec<Id> = d
            .get("recipes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, r)| cat.recipes.reference(&ns, r.as_str().unwrap_or(""), "recipe", &at.at("recipes").at(i)).map_err(|e| errs.push(e)).ok())
            .collect();
        if recipes.is_empty() && !errs.iter().any(|e| e.at.starts_with(&at.to_string())) {
            errs.push(Diagnostic::new("missing-field", at.at("recipes"), "a machine runs at least one recipe"));
        }
        let speed = number(&d, "speed", &at, &mut errs).unwrap_or(1.0) as f32;
        let input = number(&d, "input", &at, &mut errs).unwrap_or(1.0) as u32;
        let output = number(&d, "output", &at, &mut errs).unwrap_or(1.0) as u32;
        for r in &recipes {
            let rec = cat.recipes.get(r).cloned().unwrap_or(Recipe { inputs: BTreeMap::new(), outputs: BTreeMap::new(), ticks: 1 });
            let (need_in, need_out) = (rec.inputs.values().sum::<u32>(), rec.outputs.values().sum::<u32>());
            if need_in > input || need_out > output {
                errs.push(
                    Diagnostic::new(
                        "out-of-range",
                        &at,
                        format!("`{r}` needs {need_in} in and {need_out} out per cycle; this machine holds {input} in and {output} out"),
                    )
                    .fix("raise `input`/`output` to at least one cycle's worth"),
                );
            }
        }
        put(&mut cat.machines, id, MachineKind { recipes, speed, input, output }, at, replace, &mut errs);
    }
    for (ns, (id, at, d, replace)) in raw.remove("stores").unwrap_or_default() {
        unknown_keys(&d, &["id", "capacity", "start", "replace"], &at, &mut errs);
        let capacity = number(&d, "capacity", &at, &mut errs).unwrap_or(1.0) as u32;
        let start = resource_map(&cat, &ns, d.get("start"), &at.at("start"), &mut errs);
        if start.values().sum::<u32>() > capacity {
            errs.push(Diagnostic::new(code::FULL, at.at("start"), format!("starts with more than its capacity ({capacity})")));
        }
        put(&mut cat.stores, id, StoreDef { capacity, start }, at, replace, &mut errs);
    }
    // Instances may name each other: register their ids first.
    let instance_raw = raw.remove("instances").unwrap_or_default();
    let instance_ids: Vec<Id> = instance_raw.iter().map(|(_, (id, ..))| *id).collect();
    let place = |cat: &Catalog, ns: &str, v: Option<&Value>, at: &Provenance, input: bool, errs: &mut Vec<Diagnostic>| -> Option<Id> {
        let name = v.and_then(Value::as_str).unwrap_or("");
        let id = Id::resolve(ns, name).map_err(|e| errs.push(Diagnostic::new(code::ID_FORM, at, e))).ok()?;
        if cat.stores.contains(&id) {
            return Some(id);
        }
        if instance_ids.contains(&id) {
            return Some(if input { Catalog::input_store(&id) } else { Catalog::output_store(&id) });
        }
        let names = cat.stores.iter().map(|(i, _)| i.as_str()).chain(instance_ids.iter().map(Id::as_str));
        let d = Diagnostic::new(code::UNKNOWN, at, format!("no store or machine `{id}`"));
        errs.push(match closest(id.as_str(), names) {
            Some(n) => d.fix(format!("did you mean `{n}`?")),
            None => d,
        });
        None
    };
    for (ns, (id, at, d, replace)) in raw.remove("sources").unwrap_or_default() {
        unknown_keys(&d, &["id", "into", "seconds", "gives", "replace"], &at, &mut errs);
        let into = place(&cat, &ns, d.get("into"), &at.at("into"), true, &mut errs);
        let every = ticks(number(&d, "seconds", &at, &mut errs).unwrap_or(1.0));
        let mut gives = BTreeMap::new();
        for (k, range) in d.get("gives").and_then(Value::as_object).into_iter().flatten() {
            let kat = at.at("gives").at(k);
            let Ok(res) = cat.resources.reference(&ns, k, "resource", &kat).map_err(|e| errs.push(e)) else { continue };
            let pair = match range {
                Value::Number(n) => n.as_u64().map(|n| (n, n)),
                Value::Array(a) if a.len() == 2 => a[0].as_u64().zip(a[1].as_u64()),
                _ => None,
            };
            match pair.filter(|(lo, hi)| lo <= hi && *hi <= 1000) {
                Some((lo, hi)) => {
                    gives.insert(res, (lo as u32, hi as u32));
                }
                None => errs.push(Diagnostic::new("wrong-type", kat, "a count, or [least, most] with least <= most <= 1000")),
            }
        }
        if let Some(into) = into {
            put(&mut cat.sources, id, Source { into, every, gives }, at, replace, &mut errs);
        }
    }
    for (ns, (id, at, d, replace)) in raw.remove("sinks").unwrap_or_default() {
        unknown_keys(&d, &["id", "from", "takes", "seconds", "replace"], &at, &mut errs);
        let from = place(&cat, &ns, d.get("from"), &at.at("from"), false, &mut errs);
        let takes = resource_map(&cat, &ns, d.get("takes"), &at.at("takes"), &mut errs);
        let every = ticks(number(&d, "seconds", &at, &mut errs).unwrap_or(1.0));
        if let Some(from) = from {
            put(&mut cat.sinks, id, Sink { from, takes, every }, at, replace, &mut errs);
        }
    }
    for (ns, (id, at, d, replace)) in instance_raw {
        unknown_keys(&d, &["id", "machine", "from", "to", "replace"], &at, &mut errs);
        let machine =
            cat.machines.reference(&ns, d.get("machine").and_then(Value::as_str).unwrap_or(""), "machine", &at.at("machine")).map_err(|e| errs.push(e)).ok();
        let from = place(&cat, &ns, d.get("from"), &at.at("from"), false, &mut errs);
        let to = place(&cat, &ns, d.get("to"), &at.at("to"), true, &mut errs);
        if let (Some(machine), Some(from), Some(to)) = (machine, from, to) {
            put(&mut cat.instances, id, Instance { machine, from, to }, at, replace, &mut errs);
        }
    }
    for (ns, (id, at, d, replace)) in raw.remove("switches").unwrap_or_default() {
        unknown_keys(&d, &["id", "var", "instance", "to", "replace"], &at, &mut errs);
        let var = d.get("var").and_then(Value::as_str).unwrap_or("").to_string();
        if var.is_empty() {
            errs.push(Diagnostic::new("missing-field", at.at("var"), "the scene variable that holds the switch's position"));
        }
        let instance = cat
            .instances
            .reference(&ns, d.get("instance").and_then(Value::as_str).unwrap_or(""), "machine instance", &at.at("instance"))
            .map_err(|e| errs.push(e))
            .ok();
        let to: Vec<Id> = d
            .get("to")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(i, t)| place(&cat, &ns, Some(t), &at.at("to").at(i), true, &mut errs))
            .collect();
        if to.len() < 2 {
            errs.push(Diagnostic::new("missing-field", at.at("to"), "a switch chooses between at least two places"));
        }
        if let Some(instance) = instance {
            put(&mut cat.switches, id, Switch { var, instance, to }, at, replace, &mut errs);
        }
    }
    if errs.is_empty() {
        Ok(cat)
    } else {
        Err(errs)
    }
}

/// Parses the scene's `workshop` block (`Ok(None)` without one): its `defs` (files, read in order), `seed` and `show` (scene variable <- sink),
/// checking every scene variable it names against the scene's `vars`.
pub fn parse_block(root: &Map<String, Value>, base: Option<&Path>) -> Result<Option<WorkshopSpec>, Vec<String>> {
    let Some(v) = root.get("workshop") else { return Ok(None) };
    let at = Provenance::new("scene", "/workshop");
    let mut errs: Vec<Diagnostic> = Vec::new();
    let Some(o) = v.as_object() else { return Err(vec!["workshop: must be an object like {\"defs\": [\"defs/base.json\"]}".into()]) };
    unknown_keys(o, &["defs", "seed", "show"], &at, &mut errs);
    let files: Vec<String> = o.get("defs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    if files.is_empty() {
        errs.push(Diagnostic::new("missing-field", at.at("defs"), "the definition files to read, in order (`[\"defs/base.json\"]`)"));
    }
    let catalog = match load(&files, base) {
        Ok(c) => Some(c),
        Err(e) => {
            errs.extend(e);
            None
        }
    };
    let declared: Vec<String> = root.get("vars").and_then(Value::as_object).map(|m| m.keys().cloned().collect()).unwrap_or_default();
    let check_var = |var: &str, at: Provenance, errs: &mut Vec<Diagnostic>| {
        if !declared.iter().any(|d| d == var) {
            let d = Diagnostic::new(code::UNKNOWN, at, format!("no scene variable `{var}`")).fix(format!("declare it in the scene's `vars` (`\"{var}\": 0`)"));
            errs.push(match closest(var, declared.iter().map(String::as_str)) {
                Some(n) => d.fix(format!("did you mean `{n}`?")),
                None => d,
            });
        }
    };
    let mut show = Vec::new();
    for (var, sink) in o.get("show").and_then(Value::as_object).into_iter().flatten() {
        let sat = at.at("show").at(var);
        check_var(var, sat.clone(), &mut errs);
        match (&catalog, sink.as_str().map(Id::parse)) {
            (Some(c), Some(Ok(id))) if c.sinks.contains(&id) => show.push((var.clone(), id)),
            (Some(c), Some(Ok(id))) => errs
                .push(c.sinks.reference(id.namespace(), id.name(), "sink", &sat).err().unwrap_or_else(|| Diagnostic::new(code::UNKNOWN, &sat, "no such sink"))),
            (_, Some(Err(e))) => errs.push(Diagnostic::new(code::ID_FORM, &sat, e)),
            (_, None) => errs.push(Diagnostic::new("wrong-type", &sat, "the sink whose shipments the variable counts (`\"ws:ship\"`)")),
            (None, _) => {}
        }
    }
    if let Some(c) = &catalog {
        for (id, s) in c.switches.iter() {
            let sat = c.switches.provenance(id).cloned().unwrap_or_default().at("var");
            check_var(&s.var, sat, &mut errs);
        }
    }
    let seed = o.get("seed").and_then(Value::as_u64).unwrap_or(1);
    match (errs.is_empty(), catalog) {
        (true, Some(catalog)) => Ok(Some(WorkshopSpec { catalog: std::sync::Arc::new(catalog), seed, show })),
        _ => Err(errs.iter().map(|e| format!("workshop: {e}")).collect()),
    }
}
