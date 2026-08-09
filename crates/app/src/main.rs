#![cfg_attr(windows, windows_subsystem = "windows")]

mod application;

fn main() -> eframe::Result {
    application::run()
}
