//! File system access (the Java `com.fileem.fs` package).

pub mod entry;
pub mod file_types;
pub mod listing;
pub mod places;
pub mod trash;

pub use entry::FileEntry;
pub use file_types::Category;
