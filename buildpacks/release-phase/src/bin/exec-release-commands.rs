// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::{env, path::Path, process::Command};

use release_phase_utils::read_commands_config;

fn main() {
    let args: Vec<String> = env::args().collect();
    let commands_toml_path = Path::new(&args[1]);
    match exec_release_sequence(commands_toml_path) {
        Ok(()) => {
            eprintln!("release-phase command executor complete.");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("release-phase command executor failed: {error:#?}");
            std::process::exit(1);
        }
    }
}

fn exec_release_sequence(commands_toml_path: &Path) -> Result<(), release_phase_utils::Error> {
    let config = read_commands_config(commands_toml_path)?;
    eprintln!("release-phase command executor plan: {config:#?}");

    if let Some(release_build_config) = config.release_build {
        eprintln!("release-phase executing release-build command: {release_build_config:#?}");
        let mut cmd = Command::new(release_build_config.command);
        release_build_config.args.clone().map(|v| cmd.args(v));
        let output = cmd
            .output()
            .map_err(release_phase_utils::Error::ReleaseCommandExecError)?;
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
        print!("{}", String::from_utf8_lossy(&output.stdout));
    };

    if let Some(release_config) = config.release {
        for config in &release_config {
            eprintln!("release-phase executing release command: {config:#?}");
            let mut cmd = Command::new(&config.command);
            config.args.clone().map(|v| cmd.args(v));
            let output = cmd
                .output()
                .map_err(release_phase_utils::Error::ReleaseCommandExecError)?;
            print!("{}", String::from_utf8_lossy(&output.stdout));
            eprint!("{}", String::from_utf8_lossy(&output.stderr));
        }
    };

    Ok(())
}
