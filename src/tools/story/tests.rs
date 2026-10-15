//! The toolchain's own tests: syntax, binding, types, flow, profiles, knowledge contracts, warnings and their allows, references.

use super::*;
use crate::sim::defs::Id;
use crate::sim::story::package::{Catalog, Stakes};
use crate::sim::systems::{QueryDecl, Ty};

fn id(s: &str) -> Id {
    Id::parse(s).unwrap()
}

/// A tiny game to bind against: a door that can be opened, a coin count, a bell event.
fn catalog() -> Catalog {
    Catalog {
        queries: vec![QueryDecl::new(id("g:coins"), Ty::Int, "coins"), QueryDecl::new(id("g:door_open"), Ty::Bool, "the door is open")],
        commands: vec![id("g:open"), id("g:pay"), id("g:hint")],
        unattended: vec![id("g:hint")],
        events: vec![id("g:bell"), id("g:coin")],
        scene_vars: vec!["near_ann".into()],
        consumers: vec![],
    }
}

const SHEETS: &str = "\
character ann \"Ann\"
  want: the door open before the bell
  need: to ask for help
  flaw: impatient
  contradiction: kind to strangers, sharp with friends
  values: pays her debts
  relationships: player, a stranger
  voice: clipped
  example: Door. Now.
  example: Not that one.
  present: scene:near_ann
";

