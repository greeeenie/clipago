#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod desktop;
mod hotkey;
mod startup;
mod ui_update;
fn main() {
    desktop::run();
}
