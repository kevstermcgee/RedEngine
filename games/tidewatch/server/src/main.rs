//! `tidewatch_server`: hosts Tidewatch matches; `tidewatch_server cli ...` is the engine's command line with Tidewatch's block; `tidewatch_server
//! story-bindings` prints the bindings its story is compiled against.

fn main() {
    if std::env::args().nth(1).as_deref() == Some("story-bindings") {
        println!("{}", serde_json::to_string_pretty(&tidewatch_server::bindings()).unwrap_or_default());
        return;
    }
    red_engine2::server_app::run(&tidewatch_server::SERVER)
}
