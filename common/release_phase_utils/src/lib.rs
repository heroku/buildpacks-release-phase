use std::path::Path;

use libcnb::{read_toml_file, write_toml_file, TomlFileError};
use libherokubuildpack::toml::toml_select_value;

#[derive(Debug)]
pub enum Error {
    ReleaseCommandsMustBeArray,
    ReleaseBuildCommandMustBeTable,
    TomlFileError(TomlFileError),
}

pub fn read_project_config(dir: &Path) -> Result<Option<toml::Value>, Error> {
    let project_toml_path = dir.join("project.toml");
    let project_toml = if project_toml_path.is_file() {
        read_toml_file::<toml::Value>(project_toml_path).map_err(Error::TomlFileError)?
    } else {
        toml::Table::new().into()
    };
    let mut release_commands = toml::Table::new();
    if let Some(release_config) =
        toml_select_value(vec!["com", "heroku", "phase", "release"], &project_toml).cloned()
    {
        release_commands.insert("release".to_string(), release_config);
    };
    if let Some(release_build_config) = toml_select_value(
        vec!["com", "heroku", "phase", "release-build"],
        &project_toml,
    )
    .cloned()
    {
        release_commands.insert("release-build".to_string(), release_build_config);
    };
    if release_commands.is_empty() {
        Ok(None)
    } else {
        Ok(Some(release_commands.into()))
    }
}

pub fn read_commands_config(dir: &Path) -> Result<Option<toml::Value>, Error> {
    let commands_toml_path = dir.join("release-commands.toml");
    let commands_toml = if commands_toml_path.is_file() {
        read_toml_file::<toml::Value>(commands_toml_path).map_err(Error::TomlFileError)?
    } else {
        toml::Table::new().into()
    };
    let mut release_commands = toml::Table::new();
    if let Some(release_config) = toml_select_value(vec!["release"], &commands_toml).cloned() {
        release_commands.insert("release".to_string(), release_config);
    };
    if let Some(release_build_config) =
        toml_select_value(vec!["release-build"], &commands_toml).cloned()
    {
        release_commands.insert("release-build".to_string(), release_build_config);
    };
    if release_commands.is_empty() {
        Ok(None)
    } else {
        Ok(Some(release_commands.into()))
    }
}

