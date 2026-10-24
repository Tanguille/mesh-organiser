use std::{path::PathBuf, process::Command};

use crate::service_error::ServiceError;

use super::app_state::AppState;

mod base;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod win;

pub use base::*;

impl Slicer {
    #[must_use]
    pub fn is_installed(&self) -> bool {
        matches!(self, Self::Custom) || self.detect_installed()
    }

    /// Opens the slicer application with the given paths. A missing or misconfigured
    /// slicer is reported before an empty `paths`.
    pub fn open(&self, paths: Vec<PathBuf>, app_state: &AppState) -> Result<(), ServiceError> {
        let custom_command = if matches!(self, Self::Custom) {
            let command = app_state.get_configuration().custom_slicer_path;
            if command.is_empty() {
                return Err(ServiceError::InternalError(String::from(
                    "Custom slicer path not set",
                )));
            }

            let (executable_path, args) = parse_command_string(&command);
            if !PathBuf::from(&executable_path).exists() {
                return Err(ServiceError::InternalError(format!(
                    "Custom slicer executable not found: {executable_path}",
                )));
            }

            println!("Executable: {executable_path}, Custom args: {args:?}");
            Some((executable_path, args))
        } else if self.is_installed() {
            None
        } else {
            return Err(ServiceError::InternalError(String::from(
                "Slicer not installed",
            )));
        };

        println!("Opening in slicer: {paths:?}");

        if paths.is_empty() {
            return Err(ServiceError::InternalError(String::from(
                "No models to open",
            )));
        }

        match custom_command {
            Some((executable_path, args)) => {
                open_with_args_and_paths(&executable_path, &args, paths)
            }
            None => self.spawn_with_paths(paths),
        }
    }
}

fn open_with_args_and_paths(
    program: &str,
    args: &[String],
    paths: Vec<PathBuf>,
) -> Result<(), ServiceError> {
    Command::new(program).args(args).args(paths).spawn()?;

    Ok(())
}

/// Parse a command string into executable path and arguments.
/// Handles quoted strings properly, e.g., `"C:\Program Files\app.exe" -arg "value"`
/// will be parsed as executable: `C:\Program Files\app.exe`, args: `["-arg", "value"]`
/// Also handles unquoted executables with spaces by assuming arguments start with '-' or '/'.
/// The first token is always part of the executable, so an absolute Unix path such as
/// `/usr/bin/orca-slicer` is not mistaken for a Windows-style `/flag`.
fn parse_command_string(cmd: &str) -> (String, Vec<String>) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return (String::default(), Vec::new());
    }

    let mut chars = cmd.chars().peekable();
    let mut args = Vec::new();
    let mut current_arg = String::default();
    let mut in_quotes = false;

    while let Some(char) = chars.next() {
        match char {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' | '\t' if !in_quotes => {
                if !current_arg.is_empty() {
                    args.push(std::mem::take(&mut current_arg));
                }
                while let Some(&next_char) = chars.peek() {
                    if next_char == ' ' || next_char == '\t' {
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            _ => {
                current_arg.push(char);
            }
        }
    }

    if !current_arg.is_empty() {
        args.push(current_arg);
    }

    if args.is_empty() {
        return (String::default(), Vec::new());
    }

    let first_flag_index = args
        .iter()
        .skip(1)
        .position(|arg| arg.starts_with('-') || arg.starts_with('/'))
        .map(|index| index + 1);

    if let Some(flag_index) = first_flag_index {
        let executable = args[..flag_index].join(" ");
        let rest = args.split_off(flag_index);

        (executable, rest)
    } else {
        let mut iter = args.into_iter();
        let executable = iter.next().unwrap_or_default();

        (executable, iter.collect())
    }
}

#[cfg(test)]
mod tests {
    fn owned(arguments: &[&str]) -> Vec<String> {
        arguments.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn an_absolute_unix_path_is_the_executable_not_a_slash_flag() {
        assert_eq!(
            super::parse_command_string("/usr/bin/orca"),
            (String::from("/usr/bin/orca"), Vec::new())
        );
    }

    #[test]
    fn an_absolute_unix_path_keeps_its_flags_as_arguments() {
        assert_eq!(
            super::parse_command_string("/opt/slicer/run.sh --flag value"),
            (
                String::from("/opt/slicer/run.sh"),
                owned(&["--flag", "value"])
            )
        );
    }

    #[test]
    fn unquoted_spaces_join_the_executable_until_the_first_flag() {
        assert_eq!(
            super::parse_command_string("/opt/My Slicer/bin/slicer -x"),
            (String::from("/opt/My Slicer/bin/slicer"), owned(&["-x"]))
        );
    }

    #[test]
    fn a_quoted_windows_path_keeps_its_spaces_and_drops_the_quotes() {
        assert_eq!(
            super::parse_command_string(r#""C:\Program Files\app.exe" -arg "value""#),
            (
                String::from(r"C:\Program Files\app.exe"),
                owned(&["-arg", "value"])
            )
        );
    }

    #[test]
    fn an_unquoted_windows_path_stops_at_a_slash_flag() {
        assert_eq!(
            super::parse_command_string(r"C:\Program Files\app.exe /S"),
            (String::from(r"C:\Program Files\app.exe"), owned(&["/S"]))
        );
    }

    #[test]
    fn a_relative_path_is_the_executable() {
        assert_eq!(
            super::parse_command_string("./.local/bin/orca-launcher"),
            (String::from("./.local/bin/orca-launcher"), Vec::new())
        );
    }

    #[test]
    fn empty_or_whitespace_input_has_no_executable() {
        for command in ["", "   ", " \t "] {
            assert_eq!(
                super::parse_command_string(command),
                (String::new(), Vec::new()),
                "{command:?}"
            );
        }
    }
}
