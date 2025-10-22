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
    TomlBuildPlanDeserializeError(toml::de::Error),
    TomlProjectFileError(TomlFileError),
    TomlReleaseCommandsFileError(TomlFileError),
    TomlProjectDeserializeError(toml::de::Error),
    TomlReleaseCommandsDeserializeError(toml::de::Error),
    TomlWriteReleaseCommandsFileError(TomlFileError),
    ReleaseCommandExecError(std::io::Error),
    ReleaseCommandExitedError(String),
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
            Error::TomlBuildPlanDeserializeError(error) => {
                write!(
                    f,
                    "Configuration error in Build Plan [requires.metadata], {error:#?}"
                )
            }
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
                write!(f, "Command exec failed, {error:#?}")
            }
            Error::ReleaseCommandExitedError(error) => {
                write!(f, "Command exited with error, {error}")
            }
        }
    }
}

pub fn generate_commands_config(
    project_config: &toml::Value,
    config_to_inherit: toml::map::Map<String, toml::Value>,
) -> Result<ReleaseCommands, Error> {
    // Extract the namespaced keys from project.toml
    let mut project_commands = toml::Table::new();
    if let Some(release_config) =
        toml_select_value(vec!["com", "heroku", "phase", "release"], project_config).cloned()
    {
        project_commands.insert("release".to_string(), release_config);
    }
    if let Some(release_build_config) = toml_select_value(
        vec!["com", "heroku", "phase", "release-build"],
        project_config,
    )
    .cloned()
    {
        project_commands.insert("release-build".to_string(), release_build_config);
    }

    // Create main command config from project
    let mut commands = project_commands
        .try_into::<ReleaseCommands>()
        .map_err(Error::TomlProjectDeserializeError)?;

    // Create secondary, inherited command config from Build Plan
    let inherited_commands = config_to_inherit
        .try_into::<ReleaseCommands>()
        .map_err(Error::TomlBuildPlanDeserializeError)?;

    // Combine inherited + project release commands
    if let Some(inherited) = inherited_commands.release {
        commands.release = commands.release.map_or(Some(inherited.clone()), |project| {
            Some([inherited, project].concat())
        });
    }

    // Inherit the release-build command if none defined for project
    if commands.release_build.is_none() {
        commands.release_build = inherited_commands.release_build;
    }

    // When Release Build is defined, add the artifacts saver exec as the first release command, immediately after release-build
    if commands.release_build.is_some() {
        let save_exec = Executable {
            command: "save-release-artifacts".to_string(),
            args: Some(vec!["static-artifacts/".to_string()]),
            source: Some("Heroku Release Phase Buildpack".to_string()),
        };
        commands.release = Some([vec![save_exec], commands.release.map_or(vec![], |v| v)].concat());
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

    use crate::generate_commands_config;
    use crate::read_commands_config;
    use crate::write_commands_config;
    use crate::Executable;
    use crate::ReleaseCommands;

    #[test]
    fn generate_commands_config_for_project_release() {
        let project_config: toml::Value = toml! {
            [[com.heroku.phase.release]]
            command = "bash"
            args = ["-c", "echo '1'"]

            [[com.heroku.phase.release]]
            command = "bash"
            args = ["-c", "echo '2'"]
        }
        .into();
        let inherit_config = toml::Table::new();
        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert_eq!(
            result.release,
            Some(vec![
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec!["-c".to_string(), "echo '1'".to_string()]),
                    source: None,
                },
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec!["-c".to_string(), "echo '2'".to_string()]),
                    source: None,
                }
            ])
        );
        assert_eq!(result.release_build, None);
    }

    #[test]
    fn generate_commands_config_for_project_release_build() {
        let project_config: toml::Value = toml! {
                    [com.heroku.phase.release-build]
        command = "bash"
        args = ["-c", "echo 'test build'"]
                }
        .into();
        let inherit_config = toml::Table::new();
        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert_eq!(
            result.release_build,
            Some(Executable {
                command: "bash".to_string(),
                args: Some(vec!["-c".to_string(), "echo 'test build'".to_string()]),
                source: None,
            })
        );
        assert_eq!(
            result.release,
            Some(vec![Executable {
                command: "save-release-artifacts".to_string(),
                args: Some(vec!["static-artifacts/".to_string()]),
                source: Some("Heroku Release Phase Buildpack".to_string()),
            }])
        );
    }

    #[test]
    fn generate_commands_config_when_not_defined() {
        let project_config: toml::Value = toml! {
            [some_other_key]
            test = true
        }
        .into();
        let inherit_config = toml::Table::new();
        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert!(result.release.is_none());
        assert!(result.release_build.is_none());
    }

    #[test]
    fn generate_commands_config_combined_from_build_plan_and_project() {
        let project_config: toml::Value = toml! {
            [[com.heroku.phase.release]]
            command = "project1"

            [[com.heroku.phase.release]]
            command = "project2"
        }
        .into();

        let mut inherit_commands = toml::value::Array::new();
        let mut inherit_command_1 = toml::Table::new();
        inherit_command_1.insert("command".to_string(), "buildplan1".to_string().into());
        inherit_commands.push(inherit_command_1.into());
        let mut inherit_command_2 = toml::Table::new();
        inherit_command_2.insert("command".to_string(), "buildplan2".to_string().into());
        inherit_commands.push(inherit_command_2.into());
        let mut inherit_config = toml::Table::new();
        inherit_config.insert("release".to_string(), inherit_commands.into());

        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert_eq!(
            result.release,
            Some(vec![
                Executable {
                    command: "buildplan1".to_string(),
                    args: None,
                    source: None,
                },
                Executable {
                    command: "buildplan2".to_string(),
                    args: None,
                    source: None,
                },
                Executable {
                    command: "project1".to_string(),
                    args: None,
                    source: None,
                },
                Executable {
                    command: "project2".to_string(),
                    args: None,
                    source: None,
                }
            ])
        );
        assert_eq!(result.release_build, None);
    }

    #[test]
    fn generate_commands_config_for_release_build_when_inherited_from_build_plan() {
        let project_config: toml::Value = toml! {
            [some_other_key]
            test = true
        }
        .into();

        let mut inherit_build_command = toml::Table::new();
        inherit_build_command.insert("command".to_string(), "buildplan1".to_string().into());
        let mut inherit_config = toml::Table::new();
        inherit_config.insert("release-build".to_string(), inherit_build_command.into());

        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert_eq!(
            result.release_build,
            Some(Executable {
                command: "buildplan1".to_string(),
                args: None,
                source: None,
            })
        );
        assert_eq!(
            result.release,
            Some(vec![Executable {
                command: "save-release-artifacts".to_string(),
                args: Some(vec!["static-artifacts/".to_string()]),
                source: Some("Heroku Release Phase Buildpack".to_string()),
            }])
        );
    }

    #[test]
    fn generate_commands_config_for_release_build_when_project_takes_precedence() {
        let project_config: toml::Value = toml! {
            [com.heroku.phase.release-build]
            command = "project1"
        }
        .into();

        let mut inherit_build_command = toml::Table::new();
        inherit_build_command.insert("command".to_string(), "buildplan1".to_string().into());
        let mut inherit_config = toml::Table::new();
        inherit_config.insert("release-build".to_string(), inherit_build_command.into());

        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert_eq!(
            result.release_build,
            Some(Executable {
                command: "project1".to_string(),
                args: None,
                source: None,
            })
        );
        assert_eq!(
            result.release,
            Some(vec![Executable {
                command: "save-release-artifacts".to_string(),
                args: Some(vec!["static-artifacts/".to_string()]),
                source: Some("Heroku Release Phase Buildpack".to_string()),
            }])
        );
    }

    #[test]
    fn generate_commands_config_combined_all() {
        let project_config: toml::Value = toml! {
            [[com.heroku.phase.release]]
            command = "project1"

            [[com.heroku.phase.release]]
            command = "project2"

            [com.heroku.phase.release-build]
            command = "projectbuild1"
        }
        .into();

        let mut inherit_commands = toml::value::Array::new();
        let mut inherit_command_1 = toml::Table::new();
        inherit_command_1.insert("command".to_string(), "buildplan1".to_string().into());
        inherit_commands.push(inherit_command_1.into());
        let mut inherit_command_2 = toml::Table::new();
        inherit_command_2.insert("command".to_string(), "buildplan2".to_string().into());
        inherit_commands.push(inherit_command_2.into());

        let mut inherit_build_command = toml::Table::new();
        inherit_build_command.insert("command".to_string(), "buildplan1".to_string().into());

        let mut inherit_config = toml::Table::new();
        inherit_config.insert("release-build".to_string(), inherit_build_command.into());
        inherit_config.insert("release".to_string(), inherit_commands.into());

        let result = generate_commands_config(&project_config, inherit_config).unwrap();
        assert_eq!(
            result.release,
            Some(vec![
                Executable {
                    command: "save-release-artifacts".to_string(),
                    args: Some(vec!["static-artifacts/".to_string()]),
                    source: Some("Heroku Release Phase Buildpack".to_string()),
                },
                Executable {
                    command: "buildplan1".to_string(),
                    args: None,
                    source: None,
                },
                Executable {
                    command: "buildplan2".to_string(),
                    args: None,
                    source: None,
                },
                Executable {
                    command: "project1".to_string(),
                    args: None,
                    source: None,
                },
                Executable {
                    command: "project2".to_string(),
                    args: None,
                    source: None,
                }
            ])
        );
        assert_eq!(
            result.release_build,
            Some(Executable {
                command: "projectbuild1".to_string(),
                args: None,
                source: None,
            })
        );
    }

    #[test]
    fn read_commands_config_for_release_commands() {
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
    fn read_commands_config_for_release_build_command() {
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
    fn read_commands_config_when_undefined() {
        let commands_config = read_commands_config(
            PathBuf::from("tests/fixtures/no_commands_toml/release-commands.toml").as_path(),
        )
        .unwrap();
        assert!(commands_config.release.is_none());
        assert!(commands_config.release_build.is_none());
    }

    // The write tests all touch the same file, so run them sequentially.
    #[test]
    fn write_commands_config_all() {
        write_commands_config_succeeds();
        write_commands_config_succeeds_when_empty();
    }

    fn write_commands_config_succeeds() {
        let expected_config: toml::Value = toml! {
            [[release]]
            command = "bash"
            args = ["-c", "echo '1'"]

            [[release]]
            command = "bash"
            args = ["-c", "echo '2'"]

            [release-build]
            command = "bash"
            args = ["-c", "echo '3'"]
        }
        .into();
        let release_commands = ReleaseCommands {
            release: Some(vec![
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec!["-c".to_string(), "echo '1'".to_string()]),
                    source: None,
                },
                Executable {
                    command: "bash".to_string(),
                    args: Some(vec!["-c".to_string(), "echo '2'".to_string()]),
                    source: None,
                },
            ]),
            release_build: Some(Executable {
                command: "bash".to_string(),
                args: Some(vec!["-c".to_string(), "echo '3'".to_string()]),
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

    fn write_commands_config_succeeds_when_empty() {
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
