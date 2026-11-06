#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod desktop;
mod hotkey;
mod startup;
fn main() {
    desktop::run();
}
