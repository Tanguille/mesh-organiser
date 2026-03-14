use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
pub enum FileType {
    Stl,
    ZippedStl,
    Obj,
    ZippedObj,
    Gcode,
    ZippedGcode,
    Step,
    ZippedStep,
    Threemf,
    Unknown,
}

#[derive(Serialize, Clone)]
pub struct Blob {
    pub id: i64,
    pub sha256: String,
    pub filetype: String,
    pub size: i64,
    pub added: String,
    pub disk_path: Option<String>,
}

impl Blob {
    #[must_use]
    pub fn to_file_type(&self) -> FileType {
        FileType::from_extension(&self.filetype)
    }
}

impl FileType {
    #[must_use]
    pub fn from_pathbuf(path: &Path) -> Self {
        let name_lower = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_lowercase();
        // Compound extensions first so "file.stl.zip" is recognized as ZippedStl
        if name_lower.ends_with(".stl.zip") {
            return Self::from_extension("stl.zip");
        }
        if name_lower.ends_with(".obj.zip") {
            return Self::from_extension("obj.zip");
        }
        if name_lower.ends_with(".gcode.zip") {
            return Self::from_extension("gcode.zip");
        }
        if name_lower.ends_with(".step.zip") || name_lower.ends_with(".stp.zip") {
            return Self::from_extension(if name_lower.ends_with(".stp.zip") {
                "stp.zip"
            } else {
                "step.zip"
            });
        }
        path.extension().map_or(Self::Unknown, |ext| {
            Self::from_extension(&ext.to_string_lossy())
        })
    }

    #[must_use]
    pub fn from_extension(extension: &str) -> Self {
        match extension.to_lowercase().as_str() {
            f if f.ends_with("stl") => Self::Stl,
            f if f.ends_with("stl.zip") => Self::ZippedStl,
            f if f.ends_with("obj") => Self::Obj,
            f if f.ends_with("obj.zip") => Self::ZippedObj,
            f if f.ends_with("gcode") => Self::Gcode,
            f if f.ends_with("gcode.zip") => Self::ZippedGcode,
            f if f.ends_with("step") => Self::Step,
            f if f.ends_with("stp") => Self::Step,
            f if f.ends_with("step.zip") => Self::ZippedStep,
            f if f.ends_with("stp.zip") => Self::ZippedStep,
            f if f.ends_with("3mf") => Self::Threemf,
            _ => Self::Unknown,
        }
    }

    /// # Panics
    /// Panics if `self` is `FileType::Unknown` (cannot convert to an extension).
    #[must_use]
    pub fn to_extension(&self) -> String {
        match self {
            Self::Stl => "stl",
            Self::ZippedStl => "stl.zip",
            Self::Obj => "obj",
            Self::ZippedObj => "obj.zip",
            Self::Gcode => "gcode",
            Self::ZippedGcode => "gcode.zip",
            Self::Step => "step",
            Self::ZippedStep => "step.zip",
            Self::Threemf => "3mf",
            Self::Unknown => panic!("Cannot convert Unknown FileType to extension"),
        }
        .to_string()
    }

    #[must_use]
    pub const fn is_zipped(&self) -> bool {
        matches!(
            self,
            Self::ZippedStl | Self::ZippedObj | Self::ZippedGcode | Self::ZippedStep
        )
    }

    #[must_use]
    pub const fn is_stl(&self) -> bool {
        matches!(self, Self::Stl | Self::ZippedStl)
    }

    #[must_use]
    pub const fn is_obj(&self) -> bool {
        matches!(self, Self::Obj | Self::ZippedObj)
    }

    #[must_use]
    pub const fn is_3mf(&self) -> bool {
        matches!(self, Self::Threemf)
    }

    #[must_use]
    pub const fn is_step(&self) -> bool {
        matches!(self, Self::Step | Self::ZippedStep)
    }

    #[must_use]
    pub const fn is_gcode(&self) -> bool {
        matches!(self, Self::Gcode | Self::ZippedGcode)
    }

    #[must_use]
    pub const fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    #[must_use]
    pub const fn is_zippable(&self) -> bool {
        matches!(self, Self::Stl | Self::Obj | Self::Step | Self::Gcode)
    }

    #[must_use]
    pub fn to_zip(&self) -> Self {
        match self {
            Self::Stl => Self::ZippedStl,
            Self::Obj => Self::ZippedObj,
            Self::Step => Self::ZippedStep,
            Self::Gcode => Self::ZippedGcode,
            _ => self.clone(),
        }
    }

    #[must_use]
    pub fn from_zip(&self) -> Self {
        match self {
            Self::ZippedStl => Self::Stl,
            Self::ZippedObj => Self::Obj,
            Self::ZippedStep => Self::Step,
            Self::ZippedGcode => Self::Gcode,
            _ => self.clone(),
        }
    }

    #[must_use]
    pub const fn is_supported_by_thumbnail_gen(&self) -> bool {
        matches!(
            self,
            Self::Stl
                | Self::ZippedStl
                | Self::Obj
                | Self::ZippedObj
                | Self::Gcode
                | Self::ZippedGcode
                | Self::Step
                | Self::ZippedStep
                | Self::Threemf
        )
    }

    #[must_use]
    pub const fn is_importable(&self) -> bool {
        matches!(
            self,
            Self::Stl | Self::Obj | Self::Gcode | Self::Step | Self::Threemf
        )
    }
}
