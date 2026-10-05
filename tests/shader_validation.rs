//! Every WGSL module the engine builds is parsed, validated (naga's uniformity analysis included) and translated to Direct3D's HLSL on any machine.
//!
//! Why: wgpu on Linux (Vulkan) accepted a shadow lookup that Windows' Direct3D compiler refused, and only hosted CI on Windows saw it. naga is the compiler wgpu
//! itself uses, so a module it rejects here would not have reached a player either. The modules are composed exactly as `gpu.rs`, `ocean_pass.rs` and the other
//! pipeline builders compose them (shared `common.wgsl` first, `DEPTH_TEXTURE_TYPE` filled in for the post pass).

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
    fs::read_to_string(shaders_dir().join(name)).unwrap_or_else(|e| panic!("src/shaders/{name}: {e}"))
}

/// `(label, source)` for each module the pipeline builders create, in the order those builders concatenate them.
fn modules() -> Vec<(String, String)> {
    let join = |parts: &[&str]| parts.iter().map(|p| read(p)).collect::<String>();
    let mut out = vec![
        ("scene".to_string(), join(&["common.wgsl", "scene.wgsl"])),
        ("shadow".to_string(), join(&["common.wgsl", "shadow.wgsl"])),
        ("sky background".to_string(), join(&["common.wgsl", "sky.wgsl", "background.wgsl"])),
        ("ocean".to_string(), join(&["common.wgsl", "sky.wgsl", "ocean.wgsl"])),
        ("crosshair".to_string(), read("crosshair.wgsl")),
        ("fx".to_string(), read("fx.wgsl")),
        ("overlay".to_string(), read("overlay.wgsl")),
    ];
    for depth_ty in ["texture_depth_2d", "texture_depth_multisampled_2d"] {
        out.push((format!("postfx ({depth_ty})"), read("postfx.wgsl").replace("DEPTH_TEXTURE_TYPE", depth_ty)));
    }
    out
}

// naga's uniformity analysis (run by `validate`) accepts a derivative-needing lookup after a `continue` or `break` taken under a per-pixel condition, and treats
// every local variable as per-pixel, so it cannot tell a loop every pixel runs the same number of times from one that varies. Direct3D's compiler (FXC) refuses
// the varying kind ("gradient instruction used in a loop with varying iteration"). So the engine's rule is the simple, safe one: no lookup that needs
// screen-space derivatives (`textureSample`, `textureSampleCompare`, `textureSampleBias`, `dpdx`...) inside a loop. Inside a loop, use the Level variants
// (`textureSampleLevel`, `textureSampleCompareLevel`), which need none, or take the lookup out of the loop.

/// Does this expression need screen-space derivatives (so it must sit in uniform control flow)?
fn needs_derivatives(expr: &Expression) -> bool {
    match expr {
        Expression::Derivative { .. } => true,
        Expression::ImageSample { gather: None, level: SampleLevel::Auto | SampleLevel::Bias(_), .. } => true,
        _ => false,
    }
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

/// A shader file that is in no composition above is never checked: a new one must be added to `modules()` (and to the pipeline builder that uses it).
#[test]
fn every_shader_file_is_in_a_checked_module() {
    let on_disk: BTreeSet<String> =
        fs::read_dir(shaders_dir()).expect("src/shaders").filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.ends_with(".wgsl")).collect();
    let used: BTreeSet<String> =
        ["common", "scene", "shadow", "sky", "background", "ocean", "crosshair", "fx", "overlay", "postfx"].iter().map(|n| format!("{n}.wgsl")).collect();
    assert_eq!(on_disk, used, "src/shaders/*.wgsl and the modules in tests/shader_validation.rs disagree: add the new file to `modules()` and to this list");
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
