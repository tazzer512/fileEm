//! File system access (the Java `com.fileem.fs` package).

pub mod entry;
pub mod file_types;
pub mod listing;
pub mod measure;
pub mod places;
pub mod preview;
pub mod search;
pub mod trash;
pub mod volumes;
pub mod watcher;

pub use entry::FileEntry;
pub use file_types::Category;
