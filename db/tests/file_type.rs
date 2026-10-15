//! Characterization tests for `FileType` detection from stored extensions and import paths.

use db::model::blob::FileType;

#[test]
fn from_extension_maps_every_branch_case_insensitively() {
    let cases = [
        ("stl", FileType::Stl),
        ("STL", FileType::Stl),
        ("stl.zip", FileType::ZippedStl),
        ("STL.ZIP", FileType::ZippedStl),
        ("obj", FileType::Obj),
        ("obj.zip", FileType::ZippedObj),
        ("gcode", FileType::Gcode),
        ("gcode.zip", FileType::ZippedGcode),
        ("step", FileType::Step),
        ("stp", FileType::Step),
        ("STP", FileType::Step),
        ("step.zip", FileType::ZippedStep),
        ("stp.zip", FileType::ZippedStep),
        ("3mf", FileType::Threemf),
        ("3MF", FileType::Threemf),
        ("3mf.zip", FileType::Unknown),
        ("zip", FileType::Unknown),
        ("txt", FileType::Unknown),
        ("", FileType::Unknown),
    ];

    for (extension, expected) in cases {
        assert_eq!(
            FileType::from_extension(extension),
            expected,
            "extension {extension:?}"
        );
    }
}

#[test]
fn from_pathbuf_detects_plain_compound_and_missing_extensions() {
    let cases = [
        ("model.stl", FileType::Stl),
        ("MODEL.STL", FileType::Stl),
        ("/library/model.Stl", FileType::Stl),
        ("model.v2.stl", FileType::Stl),
        ("model.obj", FileType::Obj),
        ("model.gcode", FileType::Gcode),
        ("model.step", FileType::Step),
        ("model.stp", FileType::Step),
        ("model.3mf", FileType::Threemf),
        ("model.stl.zip", FileType::ZippedStl),
        ("MODEL.STL.ZIP", FileType::ZippedStl),
        ("model.obj.zip", FileType::ZippedObj),
        ("model.gcode.zip", FileType::ZippedGcode),
        ("model.step.zip", FileType::ZippedStep),
        ("model.stp.zip", FileType::ZippedStep),
        ("model.3mf.zip", FileType::Unknown),
        ("archive.zip", FileType::Unknown),
        ("notes.txt", FileType::Unknown),
        ("model", FileType::Unknown),
        ("/library/gcode", FileType::Unknown),
        ("/library/v1.2/part.stl", FileType::Stl),
        ("/library/models.stl/readme", FileType::Unknown),
        ("/library/models.stl.zip/readme", FileType::Unknown),
    ];

    for (path, expected) in cases {
        assert_eq!(
            FileType::from_pathbuf(std::path::Path::new(path)),
            expected,
            "path {path:?}"
        );
    }
}
