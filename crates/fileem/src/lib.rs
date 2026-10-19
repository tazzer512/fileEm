//! fileEm's window (Slint UI over `fileem-core`). A library so integration tests can build the
//! real window headless; `main.rs` only starts it.

slint::include_modules!();

pub mod app;
pub mod bg;
pub mod ops_ui;
pub mod progress;
pub mod shell;
pub mod tab;
pub mod tree;
