#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// The modules below have no callers until the card lands in the next commits.
#![allow(dead_code)]

mod cache;
mod dictionary;
mod lookup;
mod selection_input;

fn main() {}
