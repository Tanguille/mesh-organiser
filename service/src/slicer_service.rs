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
        matches!(self, Self::Custom) || self.find_installed().is_some()
    }

    /// Opens the slicer application with the given paths. A missing or misconfigured
    /// slicer is reported before an empty `paths`.
    pub fn open(&self, paths: Vec<PathBuf>, app_state: &AppState) -> Result<(), ServiceError> {
        let installed_slicer = self.find_installed();
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
        } else if installed_slicer.is_some() {
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
            None => Self::spawn_with_paths(installed_slicer.unwrap(), paths),
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
/// Also handles unquoted executables with spaces by assuming arguments start with '-' or '/'
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
        .position(|arg| arg.starts_with('-') || arg.starts_with('/'));

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
