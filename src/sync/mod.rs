//! Local-first synchronization with the Google Drive app data folder.
//!
//! - [`document`] is the on-the-wire JSON document and its validation.
//! - [`merge`] is the pure three-way merge.
//! - [`store`] maps the document to SQLite and records conflicts.
//! - [`engine`] runs a single-flight sync cycle against a cloud drive.

pub mod document;
pub mod engine;
pub mod merge;
pub mod store;
