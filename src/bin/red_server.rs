//! `red_server`: the authoritative, headless Red multiplayer server (see `red_engine2::server_app` for its flags; a game's own server runs the same
//! program with its own scene blocks and match games).

fn main() {
    red_engine2::server_app::main()
}
