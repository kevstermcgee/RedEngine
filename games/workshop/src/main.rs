//! `workshop [SCENE]`: Workshop in a window. Progress is saved in the game's save folder (`RE2_SAVE_DIR`, else the per-user one).

fn main() {
    let path = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(workshop::default_scene);
    let game = workshop::Workshop::load(&path, workshop::save_path(&path)).unwrap_or_else(|errs| {
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
