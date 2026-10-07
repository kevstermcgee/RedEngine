//! `red_engine2 mcp`: the engine as a Model Context Protocol server on stdio (ADR 2026-10-06-a-native-mcp-server).
//!
//! One process serves a whole session: each tool call runs the same command line the CLI would, **in this process** (the output capture of `tools::envelope`, no
//! spawn), so what an agent is told is exactly what `red_engine2 <command>` prints and there is no second implementation. Eleven tools instead of the Python
//! adapter's 36; their argument structs are the schema (`schemars`), and every file argument is a path in the server's working directory, so a scene is never sent
//! as text. Work that is expensive to repeat (the search index, a scene's analysis world) is cached by what it was built from; see `tools::world` and `tools::search`.
//!
//! Standard output belongs to the protocol: nothing else may print to it. Commands that never return (servers, interactive play) are refused, not run.

use super::args::Cli;
use base64::Engine as _;
use red_engine2::tools::envelope;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// The most text one tool result may carry; a longer one is cut with a note saying how to ask for less.
const MAX_RESULT_BYTES: usize = 40_000;

/// Whether a command line is one an agent must not start through the server: it runs until stopped, needs a person at a window, or would start another server.
/// `top` and `sub` are the command and its subcommand; `args` is the whole line (`portmap keep` takes its action as an argument).
fn refused(top: &str, sub: &str, args: &[String]) -> bool {
    matches!((top, sub), ("mcp", _) | ("game", "serve" | "play" | "play-local") | ("play2d", _)) || (top == "portmap" && args.iter().any(|a| a == "keep"))
}

/// What a command line did.
pub struct Outcome {
    /// Process-style exit code (0 = success).
    pub code: i32,
    /// What it printed on stdout.
    pub out: String,
    /// What it printed on stderr, plus its error message.
    pub err: String,
}

/// Runs one `red_engine2` command line in this process with its output captured (text form). Never exits the process, never panics out.
pub fn exec(args: &[String]) -> Outcome {
    use clap::{CommandFactory, FromArgMatches};
    let mut argv = vec!["red_engine2".to_string()];
    argv.extend(args.iter().cloned());
    let matches = match Cli::command().try_get_matches_from(&argv) {
        Ok(m) => m,
        // `--help` and usage errors are answers, not failures of the server.
        Err(e) => {
            return Outcome {
                code: if e.use_stderr() { 2 } else { 0 },
                out: if e.use_stderr() { String::new() } else { e.to_string() },
                err: if e.use_stderr() { e.to_string() } else { String::new() },
            }
        }
    };
    let top = matches.subcommand_name().unwrap_or("").to_string();
    let sub = matches.subcommand().and_then(|(_, m)| m.subcommand_name().map(str::to_string)).unwrap_or_default();
    if refused(&top, &sub, args) {
        return Outcome {
            code: 2,
            out: String::new(),
            err: format!(
                "`{top}{}` runs until stopped or needs a window, so the MCP server does not start it: run it in a terminal\n",
                if sub.is_empty() { String::new() } else { format!(" {sub}") }
            ),
        };
    }
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(c) => c,
        Err(e) => return Outcome { code: 2, out: String::new(), err: e.to_string() },
    };
    envelope::begin_capture_text();
    let started = std::time::Instant::now();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::run(cli.command)));
    let (out, mut err) = envelope::end_capture();
    let code = match result {
        Ok(Ok(())) => 0,
        Ok(Err(msg)) => {
            if !msg.is_empty() {
                err.push_str(&msg);
                err.push('\n');
            }
            1
        }
        Err(_) => {
            err.push_str("the command panicked (a bug in the engine: report it with the arguments you gave)\n");
            101
        }
    };
    red_engine2::tools::agent_trace::record(&top, code, started.elapsed().as_millis(), "");
    Outcome { code, out, err }
}

/// What an agent is shown for an outcome: stdout, then stderr when there is any; an error result when the command failed.
fn shape(o: Outcome) -> CallToolResult {
    let mut text = o.out.trim_end().to_string();
    let err = o.err.trim_end();
    if !err.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(err);
    }
    if text.len() > MAX_RESULT_BYTES {
        let cut = text.floor_char_boundary(MAX_RESULT_BYTES);
        let dropped = text.len() - cut;
        text.truncate(cut);
        text.push_str(&format!("\n… {dropped} more bytes not shown: ask for less (a narrower `only`, `limit`, or a smaller topic)"));
    }
    if text.is_empty() {
        text = if o.code == 0 { "ok".to_string() } else { format!("failed (exit {})", o.code) };
    }
    if o.code == 0 {
        CallToolResult::success(vec![ContentBlock::text(text)])
    } else {
        CallToolResult::error(vec![ContentBlock::text(text)])
    }
}

