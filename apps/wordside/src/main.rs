#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod cache;
mod card;
// Some backend diagnostics are only reached by the CLI entries added later in this milestone.
#[allow(dead_code)]
mod dictionary;
mod entry;
mod lookup;
mod presentation;
mod render;
mod selection_input;
mod service;
mod settings;

fn main() -> windows::core::Result<()> {
    app::run()
}
