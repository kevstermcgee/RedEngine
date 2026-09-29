//! `cargo run` (from this directory): opens the top-down game window. An optional argument names another scene.

fn main() {
    let path = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(topdown_switch::default_scene);
    let game = topdown_switch::TopDown::load(&path).unwrap_or_else(|errs| {
        eprintln!("{} is not a valid scene:\n  {}", path.display(), errs.join("\n  "));
        std::process::exit(1);
    });
    if let Err(e) = red_engine2::app::run(game, red_engine2::app::WindowOptions::default()) {
        eprintln!("{e:#}");
        std::process::exit(1);
    }
}
