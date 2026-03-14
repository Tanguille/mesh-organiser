use std::path::{Path, PathBuf};

use async_zip::tokio::read::seek::ZipFileReader;
use chrono::Utc;
use indexmap::IndexMap;
use itertools::join;
use regex::Regex;
use serde::{Deserialize, Serialize};
use stl_io::Vector;
use tokio::{fs::File, io::BufReader};

use db::model::{Model, user::User};

use crate::{
    AppState, ServiceError, cleanse_evil_from_name, export_service, import_service,
    import_state::ImportState,
};

#[derive(Deserialize)]
pub struct ProjectSettingsConfig {
    pub nozzle_diameter: Vec<String>,
    pub layer_height: String,
    pub filament_type: Vec<String>,
    pub enable_support: String,
}

#[derive(Serialize)]
pub struct ThreemfMetadata {
    pub nozzle_diameter: Option<f32>,
    pub layer_height: Option<f32>,
    pub material_type: Option<String>,
    pub supports_enabled: Option<bool>,
}

fn parse_project_settings_config(data: &str) -> Result<ThreemfMetadata, ServiceError> {
    let parsed_data = serde_json::from_str::<ProjectSettingsConfig>(data)?;

    let nozzle_diameter = parsed_data
        .nozzle_diameter
        .first()
        .and_then(|s| s.parse::<f32>().ok());

    let layer_height = parsed_data.layer_height.parse::<f32>().ok();

    let filament_type = join(parsed_data.filament_type, ", ");

    let enable_support = match parsed_data.enable_support.as_str() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    };

    Ok(ThreemfMetadata {
        nozzle_diameter,
        layer_height,
        material_type: Some(filament_type),
        supports_enabled: enable_support,
    })
}

