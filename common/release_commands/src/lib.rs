use std::{
    fmt::{self, Debug},
    path::Path,
};

use libcnb::{read_toml_file, write_toml_file, TomlFileError};
use libherokubuildpack::toml::toml_select_value;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Eq, PartialEq, Debug, Default, Clone)]
pub struct ReleaseCommands {
    #[serde(rename = "release-build")]
    pub release_build: Option<Executable>,
    pub release: Option<Vec<Executable>>,
}

impl fmt::Display for ReleaseCommands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "commands:\n  release-build: {}\n  release: {}",
            self.release_build
                .clone()
                .map_or("None".to_string(), |r| format!("{r}")),
            self.release.clone().map_or("None".to_string(), |r| r
                .into_iter()
                .fold(String::new(), |r, e| format!("{r}\n    {e}"))),
        )
    }
}

#[derive(Deserialize, Serialize, Eq, PartialEq, Debug, Default, Clone)]
pub struct Executable {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub source: Option<String>,
}

impl fmt::Display for Executable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}{}",
            self.command,
            self.args
                .clone()
                .map_or(String::new(), |a| format!(" {}", a.join(" "))),
            self.source
                .clone()
                .map_or(String::new(), |s| format!(" ({s})")),
        )
    }
}

#[derive(Debug)]
pub enum Error {
    ReleaseCommandsMustBeArray,
    ReleaseBuildCommandMustBeTable,
    TomlProjectFileError(TomlFileError),
    TomlReleaseCommandsFileError(TomlFileError),
    TomlProjectDeserializeError(toml::de::Error),
    TomlReleaseCommandsDeserializeError(toml::de::Error),
    TomlWriteReleaseCommandsFileError(TomlFileError),
    ReleaseCommandExecError(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::ReleaseCommandsMustBeArray => write!(
                f,
                "Configuration of `release` must be an array of commands."
            ),
            Error::ReleaseBuildCommandMustBeTable => write!(
                f,
                "Configuration of `release-build` must be a single command."
            ),
            Error::TomlProjectFileError(error) => {
                write!(f, "Failure reading `project.toml`, {error:#?}")
            }
            Error::TomlReleaseCommandsFileError(error) => {
                write!(f, "Failure reading `release-commands.toml`, {error:#?}")
            }
            Error::TomlProjectDeserializeError(error) => {
                write!(f, "Configuration error in `project.toml`, {error:#?}")
            }
            Error::TomlReleaseCommandsDeserializeError(error) => {
                write!(
                    f,
                    "Configuration error in `release-commands.toml`, {error:#?}"
                )
            }
            Error::TomlWriteReleaseCommandsFileError(error) => {
                write!(f, "Failure writing `release-commands.toml`, {error:#?}")
            }
            Error::ReleaseCommandExecError(error) => {
                write!(f, "Command failed, {error:#?}")
            }
        }
    }
}

pub fn read_project_config(project_toml_path: &Path) -> Result<ReleaseCommands, Error> {
    let project_toml = if project_toml_path.is_file() {
        read_toml_file::<toml::Value>(project_toml_path).map_err(Error::TomlProjectFileError)?
    } else {
        toml::Table::new().into()
    };

    let mut commands_toml = toml::Table::new();
    if let Some(release_config) =
        toml_select_value(vec!["com", "heroku", "phase", "release"], &project_toml).cloned()
    {
        commands_toml.insert("release".to_string(), release_config);
    };
    if let Some(release_build_config) = toml_select_value(
        vec!["com", "heroku", "phase", "release-build"],
        &project_toml,
    )
    .cloned()
    {
        commands_toml.insert("release-build".to_string(), release_build_config);
    };

    let mut commands = commands_toml
        .try_into::<ReleaseCommands>()
        .map_err(Error::TomlProjectDeserializeError)?;

    if commands.release_build.is_some() {
        // Add the uploader as the first release command, immediately after release-build.
        let upload_helper = Executable {
            command: "upload-release-artifacts".to_string(),
            args: Some(vec!["static-artifacts/".to_string()]),
            source: Some("Heroku Release Phase Buildpack".to_string()),
        };
        commands.release =
            Some([vec![upload_helper], commands.release.map_or(vec![], |v| v)].concat());
    }

    Ok(commands)
}

pub fn read_commands_config(commands_toml_path: &Path) -> Result<ReleaseCommands, Error> {
    let commands_toml = if commands_toml_path.is_file() {
        read_toml_file::<toml::Value>(commands_toml_path)
            .map_err(Error::TomlReleaseCommandsFileError)?
    } else {
        toml::Table::new().into()
    };

    commands_toml
        .try_into::<ReleaseCommands>()
        .map_err(Error::TomlReleaseCommandsDeserializeError)
}

pub fn write_commands_config(dir: &Path, commands: &ReleaseCommands) -> Result<(), Error> {
    let commands_toml_path = dir.join("release-commands.toml");
    write_toml_file(&commands, commands_toml_path).map_err(Error::TomlWriteReleaseCommandsFileError)
}

#[cfg(test)]
mod tests {
    use std::env;
    use std::fs::remove_file;
    use std::path::PathBuf;

    use libcnb::read_toml_file;
    use libherokubuildpack::toml::toml_select_value;
    use toml::toml;

    use crate::read_commands_config;
    use crate::read_project_config;
    use crate::write_commands_config;
    use crate::Executable;
    use crate::ReleaseCommands;

