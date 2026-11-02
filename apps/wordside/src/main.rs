#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod audio;
mod cache;
mod card;
mod cli;
mod dictionary;
mod entry;
mod icon;
mod lookup;
mod menu;
mod presentation;
mod render;
mod selection_input;
mod service;
mod settings;
// Copied unchanged from 0.2.0's capture layer; `HotkeyStatus` returns with capture in N3.
#[allow(dead_code)]
mod shortcut;
mod tray;

fn main() -> windows::core::Result<()> {
    if cli::run() {
        return Ok(());
    }
    app::run()
}
