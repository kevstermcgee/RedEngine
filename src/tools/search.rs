//! `red_engine2 search <words>`: one query across everything an AI could need — the docs
//! (SPEC/AGENTS/glossary chunked by heading, ADRs whole), the asset catalogue, lint codes, conventions, recipes,
//! CLI commands and the Rust symbol table — returning only the few best *fragments*, each with
//! where to read more. BM25-style scoring (IDF-weighted, title/tag boosts, light stemming and a
//! small synonym table): no model download, no vector store, deterministic, milliseconds.
//!
//! Embeddings were deliberately skipped: for a corpus this small and this jargon-heavy, weighted
//! keyword search with synonyms finds the right section reliably, and adds zero dependencies.

use super::{catalog, describe, features, recipes, symbols};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

use super::own_docs::{self, Docs};

/// One searchable fragment: kind, title, body, where to read more, and boosted tokens.
pub struct Doc {
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    /// Where to read more (file:line, or a command to run).
    pub loc: String,
    /// Tokens weighted 3x (tags, names).
    pub extra: String,
}

const STOP: &[&str] = &[
    "the", "a", "an", "of", "to", "in", "is", "it", "and", "or", "for", "on", "how", "do", "i", "can", "what", "with", "my", "be", "are", "does", "make",
    "get", "use", "add", "want", "need",
];

const SYNONYMS: &[(&str, &str)] = &[
    ("test", "scenario scenarios verify expect"),
    ("collide", "collision collider blocks block solid"),
    ("collision", "collide collider blocks solid walk-through"),
    ("block", "collision collider solid"),
    ("stair", "stairs staircase steps rise run landing"),
    ("door", "doorway opening header"),
    ("window", "opening glass sill"),
    ("light", "lamp lights point sun ambient shadow"),
    ("lamp", "light point"),
    ("floor", "slab ground plane level upstairs"),
    ("upstairs", "floor slab stairs level"),
    ("template", "prefab prefabs"),
    ("reusable", "prefab prefabs"),
    ("wall", "walls opening thickness"),
    ("fence", "perimeter gate posts leak"),
    ("fall", "drop railing"),
    ("stuck", "unreachable walk blocked"),
    ("reach", "reachable unreachable walk"),
    ("spawn", "camera start player"),
    ("arena", "movement sprint jump pad launch fast fov"),
    ("jump-pad", "launch pad vertical velocity arena movement"),
    ("color", "material hex emissive"),
    ("glow", "emissive light"),
    ("sound", "audio"),
    ("size", "dimensions bounds"),
    ("furniture", "prop prefab chair table sofa"),
    ("test", "verify checks golden"),
    ("screenshot", "frame tour plan render view"),
    ("picture", "frame tour plan render view png"),
    ("image", "frame tour plan render png"),
    ("place", "position add move"),
    ("delete", "rm remove"),
    ("copy", "clone array duplicate"),
    ("random", "scatter seed"),
    ("tree", "scatter landscaping plant"),
    ("room", "zone wall floor"),
    ("map", "scene json level"),
    ("level", "map scene floor"),
    ("server", "red_server multiplayer authoritative udp headless host"),
    ("multiplayer", "red_server red_bot server udp online"),
    ("encryption", "quic tls transport fingerprint identity net-identity"),
    ("encrypt", "quic tls transport fingerprint identity"),
    ("tls", "quic transport fingerprint identity certificate"),
    ("certificate", "net-identity fingerprint tls quic"),
    ("online", "multiplayer server connect"),
    ("bot", "red_bot scripted client headless"),
    ("client", "connect re2 red_bot multiplayer"),
    ("connect", "server join client udp multiplayer"),
    ("coin", "collectible pickup score rules vars"),
    ("collectible", "coin pickup score rules trigger"),
    ("score", "vars rules variable add"),
    ("win", "victory end rules condition goal"),
    ("trigger", "rules enter exit zone volume event"),
    ("rule", "rules vars when if do actions"),
    ("gameplay", "rules scenario sim checks"),
    ("damage", "health hp hit shot weapons interact combat"),
    ("shoot", "firearm pistol hitscan attack weapon damage"),
    ("weapon", "firearm pistol bat attack ammo weapons ladder"),
    ("ammo", "reload magazine weapons firearm"),
    ("replay", "trace checksum desync sim record determinism"),
    ("desync", "replay trace checksum diverge"),
    ("record", "trace replay red_server"),
    ("typo", "unknown field did you mean validate strict"),
    ("json", "envelope machine-readable diagnostics"),
    ("bandwidth", "interest snapshot budget packet"),
    ("interest", "rooms portals zones snapshot bandwidth"),
];

