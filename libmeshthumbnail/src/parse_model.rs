use std::{fs::File, io, path::Path};

use zip::ZipArchive;

use crate::{
    error::MeshThumbnailError,
    mesh::Mesh,
    path_ext::{is_zip_of, matches_ext},
};

mod gcode;
mod obj;
#[cfg(feature = "step")]
mod step;
mod stl;
mod threemf;

/// Opens `path` as a zip, finds the first entry whose name satisfies `matches`,
/// and hands its reported decompressed size plus reader to `read_entry`, returning
/// whatever `read_entry` produces.
///
/// Returns `InternalError(not_found)` when no entry matches.
pub(crate) fn with_zip_entry<R>(
    path: &Path,
    matches: impl Fn(&str) -> bool,
    not_found: &str,
    read_entry: impl FnOnce(u64, &mut dyn io::Read) -> Result<R, MeshThumbnailError>,
) -> Result<R, MeshThumbnailError> {
    let handle = File::open(path)?;
    let mut zip = ZipArchive::new(handle)?;

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if matches(file.name()?.as_ref()) {
            return read_entry(file.size(), &mut file);
        }
    }

    Err(MeshThumbnailError::InternalError(String::from(not_found)))
}

/// Buffered variant of [`with_zip_entry`] for the small mesh formats that are parsed
/// from memory, pre-sized to the entry's reported size.
pub(crate) fn find_zip_entry_bytes(
    path: &Path,
    matches: impl Fn(&str) -> bool,
    not_found: &str,
) -> Result<Vec<u8>, MeshThumbnailError> {
    with_zip_entry(path, matches, not_found, |size, reader| {
        let mut buffer = Vec::with_capacity(usize::try_from(size).unwrap_or(0));
        io::copy(reader, &mut buffer)?;

        Ok(buffer)
    })
}

/// Routes `path` to `parse_zip` when it is a `.<extension>.zip`, to `parse` when it is
/// a `.<extension>`, and returns `Ok(None)` for anything else.
pub(crate) fn parse_by_extension<T>(
    path: &Path,
    extension: &str,
    parse: fn(&Path) -> Result<T, MeshThumbnailError>,
    parse_zip: fn(&Path) -> Result<T, MeshThumbnailError>,
) -> Result<Option<T>, MeshThumbnailError> {
    if is_zip_of(path, extension) {
        parse_zip(path).map(Some)
    } else if matches_ext(path, extension) {
        parse(path).map(Some)
    } else {
        Ok(None)
    }
}

type MeshHandler = fn(&Path) -> Result<Option<Mesh>, MeshThumbnailError>;

/// Parses a mesh from the given path (STL, OBJ, 3MF, G-code, etc.). The first handler
/// that claims the path decides the result, including its error.
pub fn handle_parse(path: &Path) -> Result<Option<Mesh>, MeshThumbnailError> {
    let handlers: &[MeshHandler] = &[
        stl::handle_stl,
        obj::handle,
        threemf::handle_threemf,
        gcode::handle_gcode,
        #[cfg(feature = "step")]
        step::handle_step,
    ];

    for handler in handlers {
        if let Some(mesh) = handler(path)? {
            return Ok(Some(mesh));
        }
    }

    Ok(None)
}

#[cfg(test)]
// Test predicates use `ends_with(".stl")` purely to exercise the helper's
// match-and-read logic; the real per-format predicates (case-insensitive) live
// in the handler modules.
#[allow(clippy::case_sensitive_file_extension_comparisons)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use crate::error::MeshThumbnailError;

    use super::find_zip_entry_bytes;

    /// Writes a zip containing `entries` (name, bytes) to a temp file and returns it.
    fn make_zip(entries: &[(&str, &[u8])]) -> NamedTempFile {
        let temp = NamedTempFile::new().unwrap();
        let mut writer = ZipWriter::new(temp.reopen().unwrap());
        // Stored (no compression) keeps the round-trip purely about byte identity.
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap();

        temp
    }

    #[test]
    fn returns_matching_entry_bytes_verbatim() {
        let payload: &[u8] = b"hello \x00 world \xff bytes";
        let zip = make_zip(&[("ignored.txt", b"nope"), ("model.stl", payload)]);

        let bytes =
            find_zip_entry_bytes(zip.path(), |name| name.ends_with(".stl"), "missing").unwrap();

        assert_eq!(bytes, payload);
    }

    #[test]
    fn returns_first_matching_entry_when_several_match() {
        let zip = make_zip(&[("a.stl", b"first"), ("b.stl", b"second")]);

        let bytes =
            find_zip_entry_bytes(zip.path(), |name| name.ends_with(".stl"), "missing").unwrap();

        assert_eq!(bytes, b"first");
    }

    #[test]
    fn returns_internal_error_with_given_message_on_miss() {
        let zip = make_zip(&[("only.txt", b"data")]);

        let result = find_zip_entry_bytes(
            zip.path(),
            |name| name.ends_with(".stl"),
            "Failed to find .stl model in zip",
        );

        match result {
            Err(MeshThumbnailError::InternalError(message)) => {
                assert_eq!(message, "Failed to find .stl model in zip");
            }
            other => panic!("expected InternalError, got {other:?}"),
        }
    }
}
