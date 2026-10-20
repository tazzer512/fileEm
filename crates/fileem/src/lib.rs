//! fileEm's window (Slint UI over `fileem-core`). A library so integration tests can build the
//! real window headless; `main.rs` only starts it.

slint::include_modules!();

pub mod address;
pub mod app;
pub mod bg;
pub mod clipboard;
pub mod columns;
pub mod grid;
pub mod menu;
pub mod ops_ui;
pub mod progress;
pub mod properties;
pub mod shell;
pub mod tab;
pub mod thumbs;
pub mod tree;
pub mod view_mode;
