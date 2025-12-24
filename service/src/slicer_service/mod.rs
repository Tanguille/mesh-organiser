mod base;
use std::path::PathBuf;

pub use base::*;
use std::process::Command;

use crate::service_error::ServiceError;

use super::app_state::AppState;

#[cfg(target_os = "windows")]
mod win;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

pub fn open_with_paths(program: &str, paths: Vec<PathBuf>) -> Result<(), ServiceError> {
    if paths.is_empty() {
        return Err(ServiceError::InternalError(String::from(
            "No models to open",
        )));
    }

    Command::new(program).args(paths).spawn()?;

    Ok(())
}

pub async fn open_custom_slicer(
    paths: Vec<PathBuf>,
    app_state: &AppState,
) -> Result<(), ServiceError> {
    let path = app_state.get_configuration().custom_slicer_path;
    if path.is_empty() {
        return Err(ServiceError::InternalError(String::from(
            "Custom slicer path not set",
        )));
    }

    // Parse the command string to separate executable from arguments
    let (executable_path, args) = parse_command_string(&path);
    let pathbuf = PathBuf::from(&executable_path);

    if !pathbuf.exists() {
        return Err(ServiceError::InternalError(format!(
            "Custom slicer executable not found: {executable_path}",
        )));
    }

    println!("Opening in slicer: {paths:?}");
    println!("Executable: {executable_path}, Custom args: {args:?}");

    open_with_args_and_paths(&executable_path, args.as_slice(), paths)
}

/// Open with custom arguments and model paths
fn open_with_args_and_paths(
    program: &str,
    args: &[String],
    paths: Vec<PathBuf>,
) -> Result<(), ServiceError> {
    if paths.is_empty() {
        return Err(ServiceError::InternalError(String::from(
            "No models to open",
        )));
    }

    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.args(paths);
    cmd.spawn()?;

    Ok(())
}

/// Parse a command string into executable path and arguments.
/// Handles quoted strings properly, e.g., `"C:\Program Files\app.exe" -arg "value"`
/// will be parsed as executable: `C:\Program Files\app.exe`, args: `["-arg", "value"]`
/// Also handles unquoted executables with spaces by assuming arguments start with '-' or '/'
fn parse_command_string(cmd: &str) -> (String, Vec<String>) {
    let cmd = cmd.trim();
    if cmd.is_empty() {
        return (String::new(), Vec::new());
    }

    let mut chars = cmd.chars().peekable();
    let mut args = Vec::new();
    let mut current_arg = String::new();
    let mut in_quotes = false;

    // Parse the entire command line into arguments using shell-like quoting rules
    while let Some(char) = chars.next() {
        match char {
            '"' => {
                in_quotes = !in_quotes;
                // Don't include quotes in the parsed value
            }
            ' ' | '\t' if !in_quotes => {
                if !current_arg.is_empty() {
                    args.push(current_arg.clone());
                    current_arg.clear();
                }
                // Skip consecutive whitespace
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

    // Handle the last argument
    if !current_arg.is_empty() {
        args.push(current_arg);
    }

    if args.is_empty() {
        return (String::new(), Vec::new());
    }

    // Now determine where the executable ends and arguments begin
    // Look for the first argument that starts with '-' or '/'
    let mut first_flag_index = None;
    for (i, arg) in args.iter().enumerate() {
        if arg.starts_with('-') || arg.starts_with('/') {
            first_flag_index = Some(i);
            break;
        }
    }

    match first_flag_index {
        Some(flag_index) => {
            // Everything before the first flag is the executable
            let executable = args[..flag_index].join(" ");
            let args = args[flag_index..].to_vec();
            (executable, args)
        }
        None => {
            // No flag-like arguments found, assume first token is executable, rest are arguments
            if args.is_empty() {
                (String::new(), Vec::new())
            } else if args.len() == 1 {
                (args[0].clone(), Vec::new())
            } else {
                (args[0].clone(), args[1..].to_vec())
            }
        }
    }
}
