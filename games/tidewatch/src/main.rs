//! `tidewatch [SCENE] [--no-pad]`: Tidewatch in a window. Progress is saved in the game's save folder (`RE2_SAVE_DIR`, else the per-user one).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pad = !args.iter().any(|a| a == "--no-pad");
    let path = args.iter().find(|a| !a.starts_with("--")).map(std::path::PathBuf::from).unwrap_or_else(tidewatch::default_scene);
    let game = tidewatch::Tidewatch::load(&path, tidewatch::save_path(&path), pad).unwrap_or_else(|errs| {
        eprintln!("{} is not a valid scene:\n  {}", path.display(), errs.join("\n  "));
        std::process::exit(1);
    });
    if let Some(note) = game.save_note() {
        eprintln!("{note}");
    }
    if let Err(e) = red_engine2::app::run(game, red_engine2::app::WindowOptions::default()) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