fn stem(w: &str) -> String {
    let w = w.to_lowercase();
    for suf in ["ing", "ies", "es", "ed", "s"] {
        if w.len() > suf.len() + 2 && w.ends_with(suf) {
            return if suf == "ies" { format!("{}y", &w[..w.len() - 3]) } else { w[..w.len() - suf.len()].to_string() };
        }
    }
    w
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|w| w.len() > 1)
        .flat_map(|w| {
            // split snake/camel-ish joined names too: chair_folding -> chair, folding
            let mut v = vec![stem(w)];
            if w.contains('-') {
                v.extend(w.split('-').filter(|p| p.len() > 1).map(stem));
            }
            v
        })
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

fn tokens_snake(s: &str) -> Vec<String> {
    tokens(&s.replace('_', " "))
}

fn chunk_markdown(file: &str, kind: &'static str, text: &str, out: &mut Vec<Doc>) {
    let lines: Vec<&str> = text.lines().collect();
    let mut heading = String::from("(top)");
    let mut start = 0usize;
    let mut sec: Vec<&str> = Vec::new();
    let flush = |heading: &str, start: usize, sec: &mut Vec<&str>, out: &mut Vec<Doc>| {
        let body: Vec<&str> = std::mem::take(sec);
        if body.iter().all(|l| l.trim().is_empty()) {
            return;
        }
        for (k, w) in body.chunks(28).enumerate() {
            out.push(Doc { kind, title: heading.to_string(), body: w.join("\n"), loc: format!("{file}:{}", start + k * 28 + 1), extra: String::new() });
        }
    };
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with('#') && !l.starts_with("#!") {
            flush(&heading, start, &mut sec, out);
            heading = l.trim_start_matches('#').trim().to_string();
            start = i;
        }
        sec.push(l);
    }
    flush(&heading, start, &mut sec, out);
}

/// The title line of the ADR whose file name is `file` (`0029-match-flow-lobby-rounds-rematch.md`), without the leading `#`.
pub fn adr_title(file: &str) -> Option<String> {
    own_docs::docs().adrs.iter().find(|(f, _)| f == file).and_then(|(_, t)| t.lines().next()).map(|l| l.trim_start_matches('#').trim().to_string())
}

/// Builds the whole search corpus (docs, glossary, ADRs, assets, lint codes, recipes, commands, public symbols) from the engine's own documents
/// as this process reads them ([`own_docs::docs`]: the checkout's, so a new ADR is found with no rebuild).
pub fn corpus(commands: &Value) -> Vec<Doc> {
    corpus_with(own_docs::docs(), commands)
}

