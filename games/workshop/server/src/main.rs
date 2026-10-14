//! `workshop_server`: hosts Workshop matches, and is the engine's command line with Workshop's blocks (`workshop_server cli ...`).

fn main() {
    red_engine2::server_app::run(&workshop_server::SERVER)
}