fn parse_slicer_pe_config(data: &str) -> ThreemfMetadata {
    let mut threemf = ThreemfMetadata {
        nozzle_diameter: None,
        layer_height: None,
        material_type: None,
        supports_enabled: None,
    };

    if let Some(line) = data
        .lines()
        .find(|line| line.starts_with("; nozzle_diameter = "))
    {
        let value = line.trim_start_matches("; nozzle_diameter = ").trim();
        let nozzle_diameters = value.split(',').collect::<Vec<&str>>();

        threemf.nozzle_diameter = nozzle_diameters.first().and_then(|s| s.parse::<f32>().ok());
    }

    if let Some(line) = data
        .lines()
        .find(|line| line.starts_with("; layer_height = "))
    {
        let value = line.trim_start_matches("; layer_height = ").trim();

        threemf.layer_height = value.parse::<f32>().ok();
    }

    if let Some(line) = data
        .lines()
        .find(|line| line.starts_with("; filament_type = "))
    {
        let value = line.trim_start_matches("; filament_type = ").trim();
        threemf.material_type = Some(value.replace(';', ", "));
    }

    if let Some(line) = data
        .lines()
        .find(|line| line.starts_with("; support_material ="))
    {
        let value = line.trim_start_matches("; support_material =").trim();
        threemf.supports_enabled = match value {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
    }

    threemf
}

/// Parses 3MF model settings config XML into a map of part id -> name.
///
/// # Panics
///
/// Panics if the regex for part/metadata parsing is invalid.
#[must_use]
pub fn parse_model_settings_config(data: &str) -> IndexMap<u32, String> {
    let mut names_map = IndexMap::new();

    let re = Regex::new(
        r#"(?s)<part\s+id="(\d+)"[^>]*>.*?<metadata\s+key="name"\s+value="([^"]+)"\s*/>"#,
    )
    .unwrap();

    for captures in re.captures_iter(data) {
        let part_id = captures[1].parse::<u32>().unwrap_or(u32::MAX);
        let part_name = captures[2].to_string();
        names_map.insert(part_id, part_name);
    }

    names_map
}

/// Reads a zip entry whose filename ends with `filename_suffix` and returns its contents as UTF-8.
///
/// # Errors
///
/// Returns an error if the file cannot be opened, no matching entry exists, or reading fails.
async fn read_zip_entry_by_suffix(
    path: PathBuf,
    filename_suffix: &str,
) -> Result<String, ServiceError> {
    let zip_file = File::open(path).await?;
    let mut buffered_reader = BufReader::new(zip_file);
    let mut zip = ZipFileReader::with_tokio(&mut buffered_reader).await?;

    let entries: Vec<_> = zip.file().entries().to_vec();

    for (i, entry) in entries.iter().enumerate() {
        if entry
            .filename()
            .as_str()
            .ok()
            .is_some_and(|s| s.ends_with(filename_suffix))
        {
            let mut file = zip.reader_with_entry(i).await?;
            let mut contents = String::new();
            file.read_to_string_checked(&mut contents).await?;
            return Ok(contents);
        }
    }

    Err(ServiceError::InternalError(
        "No zip entry matching suffix".to_string(),
    ))
}

/// Reads `model_settings.config` from a 3MF zip.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or the config is missing.
pub async fn fetch_model_settings_config_from_3mf(
    threemf_path: PathBuf,
) -> Result<IndexMap<u32, String>, ServiceError> {
    let contents = read_zip_entry_by_suffix(threemf_path, "model_settings.config").await?;
    Ok(parse_model_settings_config(&contents))
}

/// Extracts metadata from a 3MF model (project/slicer config).
///
/// # Errors
///
/// Returns an error if the model is not 3MF or extraction fails.
pub async fn extract_metadata(
    model: &Model,
    app_state: &AppState,
) -> Result<ThreemfMetadata, ServiceError> {
    if !model.blob.to_file_type().is_3mf() {
        return Err(ServiceError::InternalError(
            "Model is not a 3MF file".to_string(),
        ));
    }

    let temp_dir = std::env::temp_dir().join("meshorganiser_metadata_action");
    if !temp_dir.exists() {
        std::fs::create_dir(&temp_dir)?;
    }

    let theemf_path =
        export_service::get_path_from_model(&temp_dir, model, app_state, true).await?;

    let c1 = read_zip_entry_by_suffix(theemf_path.clone(), "project_settings.config").await;
    if let Ok(contents) = c1 {
        return parse_project_settings_config(&contents);
    }

    let c2 = read_zip_entry_by_suffix(theemf_path, "Slic3r_PE.config").await;
    if let Ok(contents) = c2 {
        return Ok(parse_slicer_pe_config(&contents));
    }

    Err(ServiceError::InternalError(
        "Failed to extract metadata".to_string(),
    ))
}

fn extract_models_inner(
    theemf_path: &Path,
    temp_dir: &Path,
    names_map: &IndexMap<u32, String>,
) -> Result<(), ServiceError> {
    let handle = std::fs::File::open(theemf_path)?;
    let threemf_model = threemf::read(handle)?;

    let objects: Vec<_> = threemf_model
        .iter()
        .flat_map(|model| &model.resources.object)
        .filter(|obj| obj.mesh.is_some())
        .collect();

    for obj in objects {
        let mesh = obj.mesh.as_ref().unwrap();

        let obj_id = u32::try_from(obj.id).unwrap_or(0);
        let obj_name = names_map.get(&obj_id).cloned().unwrap_or_else(|| {
            obj.name.as_ref().map_or_else(
                || format!("object_{}", obj.id),
                |s| cleanse_evil_from_name(s),
            )
        });

        let stl_path = temp_dir.join(format!("{obj_name}.stl"));

        let mut triangles = Vec::new();

        for triangle in &mesh.triangles.triangle {
            let v1 = &mesh.vertices.vertex[triangle.v1];
            let v2 = &mesh.vertices.vertex[triangle.v2];
            let v3 = &mesh.vertices.vertex[triangle.v3];

            // 3MF vertices are f64; stl_io expects f32. Mesh coordinates are bounded (print volume)
            // and f32 precision is sufficient; truncation is intentional.
            #[allow(clippy::cast_possible_truncation)]
            let vertex1 = [v1.x as f32, v1.y as f32, v1.z as f32];
            #[allow(clippy::cast_possible_truncation)]
            let vertex2 = [v2.x as f32, v2.y as f32, v2.z as f32];
            #[allow(clippy::cast_possible_truncation)]
            let vertex3 = [v3.x as f32, v3.y as f32, v3.z as f32];

            let edge1 = [
                vertex2[0] - vertex1[0],
                vertex2[1] - vertex1[1],
                vertex2[2] - vertex1[2],
            ];
            let edge2 = [
                vertex3[0] - vertex1[0],
                vertex3[1] - vertex1[1],
                vertex3[2] - vertex1[2],
            ];
            let normal = [
                edge1[1].mul_add(edge2[2], -(edge1[2] * edge2[1])),
                edge1[2].mul_add(edge2[0], -(edge1[0] * edge2[2])),
                edge1[0].mul_add(edge2[1], -(edge1[1] * edge2[0])),
            ];

            triangles.push(stl_io::Triangle {
                normal: Vector(normal),
                vertices: [Vector(vertex1), Vector(vertex2), Vector(vertex3)],
            });
        }

        let mut file = std::fs::File::create(stl_path)?;
        stl_io::write_stl(&mut file, triangles.iter())?;
    }

    Ok(())
}

/// Extracts 3MF objects as STL files into a temp dir and returns import state.
///
/// # Errors
///
/// Returns an error if the model is not 3MF or extraction fails.
///
/// # Panics
///
/// Panics if the system clock cannot provide nanosecond timestamps for the temp dir.
pub async fn extract_models(
    model: &Model,
    user: &User,
    app_state: &AppState,
) -> Result<ImportState, ServiceError> {
    if !model.blob.to_file_type().is_3mf() {
        return Err(ServiceError::InternalError(
            "Model is not a 3MF file".to_string(),
        ));
    }

    let mut temp_dir = std::env::temp_dir().join(format!(
        "meshorganiser_extract_action_{}",
        Utc::now().timestamp_nanos_opt().unwrap()
    ));
    std::fs::create_dir(&temp_dir)?;

    let theemf_path =
        export_service::get_path_from_model(&temp_dir, model, app_state, true).await?;

    let safe_model_name = cleanse_evil_from_name(&model.name);
    temp_dir.push(safe_model_name);

    std::fs::create_dir(&temp_dir)?;

    {
        let temp_dir = temp_dir.clone();
        let names_map = fetch_model_settings_config_from_3mf(theemf_path.clone())
            .await
            .unwrap_or_default();

        tokio::task::spawn_blocking(move || {
            extract_models_inner(&theemf_path, &temp_dir, &names_map)
        })
        .await??;
    }

    let import_state = ImportState::new(model.link.clone(), false, true, false, user.clone());
    let result =
        import_service::import_path(temp_dir.to_str().unwrap(), app_state, import_state).await?;

    Ok(result)
}

// -----------------------------------------------------------------------------
// Regression tests: lock in parse_model_settings_config (regex, part id/name,
// unwrap_or(u32::MAX)) after clippy refactors.
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::Write;

    use indexmap::IndexMap;
    use zip::ZipWriter;
    use zip::write::FileOptions;

    use super::{fetch_model_settings_config_from_3mf, parse_model_settings_config};

    #[test]
    fn parse_model_settings_config_empty_returns_empty_map() {
        let out = parse_model_settings_config("");
        assert!(out.is_empty());
    }

    #[test]
    fn parse_model_settings_config_extracts_part_id_and_name() {
        let xml = r#"
        <part id="1" something="other">
        <metadata key="name" value="Part One" />
        </part>
        <part id="2">
        <metadata key="name" value="Part Two" />
        </part>
        "#;
        let out = parse_model_settings_config(xml);
        let expected: IndexMap<u32, String> =
            [(1, "Part One".to_string()), (2, "Part Two".to_string())]
                .into_iter()
                .collect();
        assert_eq!(out, expected);
    }

    #[test]
    fn parse_model_settings_config_part_id_overflow_uses_max() {
        // Regex captures id as \d+; parse::<u32>() fails for overflow, unwrap_or(u32::MAX)
        let xml = r#"<part id="99999999999"><metadata key="name" value="X" /></part>"#;
        let out = parse_model_settings_config(xml);
        assert_eq!(out.get(&u32::MAX), Some(&"X".to_string()));
    }

    // -------------------------------------------------------------------------
    // Zip-reading behaviour for fetch_model_settings_config_from_3mf (DRY
    // refactor guard: read_zip_entry_by_suffix + callers).
    // -------------------------------------------------------------------------

    fn write_zip_with_entries(
        path: &std::path::Path,
        entries: &[(&str, &[u8])],
    ) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let mut zip = ZipWriter::new(file);
        let options: FileOptions<()> = FileOptions::default();
        for (name, data) in entries {
            zip.start_file(*name, options)?;
            zip.write_all(data)?;
        }
        zip.finish()?;
        Ok(())
    }

    #[tokio::test]
    async fn fetch_model_settings_config_from_3mf_returns_parsed_map_when_entry_present() {
        let xml = r#"<part id="42"><metadata key="name" value="My Part" /></part>"#;
        let _tmp = tempfile::NamedTempFile::new().expect("temp file");
        let path = _tmp.path().to_path_buf();
        write_zip_with_entries(&path, &[("Metadata/model_settings.config", xml.as_bytes())])
            .expect("write zip");

        let result = fetch_model_settings_config_from_3mf(path).await;
        let map = result.expect("fetch should succeed");
        let expected: IndexMap<u32, String> = [(42, "My Part".to_string())].into_iter().collect();
        assert_eq!(map, expected);
    }

    #[tokio::test]
    async fn fetch_model_settings_config_from_3mf_returns_err_when_no_matching_entry() {
        let _tmp = tempfile::NamedTempFile::new().expect("temp file");
        let path = _tmp.path().to_path_buf();
        write_zip_with_entries(&path, &[("other.txt", b"data")]).expect("write zip");

        let result = fetch_model_settings_config_from_3mf(path).await;
        assert!(result.is_err());
    }
}
