// Pedantic doc lints (missing_errors_doc, missing_panics_doc) are allowed here;
// the crate is only used inside this workspace and fails with MeshThumbnailError throughout.
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod error;
pub mod extract_image;
pub mod mesh;
pub mod parse_model;
pub mod path_ext;
pub mod render;
pub mod scene;
