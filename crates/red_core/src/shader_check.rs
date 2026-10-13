//! Is a WGSL module safe to hand to every GPU backend the engine ships on? Parse, validate (naga's uniformity analysis included), refuse derivative lookups inside
//! loops, and translate to Direct3D's HLSL, all with naga (the compiler wgpu uses) on any machine.
//!
//! Used by `tests/shader_validation.rs` for every module the engine composes, and by scene validation for a game's own sky (`sky.shader`), so a game's shader that
//! would fail on Windows fails `validate` on a laptop instead. What it does NOT do: run Microsoft's compiler (FXC/DXC), so an error only that compiler makes is not
//! caught here; hosted CI on Windows stays authoritative for the engine's own shaders.

use naga::back::hlsl;
use naga::valid::{Capabilities, ValidationFlags, Validator};
use naga::{Block, Expression, Function, Handle, Module, SampleLevel, Statement};
use std::collections::HashSet;

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
pub fn derivatives_in_loops(module: &Module, source: &str) -> Vec<String> {
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

/// Parse, validate and write HLSL for one module; the error says which step failed and where (naga's own report, with the source line).
pub fn check(label: &str, source: &str) -> Result<(), String> {
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
