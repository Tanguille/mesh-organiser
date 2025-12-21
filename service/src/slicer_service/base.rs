use serde::{Deserialize, Serialize};
use strum::EnumIter;

// TODO: Make all of this async
#[derive(Clone, Serialize, Deserialize, EnumIter)]
#[allow(clippy::enum_variant_names)]
pub enum Slicer {
    PrusaSlicer,
    OrcaSlicer,
    Cura,
    BambuStudio,
    Custom,
}
