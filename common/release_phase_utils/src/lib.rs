use std::path::Path;

use libcnb::{read_toml_file, TomlFileError};
use libherokubuildpack::toml::toml_select_value;

pub fn read_project_config(dir: &Path) -> Result<Option<toml::Value>, TomlFileError> {
    let project_toml_path = dir.join("project.toml");
    let project_toml = if project_toml_path.is_file() {
        read_toml_file::<toml::Value>(project_toml_path)?
    } else {
        toml::Table::new().into()
    };
    let mut release_phase_toml = toml::Table::new();
    if let Some(release_config) =
        toml_select_value(vec!["com", "heroku", "phase", "release"], &project_toml).cloned()
    {
        release_phase_toml.insert("release".to_string(), release_config);
    };
    if let Some(release_build_config) = toml_select_value(
        vec!["com", "heroku", "phase", "release-build"],
        &project_toml,
    )
    .cloned()
    {
        release_phase_toml.insert("release-build".to_string(), release_build_config);
    };
    if release_phase_toml.is_empty() {
        Ok(None)
    } else {
        Ok(Some(release_phase_toml.into()))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use libherokubuildpack::toml::toml_select_value;
    use toml::toml;

    use crate::read_project_config;

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
            [[release-build]]
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
}
