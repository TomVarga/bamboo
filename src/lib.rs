//! Bamboo compresses local PNG and JPEG files with the Tinify API.
//!
//! Originally written by Chris Anselmo. Rewritten in Rust by Tamás Varga and Grok.

pub mod files;
pub mod settings;
pub mod state;
pub mod tinify;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SOURCE_URL: &str = "https://github.com/TomVarga/bamboo";
pub const API_KEY_URL: &str = "https://tinypng.com/developers";
