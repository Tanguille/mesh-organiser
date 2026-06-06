use std::{env, path::Path};

use libmeshthumbnail::parse_model::convert_step_path_to_stl;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let step_path = env::args().nth(1).ok_or(
        "usage: cargo run -p libmeshthumbnail --example step_probe --features step -- <file.step>",
    )?;
    let stl = convert_step_path_to_stl(Path::new(&step_path))?;

    println!("converted {} to {} STL bytes", step_path, stl.len());

    Ok(())
}