/// [`corpus`] over a given set of documents.
pub fn corpus_with(own: &Docs, commands: &Value) -> Vec<Doc> {
    let mut docs = Vec::new();
    chunk_markdown("SPEC.md", "doc", &own.spec, &mut docs);
    chunk_markdown("AGENTS.md", "doc", &own.agents, &mut docs);
    chunk_markdown("docs/MAKING_GAMES.md", "doc", &own.making_games, &mut docs);
    chunk_markdown("docs/ENGINE_WORK.md", "doc", &own.engine_work, &mut docs);
    chunk_markdown("docs/AGENT_REFERENCE.md", "doc", &own.agent_reference, &mut docs);
    chunk_markdown("docs/GLOSSARY.md", "glossary", &own.glossary, &mut docs);
    // The 2D path: a question about a 2D game, a sprite, a scenario or playing it natively must find the 2D material, not scene objects (a fresh author's first `search` did).
    chunk_markdown("docs/PLAY_2D.md", "doc", &own.play_2d, &mut docs);
    // Narrative: a question that names a story, dialogue, a bark or a character finds the guide and its contract; it is a domain of its own
    // (kind `story`), so its words ("recipes", "state", "workflow") never take an ordinary answer's place (`query_names_story`).
    chunk_markdown("docs/STORY.md", "story", &own.story, &mut docs);
    for (k, group) in red2d::reference::REFERENCE.lines().collect::<Vec<_>>().chunks(2).enumerate() {
        docs.push(Doc {
            kind: "doc",
            title: "2D game file format (describe 2d)".to_string(),
            body: format!("2D game .game2d.json sprites prefabs rules scenarios\n{}", group.join("\n")),
            loc: format!("describe 2d:{}", k * 2 + 1),
            extra: String::new(),
        });
    }
    for (file, text) in &own.adrs {
        // One doc per ADR (they are short): the title line names the decision, the rest is the body.
        let title = text.lines().next().unwrap_or("").trim_start_matches('#').trim();
        docs.push(Doc {
            kind: "adr",
            title: format!("ADR {title}"),
            body: text.lines().skip(1).collect::<Vec<_>>().join("\n"),
            loc: format!("docs/adr/{file}"),
            extra: title.to_string(),
        });
    }
    for (file, text) in &own.analyses {
        chunk_markdown(&format!("docs/analysis/{file}"), "analysis", text, &mut docs);
    }
    for e in catalog::entries() {
        docs.push(Doc {
            kind: "asset",
            title: format!("{} ({}, {})", e.name, e.kind, e.category),
            body: format!(
                "{} — {:.2}x{:.2}x{:.2} m; tags: {}; aliases: {}; roles: {}; styles: {}; status: {}",
                e.desc,
                e.size().x,
                e.size().y,
                e.size().z,
                e.tags.join(" "),
                e.aliases.join(" "),
                e.roles.join(" "),
                e.styles.join(" "),
                e.status
            ),
            loc: format!("red_engine2 catalog {}", e.name),
            extra: format!("{} {} {} {}", e.name.replace('_', " "), e.tags.join(" "), e.aliases.join(" "), e.roles.join(" ")),
        });
    }
    for (c, sev, d) in describe::LINT_CODES {
        docs.push(Doc {
            kind: "lint",
            title: format!("lint code `{c}` ({sev})"),
            body: d.to_string(),
            loc: "red_engine2 describe lint".into(),
            extra: c.replace('-', " "),
        });
    }
    for (k, d) in describe::CONVENTIONS {
        docs.push(Doc {
            kind: "rule",
            title: format!("convention: {k}"),
            body: d.to_string(),
            loc: "red_engine2 describe conventions".into(),
            extra: String::new(),
        });
    }
    for t in describe::TYPES {
        docs.push(Doc {
            kind: "type",
            title: format!("object type `{}`", t.name),
            body: format!("{}\n{}", t.summary, t.fields.iter().map(|(n, ty, d)| format!("{n} {ty} {d}")).collect::<Vec<_>>().join("\n")),
            loc: "red_engine2 describe objects".into(),
            extra: t.name.to_string(),
        });
    }
    for r in recipes::all() {
        docs.push(Doc {
            kind: "recipe",
            title: format!("recipe `{}`: {}", r.name, r.title()),
            body: format!(
                "{} Teaches: {}",
                r.summary(),
                r.json["recipe"]["teaches"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default()
            ),
            loc: format!("red_engine2 recipe {}", r.name),
            extra: r.name.replace('_', " "),
        });
    }
    for p in recipes::patterns() {
        docs.push(Doc {
            kind: "mechanic",
            title: format!("2D mechanic `{}`: {}", p.name, p.title()),
            body: format!("{} Rules: {}. Scenarios: {}.", p.description(), p.rules().join("; "), p.scenarios().join("; ")),
            loc: format!("red_engine2 recipe {}", p.name),
            extra: format!("{} 2d", p.name.replace('-', " ")),
        });
    }
    for c in commands.as_array().into_iter().flatten() {
        let name = c["name"].as_str().unwrap_or("?");
        let flags: Vec<String> =
            c["args"].as_array().into_iter().flatten().map(|a| format!("{} {}", a["name"].as_str().unwrap_or(""), a["help"].as_str().unwrap_or(""))).collect();
        docs.push(Doc {
            kind: "command",
            title: format!("command `{name}`"),
            body: format!("{}\n{}", c["about"].as_str().unwrap_or(""), flags.join("\n")),
            loc: format!("red_engine2 {name} --help"),
            extra: name.to_string(),
        });
    }
    if let Some(root) = symbols::find_root() {
        let ix = symbols::Index::build(&root);
        for s in ix.symbols.iter().filter(|s| !s.in_tests && s.public && !matches!(s.kind, "impl" | "mod")) {
            docs.push(Doc {
                kind: "src",
                title: s.sig.clone(),
                body: s.doc.clone(),
                loc: format!("{}:{}  (red_engine2 src show {})", s.file, s.line, s.name),
                extra: format!("{} {}", s.name.replace('_', " "), s.container.clone().unwrap_or_default()),
            });
        }
    }
    docs
}

/// A scored search result with the best-matching fragment.
pub struct Hit<'a> {
    pub doc: &'a Doc,
    pub score: f32,
    pub fragment: String,
}

