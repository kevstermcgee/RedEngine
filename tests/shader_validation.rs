//! Every WGSL module the engine builds is parsed, validated (naga's uniformity analysis included) and translated to Direct3D's HLSL on any machine.
//!
//! **What this proves, and what it does not.** It proves that naga (the compiler wgpu uses) parses and validates every module as the engine composes it, that naga's HLSL
//! backend can write each one, and that no lookup needing screen-space derivatives sits inside a loop (the rule that would have caught the Windows failure below). It does
//! NOT run Microsoft's compiler (FXC/DXC): an error only that compiler makes still reaches hosted CI, and **the Windows job there stays authoritative for Direct3D**. Nothing
//! here replaces it, and a green run here is never evidence that a shader compiles on Windows, only that it is free of the errors listed above.
//!
//! Why it exists: wgpu on Linux (Vulkan) accepted a shadow lookup that Windows' Direct3D compiler refused, and only hosted CI on Windows saw it.
//!
//! **Coverage is derived, not listed.** The compositions checked are compared with the ones the engine's pipeline builders actually contain (every `include_str!` of a
//! shader in `src/`, grouped by the `concat!` that joins them), so a shader added or re-composed in `gpu.rs` and not here fails
//! `every_composition_the_engine_builds_is_checked`, and one here that no builder uses fails the same test.

use naga::back::hlsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{Block, Expression, Function, Handle, Module, SampleLevel, Statement};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::PathBuf;

fn shaders_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/shaders")
}

fn read(name: &str) -> String {
    fs::read_to_string(shaders_dir().join(format!("{name}.wgsl"))).unwrap_or_else(|e| panic!("src/shaders/{name}.wgsl: {e}"))
}

/// The modules the pipeline builders create, as `(label, shader files in the order they are joined)`. Must equal what `src/` contains (see the test below).
const COMPOSITIONS: &[(&str, &[&str])] = &[
    ("scene", &["common", "scene"]),
    ("shadow", &["common", "shadow"]),
    ("sky background", &["common", "sky", "background"]),
    ("ocean", &["common", "sky", "ocean"]),
    ("crosshair", &["crosshair"]),
    ("fx", &["fx"]),
    ("overlay", &["overlay"]),
    ("postfx", &["postfx"]),
];

/// `(label, source)` for each module, with the post pass filled in for both depth texture types the engine builds.
fn modules() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (label, files) in COMPOSITIONS {
        let source: String = files.iter().map(|f| read(f)).collect();
        if *label == "postfx" {
            for depth_ty in ["texture_depth_2d", "texture_depth_multisampled_2d"] {
                out.push((format!("postfx ({depth_ty})"), source.replace("DEPTH_TEXTURE_TYPE", depth_ty)));
            }
        } else {
            out.push((label.to_string(), source));
        }
    }
    out
}

