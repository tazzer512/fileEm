//! fileEm's core, free of any UI toolkit: everything the app does to the file system.
//!
//! Ported from the Java version's `util`, `fs` and `ops` packages; the UI crate (`fileem`) only
//! calls into this one, so its behavior can be tested without a window.

pub mod fs;
pub mod images;
pub mod ops;
pub mod search_history;
pub mod session;
pub mod util;