/// Builds a story from `text` (header added) in a fresh folder with the catalog's bindings and a scene declaring `near_ann`.
fn build_text(name: &str, text: &str) -> Built {
    let dir = std::env::temp_dir().join(format!("red-story-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("bindings.json"), bindings_json("g", &catalog(), &[]).to_string()).unwrap();
    std::fs::write(dir.join("scene.json"), r#"{"vars": {"near_ann": false}, "objects": []}"#).unwrap();
    std::fs::write(dir.join("s.story"), format!("story t version 1\nuses bindings.json\nmap scene.json\nround g:bell\n{text}")).unwrap();
    build(&dir).unwrap()
}

fn messages(b: &Built) -> String {
    b.diags.iter().map(|d| d.render(&b.ast.files)).collect::<Vec<_>>().join("\n")
}

fn errors(text: &str) -> String {
    let b = build_text(&format!("e{:x}", text.len() * 31 + text.bytes().map(|c| c as usize).sum::<usize>()), text);
    assert!(b.errors() > 0, "expected errors in:\n{text}\ngot:\n{}", messages(&b));
    messages(&b)
}

const DOOR: &str = "
fact asked bool = false run \"the player asked Ann about the door\"
conversation door with ann
node start
  ann: The door. Will you open it?
  * yes: I'll open it.
    if !g:door_open
    kind consequential
    do run g:open; set asked = true
    -> thanks
  * no: Not now.
    -> end
node thanks
  ann: Good.
    if asked
  -> end
";

#[test]
fn a_well_formed_story_compiles_binds_and_says_why_each_choice_matters() {
    let b = build_text("ok", &format!("{SHEETS}{DOOR}"));
    assert_eq!(b.errors(), 0, "{}", messages(&b));
    let p = b.package.as_ref().unwrap();
    assert_eq!(p.story, "t");
    assert_eq!(p.conversations.len(), 1);
    let start = &p.lines[0];
    assert_eq!(start.choices[0].stakes, Stakes::Consequential);
    assert_eq!(start.choices[1].stakes, Stakes::Expressive, "traced: it changes nothing");
    // The runtime's own check and bind accept what the compiler wrote.
    let text = package_text(p);
    crate::sim::story::package::Loaded::from_json(&text, &catalog()).unwrap();
}

#[test]
fn syntax_and_foreign_syntax_are_refused_by_name_with_their_line() {
    let e = errors(&format!("{SHEETS}=== knot ===\n"));
    assert!(e.contains("s.story:16: error[story-syntax]: not supported: an Ink knot"), "{e}");
    let e = errors(&format!("{SHEETS}<<set $x to 1>>\n"));
    assert!(e.contains("a Yarn command"), "{e}");
    let e = errors(&format!("{SHEETS}conversation door with ann\n  ann: hello\n"));
    assert!(e.contains("a line of a conversation belongs to a node"), "{e}");
    let e = errors(&format!("{SHEETS}fact x number = 3 \"x\"\n"));
    assert!(e.contains("write `fact <id> bool|int"), "{e}");
}

#[test]
fn names_types_and_bindings_are_checked_with_a_did_you_mean() {
    let e = errors(&format!("{SHEETS}{}", DOOR.replace("g:door_open", "g:door_opn")));
    assert!(e.contains("no query the game answers named `g:door_opn` (did you mean `g:door_open`?)"), "{e}");
    let e = errors(&format!("{SHEETS}{}", DOOR.replace("run g:open", "run g:opne")));
    assert!(e.contains("did you mean `g:open`?"), "{e}");
    let e = errors(&format!("{SHEETS}{}", DOOR.replace("if asked", "if g:coins")));
    assert!(e.contains("`if g:coins` is a whole number, not a yes/no"), "{e}");
    let e = errors(&format!("{SHEETS}{}", DOOR.replace("set asked = true", "set asked = g:coins + 1")));
    assert!(e.contains("takes a yes/no"), "{e}");
    let e = errors(&format!("{SHEETS}{}", DOOR.replace("if asked", "if random(1, 2) == 1")));
    assert!(e.contains("`random` is not allowed in a story"), "{e}");
    let e = errors(&format!("{SHEETS}{}", DOOR.replace("ann: Good.", "bob: Good.")));
    assert!(e.contains("no speaker (a `character` or `extra`, or `narrator`) `bob`"), "{e}");
    let e = errors(&format!("{SHEETS}{DOOR}bark b on g:gong\n  ann: hi\n"));
    assert!(e.contains("no event the game emits named `g:gong`"), "{e}");
}

#[test]
fn a_director_may_only_run_what_the_game_lets_be_issued_unattended() {
    let e = errors(&format!("{SHEETS}{DOOR}director d\n  every 5s\n  intervene shove\n    do run g:pay 3\n"));
    assert!(e.contains("a director cannot run `g:pay`"), "{e}");
    let b =
        build_text("dir_ok", &format!("{SHEETS}{DOOR}director d\n  every 5s\n  rest if g:door_open\n  intervene nudge\n    chance 50%\n    do run g:hint\n"));
    assert_eq!(b.errors(), 0, "{}", messages(&b));
}

#[test]
fn dead_ends_endless_conversations_and_loops_that_never_wait_are_errors() {
    let e = errors(&format!("{SHEETS}conversation c with ann\nnode a\n  ann: Hi.\n"));
    assert!(e.contains("node `a` ends without saying where it goes"), "{e}");
    let e = errors(&format!("{SHEETS}conversation c with ann\nnode a\n  ann: Hi.\n  -> a\n"));
    assert!(e.contains("can never end"), "{e}");
    let e = errors(&format!(
        "{SHEETS}fact f bool = false run \"f\"\nconversation c with ann\nnode a\n  ann: Hi.\n    if f\n  -> b\nnode b\n  ann: Ho.\n    if f\n  -> a\nnode z\n  ann: Bye.\n  -> end\n"
    ));
    assert!(e.contains("a loop that may never wait for the player"), "{e}");
    let e = errors(&format!("{SHEETS}conversation c with ann\nnode a\n  ann: Pick.\n  * x: X\n"));
    assert!(e.contains("does not say where it goes"), "{e}");
    let e = errors(&format!("{SHEETS}conversation c with ann\nnode a\n  ann: Pick.\n  * x: X\n    -> nowhere\n"));
    assert!(e.contains("no node of `c` `nowhere`"), "{e}");
}

#[test]
fn a_profile_missing_a_required_field_is_incomplete_but_a_bland_one_is_not_judged() {
    let e = errors(&format!("character bo \"Bo\"\n  want: x\n{DOOR}"));
    assert!(e.contains("a character `bo` has no `need:`"), "{e}");
    assert!(e.contains("needs at least two"), "{e}");
    let e = errors(&format!("{SHEETS}extra kid \"Kid\"\n  voice: shouts\n{DOOR}"));
    assert!(e.contains("an extra `kid` has no `role:`"), "{e}");
    let e = errors(&format!("{SHEETS}  favourite_colour: blue\n{DOOR}"));
    assert!(e.contains("`favourite_colour` is not a profile field"), "{e}");
    // A stable arc stands in for a need; the words themselves are never scored.
    let b = build_text("stable", &format!("{}{DOOR}", SHEETS.replace("need: to ask for help", "arc: stable, she is the island's fixed point")));
    assert_eq!(b.errors(), 0, "{}", messages(&b));
}

#[test]
fn a_speaker_cannot_reveal_what_they_could_never_know() {
    let e = errors(&format!(
        "{SHEETS}info key \"the key is under the mat\"\nconversation c with ann\nnode a\n  ann: It's under the mat.\n    reveals key\n  -> end\n"
    ));
    assert!(e.contains("ann reveals key in `c/a/l1`, but can never know it"), "{e}");
    // Knowing it from her sheet, or claiming it, is fine.
    let ok = format!(
        "{}info key \"the key is under the mat\"\nconversation c with ann\nnode a\n  ann: It's under the mat.\n    reveals key\n  -> end\n",
        SHEETS.replace("present:", "knows: key\n  present:")
    );
    assert_eq!(build_text("knows", &ok).errors(), 0);
    let claim =
        format!("{SHEETS}info key \"the key is under the mat\"\nconversation c with ann\nnode a\n  ann: I bet it's under the mat.\n    claims key\n  -> end\n");
    assert_eq!(build_text("claims", &claim).errors(), 0);
}

#[test]
fn warnings_are_specific_and_an_allow_is_narrow_explained_and_listed() {
    let text = format!("{SHEETS}{DOOR}fact unused_fact bool = false run \"never used\"\nsetup promise \"Ann was promised\"\n  by door/yes\n");
    let b = build_text("warn", &text);
    let m = messages(&b);
    assert!(m.contains("[unused] `unused_fact` is declared and never used"), "{m}");
    assert!(m.contains("[unpaid] setup `promise`"), "{m}");
    assert_eq!(b.errors(), 0);
    // One warning, one thing, a reason: it is listed as allowed, and the other warning stays.
    let allowed = build_text("allow", &format!("{text}allow unpaid promise \"paid off in the sequel\"\n"));
    let m = messages(&allowed);
    assert!(m.contains("allowed unpaid promise") && m.contains("because: paid off in the sequel"), "{m}");
    assert!(m.contains("[unused] `unused_fact`"), "{m}");
    assert_eq!(allowed.allowed(), 1);
    // An allow without a reason, of a warning that does not exist, or that allows nothing: refused or reported.
    assert!(errors(&format!("{text}allow unpaid promise\n")).contains("a reason"));
    assert!(errors(&format!("{text}allow everything promise \"x\"\n")).contains("no such warning"));
    let stale = build_text("stale", &format!("{SHEETS}{DOOR}allow unpaid nothing \"x\"\n"));
    assert!(messages(&stale).contains("allows nothing"), "{}", messages(&stale));
}

#[test]
fn a_declared_stakes_that_the_choice_does_not_have_is_flagged() {
    let text = format!("{SHEETS}{}", DOOR.replace("  * no: Not now.\n", "  * no: Not now.\n    kind consequential\n"));
    let m = messages(&build_text("stakes", &text));
    assert!(m.contains("[stakes] `door/no` is declared consequential, but it ends the conversation, changing nothing"), "{m}");
}

#[test]
fn refs_say_who_reads_writes_and_depends() {
    let b = build_text("refs", &format!("{SHEETS}{DOOR}"));
    let f = refs::refs(&b, "asked").unwrap();
    assert_eq!(f["written_by"][0]["by"], "choice door/yes");
    assert_eq!(f["read_by"][0]["by"], "line door/thanks/l1");
    let c = refs::refs(&b, "door/yes").unwrap();
    assert_eq!(c["effects"], serde_json::json!(["run g:open", "set asked = true"]));
    let conv = refs::refs(&b, "door").unwrap();
    assert_eq!(conv["opens_when"]["they_are_present"], "scene:near_ann");
    let line = refs::refs(&b, "door/thanks/l1").unwrap();
    assert!(line["removing_it_breaks"].to_string().contains("jump"), "{line}");
    assert!(refs::refs(&b, "asekd").unwrap_err().contains("did you mean `asked`?"));
    let ledger = refs::ledger(&b).unwrap();
    assert_eq!(ledger["choices"][0]["stakes"], "consequential");
}

#[test]
fn the_exemplars_compile_cleanly() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/story");
    for name in ["the_debt", "the_rumour", "rain"] {
        let b = build(&root.join(name)).unwrap();
        assert_eq!((b.errors(), b.warnings()), (0, 0), "{name}: {}", messages(&b));
        assert!(b.package.is_some(), "{name}");
    }
}