    #[test]
    fn reads_project_toml_for_release_commands() {
        let project_config = read_project_config(
            PathBuf::from(
                "../../buildpacks/release-phase/tests/fixtures/project_uses_release/project.toml",
            )
            .as_path(),
        )
        .unwrap();
        assert_eq!(
            project_config.release,
            Some(vec![
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec![
                        "-c".to_string(),
                        "echo 'Hello from Release Phase Buildpack!'".to_string()
                    ]),
                    source: None,
                },
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec![
                        "-c".to_string(),
                        "echo 'Hello again from Release Phase Buildpack!'".to_string()
                    ]),
                    source: None,
                }
            ])
        );
        assert_eq!(project_config.release_build, None);
    }

    #[test]
    fn reads_project_toml_for_release_build_command() {
        let project_config = read_project_config(
            PathBuf::from(
                "../../buildpacks/release-phase/tests/fixtures/project_uses_release_build/project.toml",
            )
            .as_path(),
        )
        .unwrap();
        assert_eq!(
            project_config.release_build,
            Some(Executable {
                command: "bash".to_string(),
                args: Some(vec![
                    "-c".to_string(),
                    "echo 'Build in Release Phase Buildpack!'".to_string()
                ]),
                source: None,
            })
        );
        assert_eq!(
            project_config.release,
            Some(vec![Executable {
                command: "upload-release-artifacts".to_string(),
                args: Some(vec!["static-artifacts/".to_string()]),
                source: Some("Heroku Release Phase Buildpack".to_string()),
            }])
        );
    }

    #[test]
    fn no_project_toml() {
        let project_config = read_project_config(
            PathBuf::from(
                "../../buildpacks/release-phase/tests/fixtures/no_project_toml/project.toml",
            )
            .as_path(),
        )
        .unwrap();
        assert!(project_config.release.is_none());
        assert!(project_config.release_build.is_none());
    }

    #[test]
    fn reads_commands_toml_for_release_commands() {
        let commands_config = read_commands_config(
            PathBuf::from("tests/fixtures/uses_release/release-commands.toml").as_path(),
        )
        .unwrap();
        assert_eq!(
            commands_config.release,
            Some(vec![
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec![
                        "-c".to_string(),
                        "echo 'Release in release-commands.toml'".to_string()
                    ]),
                    source: None,
                },
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec![
                        "-c".to_string(),
                        "echo 'Another release command in release-commands.toml'".to_string()
                    ]),
                    source: None,
                }
            ])
        );
        assert_eq!(commands_config.release_build, None);
    }

    #[test]
    fn reads_commands_toml_for_release_build_command() {
        let commands_config = read_commands_config(
            PathBuf::from("tests/fixtures/uses_release_build/release-commands.toml").as_path(),
        )
        .unwrap();
        assert_eq!(
            commands_config.release_build,
            Some(Executable {
                command: "bash".to_string(),
                args: Some(vec![
                    "-c".to_string(),
                    "echo 'Release Build in release-commands.toml'".to_string()
                ]),
                source: None,
            })
        );
        assert_eq!(commands_config.release, None);
    }

    #[test]
    fn no_commands_toml() {
        let commands_config = read_commands_config(
            PathBuf::from("tests/fixtures/no_commands_toml/release-commands.toml").as_path(),
        )
        .unwrap();
        assert!(commands_config.release.is_none());
        assert!(commands_config.release_build.is_none());
    }

    // The write tests all touch the same file, so run them sequentially.
    #[test]
    fn writes_commands_toml_all() {
        writes_commands_toml();
        writes_empty_commands_toml();
    }

    fn writes_commands_toml() {
        let expected_config: toml::Value = toml! {
            [[release]]
            command = "bash"
            args = ["-c", "echo 'Release in write test'"]

            [[release]]
            command = "bash"
            args = ["-c", "echo 'Another release command in write test'"]

            [release-build]
            command = "bash"
            args = ["-c", "echo 'Release Build in write test'"]
        }
        .into();
        let release_commands = ReleaseCommands {
            release: Some(vec![
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec![
                        "-c".to_string(),
                        "echo 'Release in write test'".to_string(),
                    ]),
                    source: None,
                },
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec![
                        "-c".to_string(),
                        "echo 'Another release command in write test'".to_string(),
                    ]),
                    source: None,
                },
            ]),
            release_build: Some(Executable {
                command: "bash".to_string(),
                args: Some(vec![
                    "-c".to_string(),
                    "echo 'Release Build in write test'".to_string(),
                ]),
                source: None,
            }),
        };

        let dir = env::temp_dir();
        write_commands_config(&dir, &release_commands).expect("toml file is written");
        let generated_path = dir.join("release-commands.toml");

        let generated_toml =
            read_toml_file::<toml::Value>(&generated_path).expect("toml file is read");
        remove_file(&generated_path).expect("toml file is deleted");

        assert_eq!(
            toml_select_value(vec!["release"], &generated_toml),
            expected_config.get("release")
        );
        assert_eq!(
            toml_select_value(vec!["release-build"], &generated_toml),
            expected_config.get("release-build")
        );
    }

    fn writes_empty_commands_toml() {
        let release_commands = ReleaseCommands {
            release: None,
            release_build: None,
        };

        let dir = env::temp_dir();
        write_commands_config(&dir, &release_commands).expect("toml file is written");
        let generated_path = dir.join("release-commands.toml");

        let generated_toml =
            read_toml_file::<toml::Value>(&generated_path).expect("toml file is read");
        remove_file(&generated_path).expect("toml file is deleted");

        let table = generated_toml.as_table().expect("a toml table");
        assert!(table.is_empty());
    }
}
