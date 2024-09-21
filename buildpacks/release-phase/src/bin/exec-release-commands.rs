// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::{env, path::Path};

use release_phase_utils::read_commands_config;

fn main() {
    let args: Vec<String> = env::args().collect();
    let commands_toml_path = Path::new(&args[1]);
    match read_commands_config(commands_toml_path) {
        Ok(commands_config) => {
            eprintln!("Command executor configuration: {commands_config:#?}");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("Command executor (from release-phase buildpack) failed: {error:#?}");
            std::process::exit(1);
        }
    }
}