fn expand(query: &str) -> Vec<(String, f32)> {
    let mut out: Vec<(String, f32)> = Vec::new();
    for t in tokens_snake(query) {
        out.push((t.clone(), 1.0));
        for (k, syn) in SYNONYMS {
            if stem(k) == t {
                out.extend(tokens_snake(syn).into_iter().map(|s| (s, 0.35)));
            }
        }
    }
    out
}

/// How much a hit's source file matters to someone building their own game: engine files 1.0, one game's code or content, legacy fixtures and past
/// reports less (`content` in `docs/features.json`). A query that names the game still finds it: the penalty only reorders, it never hides.
fn content_bias(d: &Doc) -> f32 {
    let file = d.loc.split([':', ' ']).next().unwrap_or_default();
    match features::content_tier(file) {
        Some("game") => 0.75,
        Some("legacy") => 0.7,
        Some("history") => 0.8,
        _ => 1.0,
    }
}

/// The domain a query names outright: `Some(true)` for 2D ("2d", "sprite", "platformer", "game2d"), `Some(false)` for a 3D scene ("3d", "scene", "room",
/// "first person", ...), `None` when it says neither (ADR 2026-10-11-search-leads-with-the-domain-a-query-names).
fn query_domain(query: &str) -> Option<bool> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |set: &[&str]| words.iter().any(|w| set.contains(w));
    let two = has(&["2d", "sprite", "sprites", "platformer", "game2d", "pixel", "topdown"]);
    let three = has(&["3d", "scene", "scenes", "room", "rooms", "floor", "floors", "stairs", "fps", "walkable"])
        || q.contains("first person")
        || q.contains("first-person");
    match (two, three) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    }
}

/// Whether a query names the narrative domain (a story, dialogue, a bark, a character, a conversation): only then do the story guide's fragments
/// compete for its answer (ADR 2026-10-15-narrative-is-compiled-content-run-by-one-story-system).
fn query_names_story(query: &str) -> bool {
    const WORDS: &[&str] = &[
        "story",
        "stories",
        "dialogue",
        "dialogues",
        "dialog",
        "bark",
        "barks",
        "conversation",
        "conversations",
        "character",
        "characters",
        "narrative",
        "speaker",
        "speakers",
        "npc",
        "npcs",
        "subtitle",
        "subtitles",
        "director",
        "storytelling",
        "lines",
        "choices",
    ];
    query.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).any(|w| WORDS.contains(&w))
}

