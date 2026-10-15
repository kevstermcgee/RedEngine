//! A compiled story package (`story build`, read by `sim::story::package::Loaded`) and a game's story bindings (`<game>_server story-bindings`):
//! both written by tools, read back by their own checkers (`sim::story::package::check`, the bindings' typed catalog). The drift test holds the
//! repository's files to both definitions but does not probe keys (as for traces). The `.story` sources they come from are text: `validate`
//! compiles each folder of them with the toolchain (`tools::story::build`).

use super::common::Any;
use crate::strict_struct;

strict_struct! {
    /// A compiled story: indexed tables with the ids and places they were written at (`describe story`).
    pub struct StoryPackage {
        story_package: u32,
        story: String,
        content_version: u32,
        source_hash: String,
        sources: Vec<String>,
        round: String,
        holders: Vec<String>,
        speakers: Vec<Any>,
        facts: Vec<Any>,
        infos: Vec<Any>,
        vars: Vec<Any>,
        exprs: Vec<String>,
        authored: Vec<String>,
        lines: Vec<Any>,
        conversations: Vec<Any>,
        barks: Vec<Any>,
        categories: Vec<Any>,
        directors: Vec<Any>,
    }
}

strict_struct! {
    /// What a game offers a story: its queries, commands (and the unattended ones), events, and what each means.
    pub struct StoryBindings {
        story_bindings: u32,
        game: String,
        about: String,
        catalog: Any,
        notes: Any,
    }
}
