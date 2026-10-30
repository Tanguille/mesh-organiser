//! Characterization tests for the per-format dispatch in `parse_model::handle_parse`
//! and `extract_image::handle_extract_image`.
//!
//! Pins which paths each dispatcher claims (by extension, case-insensitively, plain
//! or `.<ext>.zip`), that a claimed-but-broken file surfaces the handler's error
//! instead of falling through to a later handler, and that anything unclaimed
//! (including a plain `.zip` holding a model) returns `None`.

use std::{
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use base64::{Engine, prelude::BASE64_STANDARD};
use image::{DynamicImage, ImageFormat, RgbaImage};
use libmeshthumbnail::{
    error::MeshThumbnailError, extract_image::handle_extract_image, parse_model::handle_parse,
};
use tempfile::TempDir;
use zip::{ZipWriter, write::SimpleFileOptions};

const TRIANGLE_STL: &str = "solid t
facet normal 0 0 1
outer loop
vertex 0 0 0
vertex 10 0 0
vertex 0 10 0
endloop
endfacet
endsolid t
";

const TRIANGLE_OBJ: &str = "v 0 0 0\nv 10 0 0\nv 0 10 0\nf 1 2 3\n";

const TRIANGLE_3MF_MODEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
  <resources>
    <object id="1" type="model">
      <mesh>
        <vertices>
          <vertex x="0" y="0" z="0"/>
          <vertex x="10" y="0" z="0"/>
          <vertex x="0" y="10" z="0"/>
        </vertices>
        <triangles>
          <triangle v1="0" v2="1" v3="2"/>
        </triangles>
      </mesh>
    </object>
  </resources>
  <build>
    <item objectid="1"/>
  </build>
</model>
"#;

/// Three extruding moves give two non-degenerate edges.
const THREE_MOVE_GCODE: &str = "G1 X0 Y0 E1\nG1 X10 Y0 E1\nG1 X10 Y10 E1\n";

const TRIANGLE_VERTICES: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]];

fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = ZipWriter::new(fs::File::create(path).unwrap());
    for (name, bytes) in entries {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(RgbaImage::new(width, height))
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();

    bytes
}

/// G-code carrying a small and a large embedded thumbnail plus real moves.
fn gcode_with_thumbnails() -> String {
    let small = BASE64_STANDARD.encode(png_bytes(2, 1));
    let large = BASE64_STANDARD.encode(png_bytes(4, 2));

    format!(
        "; thumbnail begin 2x1 {}\n; {small}\n; thumbnail end\n\
         ; thumbnail begin 4x2 {}\n; {large}\n; thumbnail end\n{THREE_MOVE_GCODE}",
        small.len(),
        large.len(),
    )
}

fn write_file(directory: &TempDir, name: &str, contents: impl AsRef<[u8]>) -> PathBuf {
    let path = directory.path().join(name);
    fs::write(&path, contents).unwrap();

    path
}

fn write_zip_file(directory: &TempDir, name: &str, entries: &[(&str, &[u8])]) -> PathBuf {
    let path = directory.path().join(name);
    write_zip(&path, entries);

    path
}

fn parsed_mesh(path: &Path) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mesh = handle_parse(path)
        .unwrap_or_else(|e| panic!("{} should parse: {e}", path.display()))
        .unwrap_or_else(|| panic!("{} should be claimed by a handler", path.display()));
    let vertices = mesh
        .vertices
        .iter()
        .map(|vertex| [vertex.x, vertex.y, vertex.z])
        .collect();

    (vertices, mesh.indices)
}

fn internal_error_message<T>(result: Result<T, MeshThumbnailError>) -> String {
    match result {
        Err(MeshThumbnailError::InternalError(message)) => message,
        Err(e) => panic!("expected InternalError, got {e:?}"),
        Ok(_) => panic!("expected InternalError, got Ok"),
    }
}

