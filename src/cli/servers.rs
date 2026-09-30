//! `servers`: list, inspect, start, stop and restart the Red game servers on this machine. The logic is `tools::servers`; this only parses, calls it and prints (or emits JSON under `--json`).

use super::*;
use red_engine2::tools::servers::{self, SystemdUser, Verb};

/// `servers [list|status|start|stop|restart|logs]`.
pub(crate) fn run_servers(cmd: Option<ServersCmd>) -> Result<(), String> {
    let b = SystemdUser;
    let json = envelope::capturing();
    let fleet = servers::discover(&b)?;
    match cmd.unwrap_or(ServersCmd::List) {
        ServersCmd::List => {
            if json {
                println!("{}", serde_json::to_string_pretty(&servers::to_json(&fleet)).unwrap_or_default());
            } else {
                print!("{}", servers::render(&fleet));
            }
            Ok(())
        }
        ServersCmd::Status { name } => {
            let s = servers::resolve(&fleet.servers, &name)?;
            if json {
                let all = servers::to_json(&fleet);
                let one = all["servers"].as_array().and_then(|a| a.iter().find(|v| v["name"] == s.name.as_str())).cloned().unwrap_or_default();
                println!("{}", serde_json::to_string_pretty(&one).unwrap_or_default());
            } else {
                print!("{}", servers::render_status(s));
                println!("  recent log:");
                servers::log_lines(&s.unit, 8, false, &mut |l| println!("    {l}"))?;
            }
            Ok(())
        }
        ServersCmd::Start { name, wait } => act(&b, &fleet, &name, Verb::Start, false, wait, json),
        ServersCmd::Restart { name, yes, wait } => act(&b, &fleet, &name, Verb::Restart, yes, wait, json),
        ServersCmd::Stop { name, pid, yes } => match (name, pid) {
            (Some(name), None) => act(&b, &fleet, &name, Verb::Stop, yes, None, json),
            (None, Some(pid)) => {
                let line = servers::stop_unmanaged(&b, &fleet, pid, yes)?;
                say(&[line], json);
                Ok(())
            }
            _ => Err("stop needs a server name, or --pid for a stray red_server".to_string()),
        },
        ServersCmd::Logs { name, lines, follow } => {
            let s = servers::resolve(&fleet.servers, &name)?;
            servers::log_lines(&s.unit, lines, follow, &mut |l| println!("{l}"))
        }
    }
}

fn act(b: &SystemdUser, fleet: &servers::Fleet, name: &str, verb: Verb, yes: bool, wait: Option<u64>, json: bool) -> Result<(), String> {
    let s = servers::resolve(&fleet.servers, name)?;
    let lines = servers::apply(b, &fleet.servers, s, verb, yes, wait)?;
    say(&lines, json);
    Ok(())
}

fn say(lines: &[String], json: bool) {
    if json {
        println!("{}", serde_json::json!({"ok": true, "messages": lines}));
    } else {
        for l in lines {
            println!("{l}");
        }
    }
}
