//! `re2`, the first-person walk-around client: the program is library code, `red_engine2::client`, so a game binary can run it too (ADR
//! 2026-10-14-a-game-ships-as-its-own-binary-the-client-is-a-library). See that module for what it does.
fn main() {
    red_engine2::client::main()
}
