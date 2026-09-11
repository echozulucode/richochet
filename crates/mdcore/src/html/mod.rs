//! HTML <-> document model.

pub mod parse;
pub mod render;
pub mod styles;

pub use parse::parse;
pub use render::render;
