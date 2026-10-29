use std::{ffi::OsStr, future::Future, panic, path::Path, sync::LazyLock};

use db::model::blob::FileType;
use regex::Regex;
use tokio::task::JoinSet;

static COLLAPSE_WHITESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r" {2,}").expect("valid regex literal"));

/// Returns a human-friendly file name (no path); strips extension if not a directory.
#[must_use]
pub fn prettify_file_name(file: &Path, is_dir: bool) -> String {
    let extension = file.extension();
    let mut file_name: String = String::from(
        file.file_name()
            .unwrap_or_else(|| OsStr::new("unknown_filename"))
            .to_str()
            .unwrap(),
    );

    if !is_dir && let Some(ext) = extension {
        file_name = String::from(&file_name[0..file_name.len() - ext.len() - 1]);
    }

    file_name = file_name.replace(['_', '-', '+'], " ");

    file_name = String::from(COLLAPSE_WHITESPACE.replace_all(&file_name, " "));

    file_name = String::from(file_name.trim());

    file_name
}

/// Percent-decodes `text` into an owned string; fails if the decoded bytes are not valid UTF-8.
/// A literal `+` is kept as-is and a malformed `%zz` sequence is left untouched.
pub fn percent_decode(text: &str) -> Result<String, std::str::Utf8Error> {
    percent_encoding::percent_decode_str(text)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
}

#[must_use]
pub fn cleanse_evil_from_name(name: &str) -> String {
    let cleansed = name
        .replace(
            |c: char| {
                c.is_control() || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            },
            " ",
        )
        .trim()
        .to_string();

    // "." or ".." contain no separator to strip above, but `Path::join` still
    // treats them as directory traversal (escaping into the parent dir); substitute
    // a placeholder here so every caller gets a safe, usable name for free.
    if !cleansed.is_empty() && cleansed.chars().all(|c| c == '.') {
        return String::from("unnamed");
    }

    cleansed
}

/// The stored type of an imported extension.
///
/// Import zips exactly what `FileType::is_zippable` names, so those get a `.zip` suffix that
/// readers recognise as zipped. Re-import dedupe leaves legacy stored .stp blobs unchanged; see docs/commands.md.
#[must_use]
pub fn convert_extension_to_zip(extension: &str) -> String {
    let lowercase = extension.to_lowercase();

    if FileType::from_extension(&lowercase).is_zippable() {
        format!("{lowercase}.zip")
    } else {
        lowercase
    }
}

#[must_use]
pub fn convert_zip_to_extension(extension: &str) -> String {
    let lowercase = extension.to_lowercase();

    match lowercase.strip_suffix(".zip") {
        Some(plain) if FileType::from_extension(&lowercase).is_zipped() => plain.to_string(),
        _ => lowercase,
    }
}

/// Runs one future per item with bounded parallelism, returning every task output in completion order.
///
/// Spawns the future produced by `make_future` for each item, keeping at most `max` tasks in
/// flight (throttling by awaiting a completed task once the in-flight count reaches `max`).
/// Outputs are collected in the order tasks complete, matching `JoinSet::join_all`.
/// A panicking task resumes its unwind here, as `JoinSet` does.
pub async fn run_bounded<Item, Fut, F, T>(
    mut items: Vec<Item>,
    max: usize,
    mut make_future: F,
) -> Vec<T>
where
    F: FnMut(Item) -> Fut,
    Fut: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let mut futures: JoinSet<T> = JoinSet::new();
    let mut results: Vec<T> = Vec::new();

    while let Some(item) = items.pop() {
        futures.spawn(make_future(item));

        if futures.len() >= max
            && let Some(res) = futures.join_next().await
        {
            match res {
                Err(err) if err.is_panic() => panic::resume_unwind(err.into_panic()),
                Err(err) => panic!("{err}"),
                Ok(output) => {
                    results.push(output);
                }
            }
        }
    }

    results.extend(futures.join_all().await);

    results
}

#[cfg(test)]
mod run_bounded_tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::run_bounded;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn returns_every_task_output() {
        let items: Vec<usize> = (0..20).collect();

        let mut results = run_bounded(items.clone(), 4, |item| async move { item * 2 }).await;

        // Completion order is nondeterministic, so compare as a sorted multiset.
        results.sort_unstable();
        let mut expected: Vec<usize> = items.into_iter().map(|item| item * 2).collect();
        expected.sort_unstable();