#[test]
fn handle_parse_claims_each_mesh_format_and_its_zip() {
    let directory = TempDir::new().unwrap();
    let triangle = (TRIANGLE_VERTICES.to_vec(), vec![0, 1, 2]);

    let stl = write_file(&directory, "model.stl", TRIANGLE_STL);
    let stl_upper = write_file(&directory, "MODEL.STL", TRIANGLE_STL);
    let stl_zip = write_zip_file(
        &directory,
        "model.stl.zip",
        &[
            ("readme.txt", b"ignored"),
            ("inner.stl", TRIANGLE_STL.as_bytes()),
        ],
    );
    let obj = write_file(&directory, "model.obj", TRIANGLE_OBJ);
    let obj_zip = write_zip_file(
        &directory,
        "model.OBJ.ZIP",
        &[("inner.obj", TRIANGLE_OBJ.as_bytes())],
    );
    let threemf = write_zip_file(
        &directory,
        "model.3mf",
        &[("3D/3dmodel.model", TRIANGLE_3MF_MODEL.as_bytes())],
    );

    assert_eq!(parsed_mesh(&stl), triangle);
    assert_eq!(parsed_mesh(&stl_upper), triangle);
    assert_eq!(parsed_mesh(&stl_zip), triangle);

    // wavefront_obj hands the face over starting from its last corner, so the OBJ
    // vertex list is rotated by one; the triangle itself is the same.
    let rotated_triangle = (
        vec![
            TRIANGLE_VERTICES[2],
            TRIANGLE_VERTICES[0],
            TRIANGLE_VERTICES[1],
        ],
        vec![0, 1, 2],
    );
    assert_eq!(parsed_mesh(&obj), rotated_triangle);
    assert_eq!(parsed_mesh(&obj_zip), rotated_triangle);

    assert_eq!(parsed_mesh(&threemf), triangle);

    // Each non-degenerate gcode edge becomes a 3-sided cylinder: 6 vertices, 18 indices.
    let gcode = write_file(&directory, "print.gcode", THREE_MOVE_GCODE);
    let gcode_zip = write_zip_file(
        &directory,
        "print.gcode.zip",
        &[("inner.gcode", THREE_MOVE_GCODE.as_bytes())],
    );
    let gcode_with_thumbnail = write_file(&directory, "thumb.gcode", gcode_with_thumbnails());
    for path in [&gcode, &gcode_zip, &gcode_with_thumbnail] {
        let (vertices, indices) = parsed_mesh(path);
        assert_eq!(
            (vertices.len(), indices.len()),
            (12, 36),
            "{}",
            path.display()
        );
    }
}

#[test]
fn handle_parse_returns_none_for_unclaimed_paths() {
    let directory = TempDir::new().unwrap();
    let unclaimed = [
        write_file(&directory, "notes.txt", TRIANGLE_STL),
        write_file(&directory, "model.stl.bak", TRIANGLE_STL),
        write_file(&directory, "no_extension", TRIANGLE_STL),
        // A plain `.zip` is not claimed even when it holds a model.
        write_zip_file(
            &directory,
            "archive.zip",
            &[("inner.stl", TRIANGLE_STL.as_bytes())],
        ),
        write_zip_file(
            &directory,
            "model.3mf.zip",
            &[("3D/3dmodel.model", TRIANGLE_3MF_MODEL.as_bytes())],
        ),
    ];

    for path in unclaimed {
        assert!(
            handle_parse(&path).unwrap().is_none(),
            "{} should not be claimed",
            path.display()
        );
    }
}

#[test]
fn handle_parse_step_fixture_is_claimed_only_with_step_feature() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cube.step");

    let parsed = handle_parse(&fixture).expect("cube.step should not error");

    assert_eq!(parsed.is_some(), cfg!(feature = "step"));
}

#[test]
fn handle_parse_surfaces_the_claiming_handlers_error() {
    let directory = TempDir::new().unwrap();

    let stl_zip_without_stl = write_zip_file(
        &directory,
        "model.stl.zip",
        &[("inner.obj", TRIANGLE_OBJ.as_bytes())],
    );
    let obj_zip_without_obj = write_zip_file(
        &directory,
        "model.obj.zip",
        &[("inner.stl", TRIANGLE_STL.as_bytes())],
    );
    let gcode_zip_without_gcode = write_zip_file(
        &directory,
        "print.gcode.zip",
        &[("inner.txt", b"G1 X0 Y0 E1")],
    );
    let gcode_without_moves =
        write_file(&directory, "empty.gcode", "; only a comment\nM104 S200\n");
    let obj_not_utf8 = write_file(&directory, "binary.obj", [0xff_u8, 0xfe, 0xfd]);
    let threemf_without_mesh = write_zip_file(
        &directory,
        "empty.3mf",
        &[(
            "3D/3dmodel.model",
            b"<model><resources></resources><build></build></model>",
        )],
    );

    assert_eq!(
        internal_error_message(handle_parse(&stl_zip_without_stl)),
        "Failed to find .stl model in zip"
    );
    assert_eq!(
        internal_error_message(handle_parse(&obj_zip_without_obj)),
        "Failed to find .obj model in zip"
    );
    assert_eq!(
        internal_error_message(handle_parse(&gcode_zip_without_gcode)),
        "Failed to find .gcode file in zip"
    );
    assert_eq!(
        internal_error_message(handle_parse(&gcode_without_moves)),
        "Gcode file contains no move instructions"
    );
    assert!(
        internal_error_message(handle_parse(&obj_not_utf8))
            .starts_with("OBJ content is not valid UTF-8: ")
    );
    assert_eq!(
        internal_error_message(handle_parse(&threemf_without_mesh)),
        "No meshes found in 3mf model"
    );

    let threemf_not_zip = write_file(&directory, "broken.3mf", "not a zip");
    assert!(matches!(
        handle_parse(&threemf_not_zip),
        Err(MeshThumbnailError::ThreemfParseError(_))
    ));
}

