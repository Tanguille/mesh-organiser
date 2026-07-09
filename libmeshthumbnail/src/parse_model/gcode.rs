use std::{
    fs::File,
    io::{BufRead, BufReader, Cursor, Read},
    path::Path,
    sync::OnceLock,
};

use regex::Regex;
use vek::{Mat4, Quaternion, Vec3};

use crate::{
    error::MeshThumbnailError,
    mesh::Mesh,
    parse_model::find_zip_entry_bytes,
    path_ext::{is_zip_of, matches_ext},
};

static REGEX_XY: OnceLock<Regex> = OnceLock::new();
static REGEX_XY_NO_EXTRUSION: OnceLock<Regex> = OnceLock::new();
static REGEX_Z: OnceLock<Regex> = OnceLock::new();

pub fn handle_gcode(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    if matches_ext(path, "gcode") {
        Ok(Some(parse_gcode(path)?))
    } else if is_zip_of(path, "gcode") {
        Ok(Some(parse_gcode_zip(path)?))
    } else {
        Ok(None)
    }
}

struct Point {
    v: Vec3<f32>,
    use_line: bool,
}

fn parse_gcode(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let mut handle = File::open(path)?;

    parse_gcode_inner(&mut handle)
}

fn parse_gcode_zip(path: &Path) -> Result<Mesh, MeshThumbnailError> {
    let buffer = find_zip_entry_bytes(
        path,
        |name| matches_ext(Path::new(name), "gcode"),
        "Failed to find .gcode file in zip",
    )?;
    let mut cursor = Cursor::new(buffer);

    parse_gcode_inner(&mut cursor)
}
fn parse_gcode_inner<W>(reader: &mut W) -> Result<Mesh, MeshThumbnailError>
where
    W: Read,
{
    let reader = BufReader::new(reader);
    let mut entries = Vec::with_capacity(0x10000);
    let mut last_x = 0f32;
    let mut last_y = 0f32;
    let mut last_z = 0f32;
    let regex_xy = REGEX_XY.get_or_init(|| Regex::new(r"X([\d.]+)\s+Y([\d.]+)\s+E").unwrap());
    let regex_xy_no_extrusion =
        REGEX_XY_NO_EXTRUSION.get_or_init(|| Regex::new(r"X([\d.]+)\s+Y([\d.]+)").unwrap());
    let regex_z = REGEX_Z.get_or_init(|| Regex::new(r"Z([\d.]+)").unwrap());
    let mut position_unsafe = false;

    for line in reader.lines() {
        let line = line?;
        if line.starts_with("G1") || line.starts_with("G0") {
            if let Some(caps) = regex_z.captures(&line) {
                last_z = caps.get(1).unwrap().as_str().parse::<f32>()?;
            }

            if let Some(caps) = regex_xy.captures(&line) {
                if position_unsafe {
                    entries.push(Point {
                        v: Vec3::new(last_x, last_y, last_z),
                        use_line: false,
                    });
                    position_unsafe = false;
                }

                last_x = caps.get(1).unwrap().as_str().parse::<f32>()?;
                last_y = caps.get(2).unwrap().as_str().parse::<f32>()?;

                entries.push(Point {
                    v: Vec3::new(last_x, last_y, last_z),
                    use_line: true,
                });
            } else if let Some(caps) = regex_xy_no_extrusion.captures(&line) {
                last_x = caps.get(1).unwrap().as_str().parse::<f32>()?;
                last_y = caps.get(2).unwrap().as_str().parse::<f32>()?;
                position_unsafe = true;
            }
        }
    }

    if entries.len() <= 2 {
        return Err(MeshThumbnailError::InternalError(String::from(
            "Gcode file contains no move instructions",
        )));
    }

    let angle_subdivisions = 3;

    let estimated_entries = entries.iter().filter(|x| x.use_line).count();
    let mut vertices =
        Vec::with_capacity(estimated_entries * (angle_subdivisions as usize + 1) * 2);
    let mut indices = Vec::with_capacity(estimated_entries * angle_subdivisions as usize * 6);

    // The cylinder topology is identical for every edge, so build it once and
    // reuse it: each edge only needs the per-edge transform applied to the base
    // vertices, avoiding a fresh Mesh allocation per move instruction.
    let base_cylinder = cylinder(angle_subdivisions);

    for i in 0..entries.len() - 1 {
        if !entries[i + 1].use_line {
            continue;
        }

        // Skip degenerate edges (zero length or too small)
        let p1 = entries[i].v;
        let p2 = entries[i + 1].v;
        let length = (p1 - p2).magnitude();

        if length < 0.001 || !length.is_finite() {
            continue;
        }

        let transform = edge_transform(p1, p2, length);

        let vertex_offset = u32::try_from(vertices.len()).unwrap_or(0);

        vertices.extend(base_cylinder.vertices.iter().map(|vertex| {
            let v4 = transform * vertex.with_w(1.0);
            v4.xyz()
        }));
        indices.extend(base_cylinder.indices.iter().map(|i| *i + vertex_offset));
    }

    Ok(Mesh { vertices, indices })
}

// The caller guarantees a finite, non-degenerate `length` (it filters edges
// before calling), so `diff / length` is safe here.
fn edge_transform(p1: Vec3<f32>, p2: Vec3<f32>, length: f32) -> Mat4<f32> {
    let diff = p2 - p1;
    let direction = diff / length; // Manual normalization to avoid potential issues

    let x_axis = Vec3::<f32>::new(1.0, 0.0, 0.0);

    // Handle the case where direction is parallel to x_axis
    let rotation: Quaternion<f32> = if (direction - x_axis).magnitude() < 0.001 {
        Quaternion::identity()
    } else if (direction + x_axis).magnitude() < 0.001 {
        // 180 degree rotation around y-axis
        Quaternion::rotation_y(std::f32::consts::PI)
    } else {
        Quaternion::rotation_from_to_3d(x_axis, direction)
    };

    Mat4::<f32>::translation_3d(p1)
        * Mat4::<f32>::from(rotation)
        * Mat4::<f32>::scaling_3d(Vec3::<f32>::new(length, 1.2, 1.0))
}

fn cylinder(angle_subdivisions: u32) -> Mesh {
    let length_subdivisions = 1;
    let mut positions = Vec::new();
    let mut indices = Vec::new();

    let radius = 0.3;

    // Subdivision indices are small (length_subdivisions=1, angle_subdivisions=3); u32 fits in f32 mantissa with no loss.
    #[allow(clippy::cast_precision_loss)]
    for i in 0..=length_subdivisions {
        let length_ratio = i as f32 / length_subdivisions as f32;
        for j in 0..angle_subdivisions {
            let angle = 2.0 * std::f32::consts::PI * j as f32 / angle_subdivisions as f32;
            positions.push(Vec3::new(
                length_ratio,
                angle.cos() * radius,
                angle.sin() * radius,
            ));
        }
    }

    // Create side triangles
    for i in 0..length_subdivisions {
        for j in 0..angle_subdivisions {
            let next_j = (j + 1) % angle_subdivisions;

            let v0 = i * angle_subdivisions + j;
            let v1 = (i + 1) * angle_subdivisions + j;
            let v2 = i * angle_subdivisions + next_j;
            let v3 = (i + 1) * angle_subdivisions + next_j;

            // Two triangles per quad
            indices.push(v0);
            indices.push(v1);
            indices.push(v2);

            indices.push(v2);
            indices.push(v1);
            indices.push(v3);
        }
    }

    Mesh {
        vertices: positions,
        indices,
    }
}