        assert_eq!(results, expected);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn empty_input_returns_empty_vec() {
        let results = run_bounded(Vec::<usize>::new(), 4, |item| async move { item }).await;

        assert_eq!(results.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn never_exceeds_max_in_flight() {
        let max = 3;
        let in_flight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let items: Vec<usize> = (0..30).collect();

        run_bounded(items, max, |item| {
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);

            async move {
                let current = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);

                // Yield repeatedly so other tasks can be scheduled while this one is "in flight".
                for _ in 0..8 {
                    tokio::task::yield_now().await;
                }

                in_flight.fetch_sub(1, Ordering::SeqCst);

                item
            }
        })
        .await;

        assert!(
            peak.load(Ordering::SeqCst) <= max,
            "peak in-flight {} exceeded max {max}",
            peak.load(Ordering::SeqCst)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[should_panic(expected = "task boom")]
    async fn propagates_task_panic() {
        let items: Vec<usize> = (0..4).collect();

        // A panicking task must surface as a panic via panic::resume_unwind.
        run_bounded(items, 2, |_item| async move { panic!("task boom") }).await;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proptest::prelude::*;

    use super::{
        cleanse_evil_from_name, convert_extension_to_zip, convert_zip_to_extension,
        prettify_file_name,
    };

    #[test]
    fn percent_decode_decodes_multibyte_and_space() {
        assert_eq!(
            super::percent_decode("caf%C3%A9%20bar").unwrap(),
            "café bar"
        );
    }

    #[test]
    fn percent_decode_keeps_plus_literal() {
        assert_eq!(super::percent_decode("a+b").unwrap(), "a+b");
    }

    #[test]
    fn percent_decode_leaves_malformed_escape_untouched() {
        assert_eq!(super::percent_decode("100%zz").unwrap(), "100%zz");
    }

    #[test]
    fn percent_decode_rejects_invalid_utf8() {
        assert!(super::percent_decode("%FF").is_err());
    }

    #[test]
    fn test_cleanse_evil_empty() {
        assert_eq!(cleanse_evil_from_name(""), "");
    }

    #[test]
    fn test_cleanse_evil_normal_unchanged() {
        assert_eq!(cleanse_evil_from_name("hello world"), "hello world");
    }

    #[test]
    fn test_cleanse_evil_removes_backslash() {
        assert_eq!(cleanse_evil_from_name("a\\b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_slash() {
        assert_eq!(cleanse_evil_from_name("a/b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_colon() {
        assert_eq!(cleanse_evil_from_name("a:b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_asterisk() {
        assert_eq!(cleanse_evil_from_name("a*b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_question_mark() {
        assert_eq!(cleanse_evil_from_name("a?b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_double_quote() {
        assert_eq!(cleanse_evil_from_name("a\"b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_less_than() {
        assert_eq!(cleanse_evil_from_name("a<b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_greater_than() {
        assert_eq!(cleanse_evil_from_name("a>b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_removes_pipe() {
        assert_eq!(cleanse_evil_from_name("a|b"), "a b");
    }

    #[test]
    fn test_cleanse_evil_trim_outer_whitespace() {
        assert_eq!(cleanse_evil_from_name("  x  "), "x");
    }

    #[test]
    fn test_cleanse_evil_rejects_dot_only_names() {
        // No separators to strip, but ".." still escapes `Path::join` into the parent dir.
        assert_eq!(cleanse_evil_from_name(".."), "unnamed");
        assert_eq!(cleanse_evil_from_name("."), "unnamed");
        assert_eq!(cleanse_evil_from_name("..."), "unnamed");
    }

    /// Model names are user-editable and end up in `Content-Disposition` headers, where the web
    /// server's `HeaderValue` parse panics on control characters, so they go like the others.
    #[test]
    fn test_cleanse_evil_replaces_control_characters() {
        for name in ["a\nb", "a\tb", "a\u{7f}b", "a\u{0}b", "a\u{85}b"] {
            let cleansed = cleanse_evil_from_name(name);
            assert!(
                !cleansed.chars().any(char::is_control),
                "{name:?} became {cleansed:?}"
            );
            assert_eq!(cleansed, "a b", "{name:?}");
            assert!(
                format!("attachment; filename=\"{cleansed}\"")
                    .parse::<reqwest::header::HeaderValue>()
                    .is_ok(),
                "{name:?} became {cleansed:?}, which is not a valid header value"
            );
        }
    }

    #[test]
    fn test_prettify_file_name_path_with_extension_is_file() {
        let path = PathBuf::from("some_dir/My_Cool-Model.stl");
        assert_eq!(prettify_file_name(&path, false), "My Cool Model");
    }

    #[test]
    fn test_prettify_file_name_path_with_extension_is_dir() {
        let path = PathBuf::from("some_dir/My_Folder.stl");
        assert_eq!(prettify_file_name(&path, true), "My Folder.stl");
    }

    #[test]
    fn test_prettify_file_name_no_extension() {
        let path = PathBuf::from("myfile");
        assert_eq!(prettify_file_name(&path, false), "myfile");
    }

    #[test]
    fn test_prettify_file_name_replaces_underscore_and_collapses_spaces() {
        let path = PathBuf::from("a__b___c.stl");
        assert_eq!(prettify_file_name(&path, false), "a b c");
    }

    #[test]
    fn test_prettify_file_name_separators_collapse_and_trim() {
        let cases = [
            ("dir/Gear - v2 + spare.stl", false, "Gear v2 spare"),
            (
                "_-+Leading_and_trailing+-_.obj",
                false,
                "Leading and trailing",
            ),
            ("Single  Spaced   Name.3mf", false, "Single Spaced Name"),
            ("v1.2_final.stl", false, "v1.2 final"),
            ("Café_Modèl.stl", false, "Café Modèl"),
            ("parent/My__Folder-Name", true, "My Folder Name"),
            ("archive.stl.zip", false, "archive.stl"),
        ];

        for (path, is_dir, expected) in cases {
            assert_eq!(
                prettify_file_name(&PathBuf::from(path), is_dir),
                expected,
                "prettify_file_name({path:?}, {is_dir})"
            );
        }
    }

    #[test]
    fn test_convert_extension_to_zip_stl() {
        assert_eq!(convert_extension_to_zip("stl"), "stl.zip");
    }

    #[test]
    fn test_convert_extension_to_zip_obj_step_gcode() {
        assert_eq!(convert_extension_to_zip("obj"), "obj.zip");
        assert_eq!(convert_extension_to_zip("step"), "step.zip");
        assert_eq!(convert_extension_to_zip("gcode"), "gcode.zip");
    }

    #[test]
    fn test_convert_extension_to_zip_unknown_returns_lowercase() {
        assert_eq!(convert_extension_to_zip("TXT"), "txt");
        assert_eq!(convert_extension_to_zip("unknown"), "unknown");
    }

    #[test]
    fn test_convert_zip_to_extension_known() {
        assert_eq!(convert_zip_to_extension("stl.zip"), "stl");
        assert_eq!(convert_zip_to_extension("obj.zip"), "obj");
        assert_eq!(convert_zip_to_extension("step.zip"), "step");
        assert_eq!(convert_zip_to_extension("gcode.zip"), "gcode");
    }

    /// Import zips exactly the types `FileType::is_zippable` names and stores them under
    /// `convert_extension_to_zip`; readers unzip whatever `is_zipped` names. Both maps must agree,
    /// or a blob is stored zipped but served raw. `FileType` matches by suffix, so `xstl` is
    /// zippable too; types it does not zip must pass through both conversions lowercased.
    #[test]
    fn every_zippable_extension_is_stored_as_a_zipped_type() {
        for extension in [
            "stl", "STL", "obj", "step", "stp", "STP", "gcode", "xstl", "3mf", "png",
        ] {
            let lowercase = extension.to_lowercase();
            let stored = convert_extension_to_zip(extension);
            if db::model::blob::FileType::from_extension(extension).is_zippable() {
                assert!(
                    db::model::blob::FileType::from_extension(&stored).is_zipped(),
                    "{extension} is zipped on import but stored as {stored}, which readers do not unzip"
                );
                assert_eq!(convert_zip_to_extension(&stored), lowercase, "{extension}");
            } else {
                assert_eq!(stored, lowercase, "{extension}");
                assert_eq!(
                    convert_zip_to_extension(extension),
                    lowercase,
                    "{extension}"
                );
            }
        }
    }

    #[test]
    fn test_convert_zip_to_extension_unknown_returns_lowercase() {
        assert_eq!(convert_zip_to_extension("other.zip"), "other.zip");
        assert_eq!(convert_zip_to_extension("STL.ZIP"), "stl");
    }

    const EVIL_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

    proptest! {
        /// Round-trip: for known zippable extensions, convert_zip_to_extension(convert_extension_to_zip(ext)) equals canonical form.
        #[test]
        fn prop_convert_extension_round_trip_known(ext in prop_oneof![
            Just("stl"),
            Just("obj"),
            Just("gcode"),
            Just("step"),
            Just("stp"),
        ]) {
            let z = convert_extension_to_zip(ext);
            let back = convert_zip_to_extension(&z);
            prop_assert_eq!(back.as_str(), ext);
        }

        /// For any string, cleanse_evil output must not contain filesystem-unsafe characters.
        #[test]
        fn prop_cleanse_evil_removes_evil(s in "[ -~]{0,80}") {
            let out = cleanse_evil_from_name(&s);
            for c in EVIL_CHARS {
                prop_assert!(!out.contains(*c), "output must not contain {:?}", c);
            }
        }

        /// Idempotent: cleansing an already-clean string only trims.
        #[test]
        fn prop_cleanse_evil_idempotent_clean(s in "[a-zA-Z0-9 _-]{0,50}") {
            let trimmed = s.trim();
            let once = cleanse_evil_from_name(trimmed);
            let twice = cleanse_evil_from_name(&once);
            prop_assert_eq!(once, twice);
        }
    }
}
