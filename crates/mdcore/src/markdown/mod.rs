//! Markdown <-> document model.

pub mod escape;
pub mod parse;
pub mod render;

pub use parse::{outline, parse};
pub use render::render;