/// Runs a command line off the async threads (commands are synchronous and may take a while) and shapes the result.
async fn run_blocking(args: Vec<String>) -> CallToolResult {
    match tokio::task::spawn_blocking(move || exec(&args)).await {
        Ok(o) => shape(o),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(format!("the command did not finish: {e}"))]),
    }
}

// ----------------------------------------------------------------------------------------------- tool arguments (these structs are the schemas)

/// Arguments of `describe`.
#[derive(Deserialize, JsonSchema)]
pub struct DescribeArgs {
    /// Omit for the ~1 KB manual.
    pub topic: Option<String>,
}

/// Arguments of `search`.
#[derive(Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// Plain words.
    pub query: String,
    /// doc, adr, asset, lint, recipe, command, src, analysis.
    pub kind: Option<String>,
    /// Default 5.
    pub limit: Option<usize>,
}

/// Arguments of `context`.
#[derive(Deserialize, JsonSchema)]
pub struct ContextArgs {
    /// A feature, a file path or words.
    pub query: String,
}

/// Arguments of `validate`.
#[derive(Deserialize, JsonSchema)]
pub struct FileArgs {
    /// Scene or 2D game path (relative to the server's cwd).
    pub file: String,
}

/// Arguments of `lint`.
#[derive(Deserialize, JsonSchema)]
pub struct LintArgs {
    /// Scene path.
    pub file: String,
    /// Warnings are errors.
    pub strict: Option<bool>,
    /// A `phases` state.
    pub phase: Option<String>,
}

/// Arguments of `analyze`.
#[derive(Deserialize, JsonSchema)]
pub struct AnalyzeArgs {
    /// reach, walk, ray, ls or info.
    pub what: String,
    /// Scene path.
    pub file: String,
    /// The command's own arguments, e.g. `["--auto","--from","0,-8","--to","1,2"]` (walk), `["sofa_1"]` (info).
    pub args: Option<Vec<String>>,
}

/// Arguments of `patch`.
#[derive(Deserialize, JsonSchema)]
pub struct PatchArgs {
    /// Scene path.
    pub file: String,
    /// Edits in order, e.g. `{"op":"move","id":"lamp_1","by":[0,-0.2,0]}` (ops: add set move rm clone rename).
    pub ops: Vec<serde_json::Value>,
    /// Preview only.
    pub dry_run: Option<bool>,
}

/// Arguments of `verify` and `sim`.
#[derive(Deserialize, JsonSchema)]
pub struct ProveArgs {
    /// Scene or 2D game path.
    pub file: String,
    /// Only checks whose name contains this.
    pub only: Option<String>,
    /// verify: skip golden views (default true).
    pub no_views: Option<bool>,
}

/// Arguments of `view`.
#[derive(Deserialize, JsonSchema)]
pub struct ViewArgs {
    /// Scene or 2D game path.
    pub file: String,
    /// plan (default), frame or tour.
    pub kind: Option<String>,
    /// Extra args, e.g. `["--eye","0,8,12","--at","0,0,0"]`.
    pub args: Option<Vec<String>>,
}

/// Arguments of `run`.
#[derive(Deserialize, JsonSchema)]
pub struct RunArgs {
    /// The command line without the program name, e.g. `["catalog","apple"]`.
    pub args: Vec<String>,
}

// ----------------------------------------------------------------------------------------------- the server

/// The server: a router over the tools below.
#[derive(Clone)]
pub struct RedMcp {
    tool_router: ToolRouter<Self>,
}

impl RedMcp {
    /// A server with every tool registered.
    pub fn new() -> Self {
        RedMcp { tool_router: Self::tool_router() }
    }

    /// The tools this server lists, as `tools/list` returns them (for the byte-budget test).
    #[cfg(test)]
    pub fn listed_tools(&self) -> Vec<rmcp::model::Tool> {
        self.tool_router.list_all()
    }
}

impl Default for RedMcp {
    fn default() -> Self {
        Self::new()
    }
}

