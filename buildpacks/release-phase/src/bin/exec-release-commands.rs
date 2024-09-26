// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::{env, path::Path, process::Command};

use release_phase_utils::read_commands_config;

fn main() {
    let args: Vec<String> = env::args().collect();
    let commands_toml_path = Path::new(&args[1]);
    match exec_release_sequence(commands_toml_path) {
        Ok(()) => {
            eprintln!("release-phase complete.");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("release-phase failed: {error:#?}");
            std::process::exit(1);
        }
    }
}

fn exec_release_sequence(commands_toml_path: &Path) -> Result<(), release_phase_utils::Error> {
    let config = read_commands_config(commands_toml_path)?;
    eprintln!("release-phase plan, {config}");

    if let Some(release_build_config) = config.release_build {
        eprintln!("release-phase executing release-build command: {release_build_config}");
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
            eprintln!("release-phase executing release command: {config}");
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

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, remove_file},
        path::Path,
    };

    use crate::exec_release_sequence;

    #[test]
    fn invokes_command_sequence() {
        let expected_output = r"1. Release Build from all release commands
2. Release from all release commands
3. Another release from all release commands
";

        exec_release_sequence(Path::new(
            "tests/fixtures/uses_all_release_commands/release-commands.toml",
        ))
        .expect("release commands completed");

        let result_path = Path::new(
            "tests/fixtures/uses_all_release_commands/exec-release-commands-test-output.txt",
        );
        let result_output = fs::read_to_string(result_path).unwrap();
        remove_file(result_path).expect("test result output file is deleted");
        assert_eq!(result_output, expected_output);
    }
}
