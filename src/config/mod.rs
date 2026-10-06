#![deny(clippy::print_stdout, clippy::print_stderr)]

pub mod auth;
pub mod manifest;

pub use manifest::Manifest;
