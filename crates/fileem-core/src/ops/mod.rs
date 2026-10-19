//! File operations (the Java `com.fileem.ops` package).

pub mod file_operation;

#[cfg(test)]
mod tests;

pub use file_operation::{ConflictHandler, Decision, FileOperation, Handle, Kind, Resolution, State};
