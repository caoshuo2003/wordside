#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod cache;
mod capture;
mod card;
mod cli;
mod dictionary;
mod dismissal;
mod entry;
mod icon;
mod lookup;
mod menu;
mod presentation;
mod render;
mod selection_input;
mod service;
mod settings;
mod tray;

fn main() -> windows::core::Result<()> {
    // The selection helper is this exe started by the capture layer; it opens no window.
    if capture::probe::helper_entry() || cli::run() {
        return Ok(());
    }
    app::run()
}