/// Ranks the corpus for `query`, optionally only one `kind`, returning at most `limit` hits (diversified per kind), preceded by a lead lane of at most two
/// complete examples when the query clearly asks for one: verified 2D mechanics and 3D recipes, in the domain the query names (a 3D query gets no 2D
/// mechanic, a 2D query no 3D recipe; with neither, the best of each). Mechanics never take an ordinary slot; a recipe in the lane is not repeated below.
pub fn search<'a>(docs: &'a [Doc], query: &str, kind: Option<&str>, limit: usize) -> Vec<Hit<'a>> {
    let q = expand(query);
    if q.is_empty() {
        return Vec::new();
    }
    let toks: Vec<(Vec<String>, Vec<String>, Vec<String>)> =
        docs.iter().map(|d| (tokens_snake(&d.title), tokens_snake(&d.body), tokens_snake(&d.extra))).collect();
    // The statistics (how common a word is, how many documents there are) leave the mechanics out, so that having them in the corpus cannot nudge any other result's score.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for (d, (t, b, e)) in docs.iter().zip(&toks) {
        if d.kind == "mechanic" || d.kind == "story" {
            continue;
        }
        let set: HashSet<&str> = t.iter().chain(b).chain(e).map(String::as_str).collect();
        for w in set {
            *df.entry(w).or_default() += 1;
        }
    }
    let n = docs.iter().filter(|d| d.kind != "mechanic" && d.kind != "story").count() as f32;
    let story = query_names_story(query);
    let mut hits: Vec<Hit> = Vec::new();
    // Verified 2D mechanics (`recipes::patterns`) are a lane of their own: they are tiny, dense documents that would outrank the real answer to a query from another domain
    // ("player shoots another player damage health" is a 3D multiplayer question), so they never take an ordinary slot; the best one or two are shown first when the query
    // clearly asks for one (at least two of the query's words are words that single out that mechanic).
    let mut lane: Vec<(usize, f32, Hit)> = Vec::new();
    // A word that occurs in nearly every mechanic ("game", "player", "2d") says nothing about which one is wanted: only words found in at most three of them count.
    let mut mech_df: HashMap<&str, usize> = HashMap::new();
    // The same test for the 3D recipes: a word in at most two of them singles one out ("key", "gate"), "room" or "door" does not.
    let mut recipe_df: HashMap<&str, usize> = HashMap::new();
    for (d, (t, b, e)) in docs.iter().zip(&toks) {
        let df = match d.kind {
            "mechanic" => &mut mech_df,
            "recipe" => &mut recipe_df,
            _ => continue,
        };
        for w in t.iter().chain(b).chain(e).map(String::as_str).collect::<HashSet<_>>() {
            *df.entry(w).or_default() += 1;
        }
    }
    let domain = query_domain(query);
    let mut recipe_lane: Vec<(usize, f32, Hit)> = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        if kind.is_some_and(|k| d.kind != k && !(k == "recipe" && d.kind == "mechanic")) {
            continue;
        }
        if d.kind == "story" && kind.is_none() && !story {
            continue;
        }
        let (t, b, e) = &toks[i];
        let mut score = 0.0;
        let mut matched_terms = 0;
        for (term, weight) in &q {
            let tf_t = t.iter().filter(|x| *x == term).count() as f32;
            let tf_e = e.iter().filter(|x| *x == term).count() as f32;
            let tf_b = b.iter().filter(|x| *x == term).count() as f32;
            let tf = tf_t * 4.0 + tf_e * 3.0 + tf_b;
            if tf == 0.0 {
                continue;
            }
            if *weight >= 1.0 {
                matched_terms += 1;
            }
            let idf = ((n - *df.get(term.as_str()).unwrap_or(&0) as f32 + 0.5) / (*df.get(term.as_str()).unwrap_or(&0) as f32 + 0.5) + 1.0).ln();
            let len_norm = 1.0 - 0.5 + 0.5 * (b.len() as f32 + 4.0) / 60.0;
            score += weight * idf * (tf * 2.2) / (tf + 1.2 * len_norm);
        }
        if score > 0.0 {
            // reward covering more of the distinct query words
            let distinct = q.iter().filter(|(_, w)| *w >= 1.0).count().max(1) as f32;
            score *= 1.0 + 0.6 * (matched_terms as f32 / distinct);
            let kind_bias = match d.kind {
                "doc" | "rule" | "recipe" | "adr" | "glossary" | "story" => 1.15,
                "asset" | "lint" | "type" | "command" => 1.05,
                _ => 0.9,
            };
            let hit = Hit { doc: d, score: score * kind_bias * content_bias(d), fragment: fragment(d, &q) };
            let distinctive = |df: &HashMap<&str, usize>, most: usize| {
                q.iter()
                    .filter(|(term, w)| *w >= 1.0 && df.get(term.as_str()).is_some_and(|n| *n <= most) && t.iter().chain(b).chain(e).any(|x| x == term))
                    .count()
            };
            if d.kind == "mechanic" && kind.is_none() {
                let n = distinctive(&mech_df, 3);
                if n >= 2 && domain != Some(false) {
                    lane.push((n, hit.score, hit));
                }
            } else if d.kind == "recipe" && kind.is_none() && domain != Some(true) && distinctive(&recipe_df, 2) >= 2 {
                recipe_lane.push((distinctive(&recipe_df, 2), hit.score, hit));
            } else {
                hits.push(hit);
            }
        }
    }
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    lane.sort_by(|a, b| (b.0, b.1).partial_cmp(&(a.0, a.1)).unwrap());
    recipe_lane.sort_by(|a, b| (b.0, b.1).partial_cmp(&(a.0, a.1)).unwrap());
    // The lead: two of the named domain's examples, or the best mechanic then the best recipe when the query names no domain. A recipe left out of the lead
    // goes back among the ordinary hits, ranked by its score.
    // The split never depends on whether a mechanic matched, so the ordinary results are the same with or without the mechanics in the corpus.
    let (take_mechanics, take_recipes) = match domain {
        Some(true) => (2, 0),
        Some(false) => (0, 2),
        None => (1, 1),
    };
    let mut lead: Vec<Hit> = lane.into_iter().take(take_mechanics).map(|(_, _, h)| h).collect();
    let mut recipes = recipe_lane.into_iter().map(|(_, _, h)| h);
    lead.extend(recipes.by_ref().take(take_recipes));
    hits.extend(recipes);
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    // diversity: at most 4 per kind unless a kind filter is on
    let mut per: HashMap<&str, usize> = HashMap::new();
    let mut out: Vec<Hit> = lead;
    let lane_len = out.len();
    for h in hits {
        let c = per.entry(h.doc.kind).or_default();
        if kind.is_none() && *c >= 4 {
            continue;
        }
        *c += 1;
        out.push(h);
        if out.len() - lane_len >= limit {
            break;
        }
    }
    out
}