fn push(args: &mut Vec<String>, items: &[&str]) {
    args.extend(items.iter().map(|s| s.to_string()));
}

#[tool_router]
impl RedMcp {
    /// The engine's own manual. Start here: with no topic it is ~1 KB (binaries, workflow, commands, topics).
    #[tool(description = "The engine's manual. No topic: ~1 KB. Topics: rules, objects, scene, lint, physics, web, 2d, decisions, overview.")]
    async fn describe(&self, Parameters(a): Parameters<DescribeArgs>) -> CallToolResult {
        run_blocking(match a.topic.as_deref() {
            None | Some("") | Some("brief") => vec!["describe".into(), "--brief".into()],
            Some(t) => vec!["describe".into(), t.into()],
        })
        .await
    }

    #[tool(description = "Ranked fragments from docs, ADRs, assets, lint codes, recipes, commands. Cheaper than reading SPEC.md.")]
    async fn search(&self, Parameters(a): Parameters<SearchArgs>) -> CallToolResult {
        let mut v = vec!["search".to_string()];
        v.extend(a.query.split_whitespace().map(str::to_string));
        if let Some(k) = a.kind {
            push(&mut v, &["--kind"]);
            v.push(k);
        }
        v.push("--limit".into());
        v.push(a.limit.unwrap_or(5).to_string());
        run_blocking(v).await
    }

    #[tool(description = "A 5-15 KB work packet for one Rust change: files, public API, tests, ADRs.")]
    async fn context(&self, Parameters(a): Parameters<ContextArgs>) -> CallToolResult {
        let mut v = vec!["context".to_string()];
        v.extend(a.query.split_whitespace().map(str::to_string));
        run_blocking(v).await
    }

    #[tool(description = "Check a scene or 2D game: OK, or `path: message` errors with a fix.")]
    async fn validate(&self, Parameters(a): Parameters<FileArgs>) -> CallToolResult {
        run_blocking(vec!["validate".into(), a.file]).await
    }

    #[tool(description = "Static map checker: overlaps, unreachable rooms, bad stairs, leaks. Error result on errors.")]
    async fn lint(&self, Parameters(a): Parameters<LintArgs>) -> CallToolResult {
        let mut v = vec!["lint".to_string(), a.file];
        if a.strict.unwrap_or(false) {
            push(&mut v, &["--strict"]);
        }
        if let Some(p) = a.phase {
            push(&mut v, &["--phase"]);
            v.push(p);
        }
        run_blocking(v).await
    }