pub fn write_commands_config(dir: &Path, commands_toml: &toml::Value) -> Result<(), Error> {
    let commands_toml_path = dir.join("release-commands.toml");
    let mut release_commands = toml::Table::new();
    if let Some(release_config) = toml_select_value(vec!["release"], commands_toml).cloned() {
        if !release_config.is_array() {
            return Err(Error::ReleaseCommandsMustBeArray);
        }
        release_commands.insert("release".to_string(), release_config);
    };
    if let Some(release_build_config) =
        toml_select_value(vec!["release-build"], commands_toml).cloned()
    {
        if !release_build_config.is_table() {
            return Err(Error::ReleaseBuildCommandMustBeTable);
        }
        release_commands.insert("release-build".to_string(), release_build_config);
    };
    write_toml_file(&release_commands, commands_toml_path).map_err(Error::TomlFileError)
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
    use crate::Error;

    #[test]
    fn reads_project_toml_for_release_commands() {
        let project_config = read_project_config(
            PathBuf::from("../../buildpacks/release-phase/tests/fixtures/project_uses_release")
                .as_path(),
        )
        .unwrap()
        .expect("TOML value");
        let expected_toml: toml::Value = toml! {
            [[release]]
            command = ["bash"]
            args = ["-c", "echo 'Hello from Release Phase Buildpack!'"]

            [[release]]
            command = ["bash"]
            args = ["-c", "echo 'Hello again from Release Phase Buildpack!'"]
        }
        .into();
        assert_eq!(
            toml_select_value(vec!["release"], &project_config),
            expected_toml.get("release")
        );
        assert_eq!(
            toml_select_value(vec!["release-build"], &project_config),
            None
        );
    }

    #[test]
    fn reads_project_toml_for_release_build_command() {
        let project_config = read_project_config(
            PathBuf::from(
                "../../buildpacks/release-phase/tests/fixtures/project_uses_release_build",
            )
            .as_path(),
        )
        .unwrap()
        .expect("TOML value");
        let expected_toml: toml::Value = toml! {
            [release-build]
            command = ["bash"]
            args = ["-c", "echo 'Build in Release Phase Buildpack!'"]
        }
        .into();
        assert_eq!(
            toml_select_value(vec!["release-build"], &project_config),
            expected_toml.get("release-build")
        );
        assert_eq!(toml_select_value(vec!["release"], &project_config), None);
    }

    #[test]
    fn no_project_toml() {
        let project_config = read_project_config(
            PathBuf::from("../../buildpacks/release-phase/tests/fixtures/no_project_toml")
                .as_path(),
        )
        .unwrap();
        assert_eq!(project_config, None);
    }

    #[test]
    fn reads_commands_toml_for_release_commands() {
        let commands_config =
            read_commands_config(PathBuf::from("tests/fixtures/uses_release").as_path())
                .unwrap()
                .expect("TOML value");
        let expected_toml: toml::Value = toml! {
            [[release]]
            command = ["bash"]
            args = ["-c", "echo 'Release in release-commands.toml'"]

            [[release]]
            command = ["bash"]
            args = ["-c", "echo 'Another release command in release-commands.toml'"]
        }
        .into();
        assert_eq!(
            toml_select_value(vec!["release"], &commands_config),
            expected_toml.get("release")
        );
        assert_eq!(
            toml_select_value(vec!["release-build"], &commands_config),
            None
        );
    }

    #[test]
    fn reads_commands_toml_for_release_build_command() {
        let commands_config =
            read_commands_config(PathBuf::from("tests/fixtures/uses_release_build").as_path())
                .unwrap()
                .expect("TOML value");
        let expected_toml: toml::Value = toml! {
            [release-build]
            command = ["bash"]
            args = ["-c", "echo 'Release Build in release-commands.toml'"]
        }
        .into();
        assert_eq!(
            toml_select_value(vec!["release-build"], &commands_config),
            expected_toml.get("release-build")
        );
        assert_eq!(toml_select_value(vec!["release"], &commands_config), None);
    }

    #[test]
    fn no_commands_toml() {
        let commands_config =
            read_commands_config(PathBuf::from("tests/fixtures/no_commands_toml").as_path())
                .unwrap();
        assert_eq!(commands_config, None);
    }

    #[test]
    fn writes_commands_toml() {
        let commands_config: toml::Value = toml! {
            [[release]]
            command = ["bash"]
            args = ["-c", "echo 'Release in write test'"]

            [[release]]
            command = ["bash"]
            args = ["-c", "echo 'Another release command in write test'"]

            [release-build]
            command = ["bash"]
            args = ["-c", "echo 'Release Build in write test'"]
        }
        .into();

        let dir = env::temp_dir();
        write_commands_config(&dir, &commands_config).expect("toml file is written");
        let generated_path = dir.join("release-commands.toml");

        let generated_toml =
            read_toml_file::<toml::Value>(&generated_path).expect("toml file is read");
        remove_file(&generated_path).expect("toml file is deleted");

        assert_eq!(
            toml_select_value(vec!["release"], &generated_toml),
            commands_config.get("release")
        );
        assert_eq!(
            toml_select_value(vec!["release-build"], &generated_toml),
            commands_config.get("release-build")
        );
    }

    #[test]
    fn write_fails_for_bad_release() {
        let commands_config: toml::Value = toml! {
            [release]
            command = ["bash"]
            args = ["-c", "echo 'Release in write test'"]
        }
        .into();

        let dir = env::temp_dir();
        let result = write_commands_config(&dir, &commands_config);
        assert!(matches!(result, Err(Error::ReleaseCommandsMustBeArray)));
    }

    #[test]
    fn write_fails_for_bad_release_build() {
        let commands_config: toml::Value = toml! {
            [[release-build]]
            command = ["bash"]
            args = ["-c", "echo 'Release Build in write test'"]
        }
        .into();

        let dir = env::temp_dir();
        let result = write_commands_config(&dir, &commands_config);
        assert!(matches!(result, Err(Error::ReleaseBuildCommandMustBeTable)));
    }
}