/// Every shader composition `src/` builds, found by reading it: each `include_str!("shaders/NAME.wgsl")` outside test code, grouped with its neighbours when they sit in
/// one `concat!( ... )`. Returned as the file names (no extension) in order, one entry per distinct composition.
fn compositions_in_source() -> BTreeSet<Vec<String>> {
    fn rust_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for e in fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                rust_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    const NEEDLE: &str = "include_str!(\"shaders/";
    let names = |text: &str| -> Vec<String> {
        text.match_indices(NEEDLE).filter_map(|(i, _)| text[i + NEEDLE.len()..].split_once(".wgsl\")").map(|(n, _)| n.to_string())).collect()
    };
    let mut files = Vec::new();
    rust_files(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"), &mut files);
    let mut found = BTreeSet::new();
    for f in files {
        let text = fs::read_to_string(&f).unwrap_or_default();
        // Test modules read shader text to check layouts; they build no pipeline.
        let code = text.split("#[cfg(test)]").next().unwrap_or("");
        let mut rest = code.to_string();
        // A `concat!( ... )` with balanced parentheses is one composition; blank it out so its parts are not also counted alone.
        while let Some(start) = rest.find("concat!(") {
            let (mut depth, mut end) = (0usize, None);
            for (i, c) in rest[start + "concat!".len()..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(start + "concat!".len() + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            let end = end.expect("a concat! with no closing parenthesis");
            let group = names(&rest[start..end]);
            if !group.is_empty() {
                found.insert(group);
            }
            rest.replace_range(start..end, "");
        }
        for single in names(&rest) {
            found.insert(vec![single]);
        }
    }
    found
}

// naga's uniformity analysis (run by `validate`) accepts a derivative-needing lookup after a `continue` or `break` taken under a per-pixel condition, and treats
// every local variable as per-pixel, so it cannot tell a loop every pixel runs the same number of times from one that varies. Direct3D's compiler (FXC) refuses
// the varying kind ("gradient instruction used in a loop with varying iteration"). So the engine's rule is the simple, safe one: no lookup that needs
// screen-space derivatives (`textureSample`, `textureSampleCompare`, `textureSampleBias`, `dpdx`...) inside a loop. Inside a loop, use the Level variants
// (`textureSampleLevel`, `textureSampleCompareLevel`), which need none, or take the lookup out of the loop.

/// Does this expression need screen-space derivatives (so it must sit in uniform control flow)?
fn needs_derivatives(expr: &Expression) -> bool {
    matches!(expr, Expression::Derivative { .. } | Expression::ImageSample { gather: None, level: SampleLevel::Auto | SampleLevel::Bias(_), .. })
}

/// The statements nested directly in `st` (branches, cases, loop bodies).
fn children(st: &Statement) -> Vec<&Block> {
    match st {
        Statement::If { accept, reject, .. } => vec![accept, reject],
        Statement::Switch { cases, .. } => cases.iter().map(|c| &c.body).collect(),
        Statement::Loop { body, continuing, .. } => vec![body, continuing],
        Statement::Block(b) => vec![b],
        _ => Vec::new(),
    }
}

/// The functions that, directly or through a call, need derivatives.
fn derivative_functions(module: &Module) -> HashSet<Handle<Function>> {
    fn calls(block: &Block, out: &mut Vec<Handle<Function>>) {
        for st in block.iter() {
            if let Statement::Call { function, .. } = st {
                out.push(*function);
            }
            children(st).into_iter().for_each(|b| calls(b, out));
        }
    }
    let mut set: HashSet<Handle<Function>> =
        module.functions.iter().filter(|(_, f)| f.expressions.iter().any(|(_, e)| needs_derivatives(e))).map(|(h, _)| h).collect();
    loop {
        let before = set.len();
        for (h, f) in module.functions.iter() {
            let mut called = Vec::new();
            calls(&f.body, &mut called);
            if called.iter().any(|c| set.contains(c)) {
                set.insert(h);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// Every derivative-needing lookup (or call to a function that has one) that sits inside a loop, as `` `function` (line N): what ``.
fn derivatives_in_loops(module: &Module, source: &str) -> Vec<String> {
    struct Scan<'a> {
        module: &'a Module,
        derivative_fns: HashSet<Handle<Function>>,
        source: &'a str,
        found: Vec<String>,
    }
    impl Scan<'_> {
        fn walk(&mut self, block: &Block, name: &str, exprs: &naga::Arena<Expression>, in_loop: bool) {
            for st in block.iter() {
                if in_loop {
                    match st {
                        Statement::Emit(range) => {
                            for h in range.clone().filter(|h| needs_derivatives(&exprs[*h])) {
                                self.report(name, exprs.get_span(h), "a lookup that needs screen-space derivatives");
                            }
                        }
                        Statement::Call { function, .. } if self.derivative_fns.contains(function) => {
                            let callee = self.module.functions[*function].name.clone().unwrap_or_default();
                            self.report(
                                name,
                                self.module.functions.get_span(*function),
                                &format!("a call to `{callee}`, which needs screen-space derivatives"),
                            );
                        }
                        _ => {}
                    }
                }
                let into_loop = in_loop || matches!(st, Statement::Loop { .. });
                for child in children(st) {
                    self.walk(child, name, exprs, into_loop);
                }
            }
        }

        fn report(&mut self, name: &str, span: naga::Span, what: &str) {
            let line = span.location(self.source).line_number;
            self.found.push(format!("`{name}` (line {line}): {what}, inside a loop"));
        }
    }
    let mut scan = Scan { module, derivative_fns: derivative_functions(module), source, found: Vec::new() };
    for (_, f) in module.functions.iter() {
        scan.walk(&f.body, f.name.as_deref().unwrap_or("?"), &f.expressions, false);
    }
    for ep in &module.entry_points {
        scan.walk(&ep.function.body, &ep.name, &ep.function.expressions, false);
    }
    scan.found.dedup();
    scan.found
}

/// Parse, validate and write HLSL for one module; the error says which step failed and where.
fn check(label: &str, source: &str) -> Result<(), String> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| format!("{label}: does not parse:\n{}", e.emit_to_string(source)))?;
    let info = Validator::new(ValidationFlags::all(), Capabilities::default())
        .validate(&module)
        .map_err(|e| format!("{label}: does not validate:\n{}", e.emit_to_string(source)))?;
    let in_loops = derivatives_in_loops(&module, source);
    if !in_loops.is_empty() {
        return Err(format!(
            "{label}: Direct3D's compiler can refuse a derivative lookup in a loop (use textureSampleLevel / textureSampleCompareLevel, or move it out of the loop):\n  {}",
            in_loops.join("\n  ")
        ));
    }
    let mut hlsl_source = String::new();
    hlsl::Writer::new(&mut hlsl_source, &hlsl::Options::default(), &hlsl::PipelineOptions::default())
        .write(&module, &info, None)
        .map_err(|e| format!("{label}: cannot be translated to HLSL: {e}"))?;
    Ok(())
}

#[test]
fn every_shader_module_validates_and_translates_to_hlsl() {
    let failures: Vec<String> = modules().iter().filter_map(|(label, source)| check(label, source).err()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The compositions this test checks are exactly the ones the engine builds: a new shader, or a changed `concat!`, in `src/` that is not mirrored in `COMPOSITIONS`
/// would otherwise never be validated, and a stale entry here would give a false sense of coverage.
#[test]
fn every_composition_the_engine_builds_is_checked() {
    let built = compositions_in_source();
    let checked: BTreeSet<Vec<String>> = COMPOSITIONS.iter().map(|(_, files)| files.iter().map(|f| f.to_string()).collect()).collect();
    let unchecked: Vec<_> = built.difference(&checked).collect();
    let stale: Vec<_> = checked.difference(&built).collect();
    assert!(
        unchecked.is_empty() && stale.is_empty(),
        "src/ builds shader compositions this test does not check: {unchecked:?}; this test checks compositions src/ no longer builds: {stale:?}. Update COMPOSITIONS in tests/shader_validation.rs."
    );
}

/// And every shader file on disk is part of a composition that is built: an orphaned `.wgsl` is dead weight that nothing validates.
#[test]
fn every_shader_file_is_part_of_a_built_composition() {
    let on_disk: BTreeSet<String> = fs::read_dir(shaders_dir())
        .expect("src/shaders")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_suffix(".wgsl").map(str::to_string))
        .collect();
    let used: BTreeSet<String> = compositions_in_source().into_iter().flatten().collect();
    assert_eq!(on_disk, used, "src/shaders/*.wgsl and the shaders the pipeline builders include disagree");
}

// The rule itself, on small modules: what Direct3D refused (a lookup in a loop that exits under a per-pixel condition) is refused, and so is any other loop (this
// check cannot tell a uniform trip count from a varying one); the Level variants and lookups outside loops are accepted.
const LOOKUP_PRELUDE: &str = "
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> count: u32;
";

fn fragment(body: &str) -> String {
    format!("{LOOKUP_PRELUDE}\n@fragment fn fs(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{ var acc = vec4<f32>(0.0); {body} return acc; }}")
}

#[test]
fn a_derivative_lookup_in_a_loop_is_refused_whatever_the_loop_does() {
    let varying = "for (var i = 0u; i < count; i = i + 1u) { if (uv.x > f32(i)) { continue; } acc = acc + textureSample(tex, smp, uv); }";
    let with_break = "for (var i = 0u; i < count; i = i + 1u) { acc = acc + textureSample(tex, smp, uv); if (acc.x > uv.y) { break; } }";
    let counted = "for (var i = 0u; i < 4u; i = i + 1u) { acc = acc + textureSample(tex, smp, uv); }";
    for (what, body) in [("per-pixel continue", varying), ("per-pixel break", with_break), ("constant count", counted)] {
        let err = check(what, &fragment(body)).expect_err(what);
        assert!(err.contains("`fs`") && err.contains("inside a loop"), "{what}: the report names the function and the loop: {err}");
    }
}

#[test]
fn a_derivative_lookup_in_a_called_function_is_refused_in_a_loop() {
    let src = format!(
        "{LOOKUP_PRELUDE}\nfn look(uv: vec2<f32>) -> vec4<f32> {{ return textureSample(tex, smp, uv); }}\n\
         @fragment fn fs(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{ var acc = vec4<f32>(0.0); for (var i = 0u; i < 2u; i = i + 1u) {{ acc = acc + look(uv); }} return acc; }}"
    );
    let err = check("call", &src).expect_err("a call that needs derivatives, in a loop");
    assert!(err.contains("`look`"), "the report names the callee: {err}");
}

#[test]
fn level_lookups_in_loops_and_plain_lookups_outside_them_are_accepted() {
    let level = "for (var i = 0u; i < count; i = i + 1u) { if (uv.x > f32(i)) { continue; } acc = acc + textureSampleLevel(tex, smp, uv, 0.0); }";
    assert_eq!(check("level in a loop", &fragment(level)), Ok(()));
    assert_eq!(check("outside a loop", &fragment("acc = textureSample(tex, smp, uv); for (var i = 0u; i < count; i = i + 1u) { acc = acc * 0.5; }")), Ok(()));
}

/// The shape that broke Windows, kept as a permanent fixture: the cascaded-shadow lookup as first written (reduced from `scene.wgsl` at commit 85ce0b4). `textureSampleCompare`
/// sits in `cascade_lit`'s tap loop, which `shadow_factor` calls inside a cascade loop that `continue`s and `return`s under per-pixel conditions. Linux/Vulkan accepted it;
/// Direct3D's compiler refused the pipeline ("gradient instruction used in a loop with varying iteration"). naga alone validates it too, which is exactly why
/// `check` has its own rule: this fixture must stay refused, naming the call chain. If it ever passes, the guard against that class of failure is gone.
const THE_SHADOW_LOOP_THAT_BROKE_DIRECT3D: &str = "
@group(0) @binding(0) var shadow_map: texture_depth_2d;
@group(0) @binding(1) var shadow_sampler: sampler_comparison;
@group(0) @binding(2) var<uniform> counts: vec4<f32>;

fn cascade_lit(uv: vec2<f32>, z: f32) -> f32 {
    var lit = 0.0;
    for (var i = 0; i < 8; i = i + 1) {
        lit = lit + textureSampleCompare(shadow_map, shadow_sampler, uv + vec2<f32>(f32(i) * 0.001, 0.0), z);
    }
    return lit / 8.0;
}

fn shadow_factor(uv: vec2<f32>, edge: f32) -> f32 {
    let count = u32(counts.z);
    for (var c = 0u; c < count; c = c + 1u) {
        if (edge >= 1.0) {
            continue;
        }
        let lit = cascade_lit(uv, 0.5);
        return lit;
    }
    return 1.0;
}

@fragment fn fs(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let s = shadow_factor(uv, uv.x);
    return vec4<f32>(s, s, s, 1.0);
}
";

#[test]
fn the_shadow_loop_that_broke_direct3d_stays_refused() {
    let err = check("the 85ce0b4 shadow lookup", THE_SHADOW_LOOP_THAT_BROKE_DIRECT3D).expect_err("the shape that Direct3D refused must be refused here too");
    assert!(err.contains("`cascade_lit`") && err.contains("inside a loop"), "the report names the lookup: {err}");
    // The fix that shipped, in the same shape: the Level variant needs no derivatives, so the identical control flow is accepted.
    let fixed = THE_SHADOW_LOOP_THAT_BROKE_DIRECT3D.replace("textureSampleCompare(", "textureSampleCompareLevel(");
    assert_eq!(check("the fixed shadow lookup", &fixed), Ok(()));
}