/// The most query-dense window (<= 7 lines) of a doc body.
fn fragment(d: &Doc, q: &[(String, f32)]) -> String {
    let lines: Vec<&str> = d.body.lines().collect();
    if lines.len() <= 7 {
        return d.body.trim().to_string();
    }
    let hits: Vec<f32> = lines.iter().map(|l| tokens_snake(l).iter().map(|t| q.iter().filter(|(x, _)| x == t).map(|(_, w)| *w).sum::<f32>()).sum()).collect();
    let mut best = (0usize, -1.0f32);
    for s in 0..=lines.len() - 7 {
        let sc: f32 = hits[s..s + 7].iter().sum();
        if sc > best.1 {
            best = (s, sc);
        }
    }
    lines[best.0..best.0 + 7].join("\n").trim_end().to_string()
}

/// The hits as JSON: `{query, hits:[{kind, title, loc, score, fragment}]}` (`--json`).
pub fn to_json(hits: &[Hit], query: &str) -> Value {
    serde_json::json!({
        "query": query,
        "hits": hits.iter().map(|h| serde_json::json!({
            "kind": h.doc.kind, "title": h.doc.title, "loc": h.doc.loc,
            "score": (h.score * 100.0).round() / 100.0, "fragment": h.fragment,
        })).collect::<Vec<_>>(),
    })
}

