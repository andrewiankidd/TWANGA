// Windows: hide the console window when run as a GUI app (release builds).
// Dev builds keep the console attached so `println!` / panics surface.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    twanga_app::run();
}
