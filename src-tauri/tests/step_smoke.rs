// Regression test for the OCCT/GTK linking hazard: cadrum's prebuilt OCCT used
// to bundle a static libstdc++ that collided with the system one loaded by
// GTK/WebKit, segfaulting this binary at load. Parsing a STEP file inside the
// fully-linked Tauri test binary proves the runtime linking stays sound.
use std::path::PathBuf;

#[test]
fn step_parses_inside_gtk_linked_binary() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../libmeshthumbnail/tests/fixtures/cube.step");
    let mesh = libmeshthumbnail::parse_model::handle_parse(&fixture)
        .expect("cube.step should parse")
        .expect("step file should be recognized");
    assert!(!mesh.vertices.is_empty());
    assert!(!mesh.indices.is_empty());
}
