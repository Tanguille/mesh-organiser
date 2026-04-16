//! Slicer kind and serde surface. Platform integration lives in `linux`, `macos`, and `win`.
//! Detection and process spawn use async-friendly APIs: prefer [`Slicer::is_installed_async`]
//! in `async` contexts so work runs on the blocking pool; [`Slicer::is_installed`] stays
//! available for synchronous callers (for example `Configuration::default`).

use serde::{Deserialize, Serialize};
use strum::EnumIter;

#[derive(Clone, Serialize, Deserialize, EnumIter)]
#[allow(clippy::enum_variant_names)]
pub enum Slicer {
    PrusaSlicer,
    OrcaSlicer,
    Cura,
    BambuStudio,
    Custom,
}
