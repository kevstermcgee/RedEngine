//! `red_engine2 propose`: a buildable plan from an idea.

use super::*;

/// `propose`: a plan from an idea.
pub(crate) fn run_propose(
    idea: &str,
    title: Option<String>,
    presentation: Option<String>,
    platforms: Vec<String>,
    inputs: Vec<String>,
    networking: Option<String>,
    session: Option<u32>,
) -> Result<(), String> {
    use red2d::caps::{Input, Networking, Platform, Presentation};
    if idea.trim().is_empty() {
        return Err("say the idea in a few words, like `propose \"a small arcade game where you dodge asteroids\"`".into());
    }
    fn one<T>(what: &str, v: &str, parse: fn(&str) -> Option<T>, names: Vec<&'static str>) -> Result<T, String> {
        parse(v).ok_or_else(|| format!("--{what} `{v}` is not one of {}", names.join(", ")))
    }
    let o = red_engine2::tools::propose::Overrides {
        title,
        presentation: presentation.as_deref().map(|p| one("presentation", p, Presentation::parse, Presentation::names())).transpose()?,
        platforms: platforms.iter().map(|p| one("platform", p, Platform::parse, Platform::names())).collect::<Result<_, _>>()?,
        input: inputs.iter().map(|p| one("input", p, Input::parse, Input::names())).collect::<Result<_, _>>()?,
        networking: networking.as_deref().map(|p| one("networking", p, Networking::parse, Networking::names())).transpose()?,
        session_minutes: session,
    };
    let p = red_engine2::tools::propose::propose(idea, &o);
    if red_engine2::tools::envelope::capturing() {
        println!("{}", serde_json::to_string_pretty(&p.to_json()).unwrap_or_default());
    } else {
        print!("{}", p.render());
    }
    if p.buildable() {
        Ok(())
    } else {
        Err(String::new())
    }
}
