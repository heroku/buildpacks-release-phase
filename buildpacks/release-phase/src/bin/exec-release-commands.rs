// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::{
    env,
    path::Path,
    process::{Command, Stdio},
};

use release_commands::read_commands_config;

fn main() {
    let args: Vec<String> = env::args().collect();
    let commands_toml_path = if let Some(p) = args.get(1) {
        Path::new(p)
    } else {
        eprintln!("release-phase failed: exec command requires argument, the path to release-commands.toml");
        std::process::exit(1);
    };
    match exec_release_sequence(commands_toml_path) {
        Ok(()) => {
            eprintln!("release-phase complete.");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("release-phase failed: {error}");
            std::process::exit(1);
        }
    }
}

fn exec_release_sequence(commands_toml_path: &Path) -> Result<(), release_commands::Error> {
    let config = read_commands_config(commands_toml_path)?;
    eprintln!("release-phase plan, {config}");

    if let Some(release_build_config) = config.release_build {
        eprintln!("release-phase executing release-build command: {release_build_config}");
        let mut cmd = Command::new(release_build_config.command);
        if let Some(args) = release_build_config.args {
            cmd.args(args);
        }
        if let Ok(path) = env::var("PATH") {
            cmd.env("PATH", path);
        }
        let status = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .status()
            .map_err(release_commands::Error::ReleaseCommandExecError)?;

        match status.code() {
            None => (),
            Some(0) => (),
            Some(code) => {
                return Err(release_commands::Error::ReleaseCommandExitedError(format!(
                    "command exited with status code {}",
                    code
                )))
            }
        }
    };

    if let Some(release_config) = config.release {
        for config in &release_config {
            eprintln!("release-phase executing release command: {config}");
            let mut cmd = Command::new(&config.command);
            if let Some(args) = &config.args {
                cmd.args(args.clone());
            }

            let status = cmd
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .status()
                .map_err(release_commands::Error::ReleaseCommandExecError)?;

            match status.code() {
                None => (),
                Some(0) => (),
                Some(code) => {
                    return Err(release_commands::Error::ReleaseCommandExitedError(format!(
                        "command exited with status code {}",
                        code
                    )))
                }
            }
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