/// Formats hits as text (kind, title, fragment, where to read more).
pub fn render(hits: &[Hit], query: &str) -> String {
    if hits.is_empty() {
        return format!("nothing found for '{query}'. Try other words, or `red_engine2 describe` for the topic list.\n");
    }
    let mut out = String::new();
    for h in hits {
        out.push_str(&format!("[{}] {}\n", h.doc.kind, h.doc.title));
        for l in h.fragment.lines().take(7) {
            out.push_str(&format!("    {}\n", l.chars().take(150).collect::<String>()));
        }
        out.push_str(&format!("    -> {}\n", h.doc.loc));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmds() -> Value {
        serde_json::json!([{"name": "lint", "about": "Static map checker", "args": []}, {"name": "scatter", "about": "Seeded random planting", "args": []}])
    }

    fn top(query: &str, kind: &str) -> String {
        let docs = corpus(&cmds());
        let hits = search(&docs, query, Some(kind), 3);
        hits.first().map(|h| format!("{} | {} | {}", h.doc.title, h.doc.loc, h.fragment)).unwrap_or_default()
    }

    /// Adding the verified 2D mechanics to the corpus must change nothing for anyone who is not asking for one: the ordinary results are the same with and without them (found the
    /// hard way: a tiny dense `health-damage` entry once outranked the multiplayer docs for "player shoots another player damage health").
    #[test]
    fn the_mechanics_lane_never_displaces_an_ordinary_result() {
        let all = corpus(&cmds());
        let without: Vec<Doc> = corpus(&cmds()).into_iter().filter(|d| d.kind != "mechanic").collect();
        assert!(all.iter().any(|d| d.kind == "mechanic") && all.len() > without.len());
        for q in [
            "player shoots another player damage health",
            "why does my door block the player",
            "how do stairs connect two floors",
            "multiplayer lobby and rounds",
            "spawn enemies in a room",
            "save game progress",
            "timer countdown for a match",
        ] {
            let titles =
                |docs: &[Doc]| search(docs, q, None, 6).into_iter().filter(|h| h.doc.kind != "mechanic").map(|h| h.doc.title.clone()).collect::<Vec<_>>();
            assert_eq!(titles(&all), titles(&without), "`{q}`: a mechanic changed the ordinary results");
        }
    }

    /// The story guide is a domain of its own: it answers questions that name a story, and changes nothing for the rest (it once took two of
    /// the slots of "reusable machine with its own state, inventory and recipes" and pushed that task past its budget).
    #[test]
    fn the_story_guide_answers_story_questions_and_displaces_nothing_else() {
        let all = corpus(&cmds());
        let without: Vec<Doc> = corpus(&cmds()).into_iter().filter(|d| d.kind != "story").collect();
        assert!(all.iter().any(|d| d.kind == "story"));
        for q in
            ["reusable machine with its own state, inventory and recipes", "why does my door block the player", "save game progress", "workflow for a new map"]
        {
            let titles = |docs: &[Doc]| search(docs, q, None, 6).into_iter().map(|h| h.doc.title.clone()).collect::<Vec<_>>();
            assert_eq!(titles(&all), titles(&without), "`{q}`: the story guide changed the results");
        }
        for q in ["how do I write dialogue for a character", "bark cooldown and priority", "a conversation with choices that change the game"] {
            let hits = search(&all, q, None, 3);
            assert!(hits.iter().any(|h| h.doc.kind == "story"), "`{q}` finds the story guide: {:?}", hits.iter().map(|h| &h.doc.title).collect::<Vec<_>>());
        }
    }

    #[test]
    fn a_request_for_a_mechanic_finds_it_first_and_a_vague_one_does_not_invent_one() {
        let docs = corpus(&cmds());
        let first = |q: &str| search(&docs, q, None, 3).first().map(|h| (h.doc.kind, h.doc.title.clone()));
        for (q, name) in [
            ("add a door that opens with a key", "key-door"),
            ("add a countdown timer that ends the game", "timer-lose"),
            ("respawn at a checkpoint after dying", "checkpoint-respawn"),
            ("spawn enemies over time but not too many", "spawner-waves"),
            ("collect artifacts, activate a generator, survive, then escape", "survive-then-escape"),
            ("draw a random number and show one of several things", "random-pick"),
            ("a table of text chosen by a variable", "text-table"),
            ("tell a story in beats that move on as the player plays", "story-beat"),
            ("a conversation with a choice that branches", "branching-dialogue"),
        ] {
            let (kind, title) = first(q).unwrap();
            assert!(kind == "mechanic" && title.contains(name), "`{q}` should open with `{name}`, got {kind}: {title}");
        }
        // Words every mechanic shares ("game", "player", "2d") single out none of them.
        for q in ["make the camera follow the player in 2d", "a boss that is a 3d model in a 2d game", "publish my browser game"] {
            assert!(search(&docs, q, None, 3).iter().all(|h| h.doc.kind != "mechanic"), "`{q}` asks for no particular mechanic");
        }
        assert!(search(&docs, "door key", Some("recipe"), 5).iter().any(|h| h.doc.kind == "mechanic"), "`--kind recipe` lists mechanics too");
    }

    /// The agent-loop trace's searches (docs/analysis/agent-loop-trace.md, task a): a 3D task got 2D mechanics first and the right recipe 9th of 9.
    #[test]
    fn the_lead_follows_the_domain_a_query_names() {
        let docs = corpus(&cmds());
        let lead = |q: &str| search(&docs, q, None, 6).into_iter().take(2).map(|h| format!("{} {}", h.doc.kind, h.doc.title)).collect::<Vec<_>>();
        let l = lead("locked door opened by a key");
        assert!(l.iter().any(|t| t.contains("gated_garden")), "no domain named: the best 3D recipe leads beside the 2D mechanic: {l:?}");
        assert!(l.iter().any(|t| t.contains("key-door")), "{l:?}");
        let l = lead("3d scene with a gate that opens with a key");
        assert!(l[0].contains("gated_garden") && l.iter().all(|t| !t.starts_with("mechanic")), "a 3D query gets no 2D mechanic: {l:?}");
        let l = lead("two rooms joined by a door");
        assert!(l[0].contains("rooms_and_door"), "{l:?}");
        let l = lead("a 2d game with a key that opens a door");
        assert!(l[0].contains("key-door") && l.iter().all(|t| !t.starts_with("recipe")), "a 2D query gets no 3D recipe: {l:?}");
        let all = search(&docs, "show dialogue when the player picks up an item in a 3D scene", None, 6);
        assert!(all.iter().all(|h| h.doc.kind != "mechanic"), "a 3D question is not answered with a 2D mechanic");
        assert_eq!(query_domain("a platformer with sprites"), Some(true));
        assert_eq!(query_domain("first person walk through two rooms"), Some(false));
        assert_eq!(query_domain("a 2d game with a 3d boss"), None);
    }

    #[test]
    fn finds_the_right_documentation_for_natural_questions() {
        let t = top("why does my door block the player", "doc");
        assert!(t.to_lowercase().contains("door") && t.to_lowercase().contains("header"), "{t}");
        let t = top("how do stairs connect two floors", "doc");
        assert!(t.to_lowercase().contains("stair"), "{t}");
        assert!(top("open stairwell fall railing", "lint").contains("drop"), "{}", top("open stairwell fall railing", "lint"));
    }

    #[test]
    fn finds_decisions_and_glossary_terms() {
        // ADR 0014 also discusses the fixed timestep, so 0005 must be among the top hits, not necessarily first.
        let docs = corpus(&cmds());
        let hits = search(&docs, "why fixed timestep interpolation", Some("adr"), 3);
        assert!(hits.iter().any(|h| h.doc.title.contains("Fixed 1/60")), "{:?}", hits.iter().map(|h| &h.doc.title).collect::<Vec<_>>());
        let t = top("headless server multiplayer physics extract", "adr");
        assert!(t.contains("Headless"), "{t}");
        let t = top("body band ground snap", "glossary").to_lowercase();
        assert!(t.contains("ground snap") || t.contains("body band"), "{t}");
    }

    /// The decision records are files, nothing else: `search` reads the folder, `docs/adr/README.md` holds an index generated from it, and
    /// [`crate::tools::adr::check`] (also `red_engine2 preflight`) says what a file lacks: the title line, `Status:`, `Summary:`, the three sections, a
    /// unique id, an existing superseder, a fresh index.
    #[test]
    fn the_decision_records_are_well_formed_and_read_and_indexed() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let problems = crate::tools::adr::check(root);
        assert!(problems.is_empty(), "fix these (`red_engine2 adr index --write` refreshes the index):\n  {}", problems.join("\n  "));
        let on_disk = crate::tools::adr::load_dir(root);
        let own = own_docs::docs();
        assert_eq!(own.adrs.len(), on_disk.len(), "every file in docs/adr is read");
        assert!(own.adrs.iter().all(|(f, t)| on_disk.iter().any(|(g, u)| g == f && u.replace("\r\n", "\n") == t.replace("\r\n", "\n"))));
        for (f, _) in &own.adrs {
            let id = crate::tools::adr::id_of(f).unwrap_or_else(|| panic!("{f}: not a valid ADR file name"));
            assert!(own.adr_index.contains(&format!("[{id}]({f})")), "{f} is not in the index");
        }
    }

    /// A decision written after the binary was built is found by the next `search`: the corpus is built from the checkout's files.
    #[test]
    fn an_adr_written_after_the_build_is_searchable() {
        let mut own = own_docs::docs().clone();
        own.adrs.push(("2099-01-01-quokka-ledger.md".into(), "# 2099-01-01. The quokka ledger\nStatus: proposed\nSummary: zebra marmalade\n".into()));
        let docs = corpus_with(&own, &cmds());
        assert!(docs.iter().any(|d| d.kind == "adr" && d.loc == "docs/adr/2099-01-01-quokka-ledger.md" && d.body.contains("zebra marmalade")));
    }

    #[test]
    fn analysis_notes_are_searchable() {
        assert!(!own_docs::docs().analyses.is_empty(), "docs/analysis holds the reports of earlier builders");
        let docs = corpus(&cmds());
        assert!(docs.iter().any(|d| d.kind == "analysis" && d.loc.starts_with("docs/analysis/")), "the notes are in the corpus");
    }

    #[test]
    fn finds_assets_by_meaning_and_tag() {
        assert!(top("apple", "asset").contains("apple"));
        assert!(top("folding metal chair", "asset").contains("chair_folding"), "{}", top("folding metal chair", "asset"));
        assert!(top("cash register checkout", "asset").contains("cash_register") || top("cash register checkout", "asset").contains("checkout"));
    }

    #[test]
    fn finds_source_symbols() {
        let t = top("player ground height stairs ramp", "src");
        assert!(t.contains("ground_height") || t.contains("StairsRamp") || t.contains("height"), "{t}");
    }

    #[test]
    fn stemming_and_synonyms_bridge_wording() {
        assert_eq!(stem("stairs"), "stair");
        assert_eq!(stem("doors"), "door");
        let t = top("reusable template object", "type");
        assert!(t.contains("prefab"), "{t}");
    }
}