#[test]
fn handle_extract_image_claims_3mf_and_gcode_only() {
    let directory = TempDir::new().unwrap();
    let thumbnail = png_bytes(4, 2);

    let threemf = write_zip_file(
        &directory,
        "model.3mf",
        &[
            ("3D/3dmodel.model", TRIANGLE_3MF_MODEL.as_bytes()),
            ("Metadata/plate_1.png", &png_bytes(8, 8)),
            ("Metadata/thumbnail_middle.png", &thumbnail),
        ],
    );
    let gcode = write_file(&directory, "print.GCODE", gcode_with_thumbnails());
    let gcode_zip = write_zip_file(
        &directory,
        "print.gcode.zip",
        &[("inner.gcode", gcode_with_thumbnails().as_bytes())],
    );

    for path in [&threemf, &gcode, &gcode_zip] {
        let image = handle_extract_image(path)
            .unwrap_or_else(|e| panic!("{} should extract: {e}", path.display()))
            .unwrap_or_else(|| panic!("{} should be claimed", path.display()));
        assert_eq!(
            (image.width(), image.height()),
            (4, 2),
            "{}",
            path.display()
        );
    }

    let unclaimed = [
        write_file(&directory, "model.stl", TRIANGLE_STL),
        write_file(&directory, "model.obj", TRIANGLE_OBJ),
        write_zip_file(
            &directory,
            "archive.zip",
            &[("Metadata/thumbnail_middle.png", &thumbnail)],
        ),
    ];
    for path in unclaimed {
        assert!(
            handle_extract_image(&path).unwrap().is_none(),
            "{} should not be claimed",
            path.display()
        );
    }
}

#[test]
fn handle_extract_image_gcode_header_area_does_not_overflow() {
    let directory = TempDir::new().unwrap();
    let small = BASE64_STANDARD.encode(png_bytes(2, 1));
    let large = BASE64_STANDARD.encode(png_bytes(4, 2));
    // 65536 * 65536 does not fit in u32, so the header area must not be u32 arithmetic: it
    // would panic in debug builds and wrap to 0 in release, losing to the 2x1 header.
    let gcode = write_file(
        &directory,
        "print.gcode",
        format!(
            "; thumbnail begin 2x1 {}\n; {small}\n; thumbnail end\n\
             ; thumbnail begin 65536x65536 {}\n; {large}\n; thumbnail end\n",
            small.len(),
            large.len(),
        ),
    );

    let image = handle_extract_image(&gcode).unwrap().unwrap();

    assert_eq!((image.width(), image.height()), (4, 2));
}

#[test]
fn handle_extract_image_surfaces_missing_thumbnail_errors() {
    let directory = TempDir::new().unwrap();

    let threemf_without_thumbnail = write_zip_file(
        &directory,
        "model.3mf",
        &[("3D/3dmodel.model", TRIANGLE_3MF_MODEL.as_bytes())],
    );
    let gcode_without_thumbnail = write_file(&directory, "print.gcode", THREE_MOVE_GCODE);
    let gcode_zip_without_gcode = write_zip_file(
        &directory,
        "print.gcode.zip",
        &[("inner.txt", THREE_MOVE_GCODE.as_bytes())],
    );

    assert_eq!(
        internal_error_message(handle_extract_image(&threemf_without_thumbnail)),
        "thumbnail_middle.png not found in 3mf file"
    );
    assert_eq!(
        internal_error_message(handle_extract_image(&gcode_without_thumbnail)),
        "No thumbnail found in gcode file"
    );
    assert_eq!(
        internal_error_message(handle_extract_image(&gcode_zip_without_gcode)),
        "No gcode file found in zip archive"
    );
}