    #[tool(description = "Ask the real collision code: reach (rooms, doorways), walk (plan/replay a route), ray (line of sight), ls, info.")]
    async fn analyze(&self, Parameters(a): Parameters<AnalyzeArgs>) -> CallToolResult {
        if !["reach", "walk", "ray", "ls", "info"].contains(&a.what.as_str()) {
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "what: `{}` is not one of reach, walk, ray, ls, info (use `run` for anything else)",
                a.what
            ))]);
        }
        let mut v = vec![a.what, a.file];
        v.extend(a.args.unwrap_or_default());
        run_blocking(v).await
    }

    #[tool(description = "Edit a scene atomically, validated once. Cheaper than rewriting the file.")]
    async fn patch(&self, Parameters(a): Parameters<PatchArgs>) -> CallToolResult {
        let mut v = vec!["patch".to_string(), a.file, serde_json::Value::Array(a.ops).to_string()];
        if a.dry_run.unwrap_or(false) {
            push(&mut v, &["--dry-run"]);
        }
        run_blocking(v).await
    }

    #[tool(description = "Run the scene's own checks or a 2D game's scenarios; a failure names its cause.")]
    async fn verify(&self, Parameters(a): Parameters<ProveArgs>) -> CallToolResult {
        let mut v = vec!["verify".to_string(), a.file];
        if let Some(o) = a.only {
            push(&mut v, &["--only"]);
            v.push(o);
        }
        if a.no_views.unwrap_or(true) {
            push(&mut v, &["--no-views"]);
        }
        run_blocking(v).await
    }

    #[tool(description = "Headless play-throughs of the rules (`checks.sim`) or a 2D game's scenarios.")]
    async fn sim(&self, Parameters(a): Parameters<ProveArgs>) -> CallToolResult {
        let mut v = vec!["sim".to_string(), a.file];
        if let Some(o) = a.only {
            push(&mut v, &["--only"]);
            v.push(o);
        }
        run_blocking(v).await
    }

    #[tool(description = "LOOK: a top-down plan (default), a rendered frame, or a tour sheet, as an image.")]
    async fn view(&self, Parameters(a): Parameters<ViewArgs>) -> CallToolResult {
        let kind = a.kind.unwrap_or_else(|| "plan".into());
        if !["plan", "frame", "tour"].contains(&kind.as_str()) {
            return CallToolResult::error(vec![ContentBlock::text(format!("kind: `{kind}` is not one of plan, frame, tour"))]);
        }
        let dir = std::env::temp_dir().join(format!("re2_mcp_view_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let out = dir.join(format!("{kind}.png"));
        let mut v = vec![kind.clone(), a.file, out.to_string_lossy().to_string()];
        v.extend(a.args.unwrap_or_default());
        let mut res = run_blocking(v).await;
        // `plan --all-floors` writes `<name>_y<h>.png`; send the first picture that exists.
        let png = std::fs::read(&out).ok().or_else(|| {
            let stem = out.file_stem()?.to_string_lossy().to_string();
            let mut found: Vec<_> = std::fs::read_dir(&dir).ok()?.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&stem)).collect();
            found.sort_by_key(|e| e.file_name());
            std::fs::read(found.first()?.path()).ok()
        });
        if res.is_error != Some(true) {
            if let Some(bytes) = png {
                let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                res.content.insert(0, ContentBlock::image(b64, "image/png"));
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        res
    }

    #[tool(description = "Any other red_engine2 command (catalog, recipe, new-game, build, ...). Servers are refused.")]
    async fn run(&self, Parameters(a): Parameters<RunArgs>) -> CallToolResult {
        run_blocking(a.args).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for RedMcp {}

/// `red_engine2 mcp`: serves until the client closes stdin.
pub fn run_mcp() -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| format!("cannot start the async runtime: {e}"))?;
    rt.block_on(async {
        let service = RedMcp::new().serve(rmcp::transport::stdio()).await.map_err(|e| format!("mcp: {e}"))?;
        service.waiting().await.map_err(|e| format!("mcp: {e}"))?;
        Ok(())
    })
}

/// The most bytes `tools/list` may take (it is loaded into the agent's context every session).
#[cfg(test)]
pub const TOOLS_LIST_BUDGET: usize = 5_200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tool_list_stays_small_and_every_tool_has_a_schema() {
        let tools = RedMcp::new().listed_tools();
        assert!(tools.len() <= 12, "{} tools: the surface is meant to stay at about a dozen", tools.len());
        let bytes = serde_json::to_string(&tools).unwrap().len();
        assert!(bytes <= TOOLS_LIST_BUDGET, "tools/list is {bytes} bytes, the budget is {TOOLS_LIST_BUDGET}: every session loads it into the agent's context (the Python adapter's 36 tools were 21618)");
        for t in &tools {
            assert_eq!(t.input_schema.get("type").and_then(|v| v.as_str()), Some("object"), "{}", t.name);
            assert!(t.description.as_deref().is_some_and(|d| !d.is_empty()), "{} needs a description", t.name);
        }
    }

    #[test]
    fn refused_commands_are_refused_and_help_is_an_answer() {
        for line in [vec!["game", "serve"], vec!["game", "play-local"], vec!["play2d", "game.game2d.json"], vec!["portmap", "keep"], vec!["mcp"]] {
            let o = exec(&line.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            assert_eq!(o.code, 2, "{line:?}: {}", o.err);
            assert!(o.err.contains("runs until stopped"), "{line:?}: {}", o.err);
        }
        let o = exec(&["validate".to_string(), "--help".to_string()]);
        assert_eq!(o.code, 0);
        assert!(o.out.contains("Usage"), "{}", o.out);
        let o = exec(&["no-such-command".to_string()]);
        assert_eq!(o.code, 2);
    }

    #[test]
    fn a_command_runs_in_process_with_its_text_output_captured() {
        let o = exec(&["describe".to_string(), "--brief".to_string()]);
        assert_eq!(o.code, 0, "{}", o.err);
        assert!(o.out.starts_with("Red Engine 2"), "{}", o.out);
        let o = exec(&["validate".to_string(), "no/such/file.json".to_string()]);
        assert_ne!(o.code, 0);
        assert!(!o.err.is_empty());
    }
}
