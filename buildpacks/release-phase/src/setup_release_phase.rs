use std::fs;

use crate::{ReleasePhaseBuildpack, ReleasePhaseBuildpackError, BUILD_PLAN_ID};
use libcnb::data::layer_name;
use libcnb::layer::LayerRef;
use libcnb::{additional_buildpack_binary_path, read_toml_file};
use libcnb::{build::BuildContext, layer::UncachedLayerDefinition};
use libherokubuildpack::log::log_info;
use release_commands::{generate_commands_config, write_commands_config};
use toml::Table;

pub(crate) fn setup_release_phase(
    context: &BuildContext<ReleasePhaseBuildpack>,
) -> Result<
    Option<LayerRef<ReleasePhaseBuildpack, (), ()>>,
    libcnb::Error<ReleasePhaseBuildpackError>,
> {
    let project_toml_path = &context.app_dir.join("project.toml");
    let project_toml = if project_toml_path.is_file() {
        read_toml_file::<toml::Value>(project_toml_path)
            .map_err(ReleasePhaseBuildpackError::CannotReadProjectToml)?
    } else {
        toml::Table::new().into()
    };

    let build_plan_config = generate_build_plan_config(context);

    let commands_config = generate_commands_config(&project_toml, build_plan_config)
        .map_err(ReleasePhaseBuildpackError::ConfigurationFailed)?;

    if commands_config.release.is_none() && commands_config.release_build.is_none() {
        log_info("No release commands are configured.");
        return Ok(None);
    }

    let release_phase_layer = context.uncached_layer(
        layer_name!("main"),
        UncachedLayerDefinition {
            build: false,
            launch: true,
        },
    )?;

    log_info("Writing release-commands.toml");
    write_commands_config(release_phase_layer.path().as_path(), &commands_config)
        .map_err(ReleasePhaseBuildpackError::ConfigurationFailed)?;

    log_info("Installing processes…");
    let exec_destination = release_phase_layer.path().join("bin");
    fs::create_dir_all(&exec_destination)
        .map_err(ReleasePhaseBuildpackError::CannotInstallCommandExecutor)?;

    let main_exec = exec_destination.join("exec-release-commands");
    log_info(format!("  {main_exec:?}"));
    fs::copy(
        additional_buildpack_binary_path!("exec-release-commands"),
        main_exec,
    )
    .map_err(ReleasePhaseBuildpackError::CannotInstallCommandExecutor)?;

    if commands_config.release_build.is_some() {
        let save_exec = exec_destination.join("save-release-artifacts");
        log_info(format!("  {save_exec:?}"));
        fs::copy(
            additional_buildpack_binary_path!("save-release-artifacts"),
            save_exec,
        )
        .map_err(ReleasePhaseBuildpackError::CannotInstallArtifactSaver)?;

        let gc_exec = exec_destination.join("gc-release-artifacts");
        log_info(format!("  {gc_exec:?}"));
        fs::copy(
            additional_buildpack_binary_path!("gc-release-artifacts"),
            gc_exec,
        )
        .map_err(ReleasePhaseBuildpackError::CannotInstallArtifactGc)?;

        let web_exec_destination = release_phase_layer.path().join("exec.d/web");
        let load_exec = web_exec_destination.join("load-release-artifacts");
        log_info(format!("  {load_exec:?}"));
        fs::create_dir_all(&web_exec_destination)
            .map_err(ReleasePhaseBuildpackError::CannotCreatWebExecD)?;
        fs::copy(
            additional_buildpack_binary_path!("load-release-artifacts"),
            load_exec,
        )
        .map_err(ReleasePhaseBuildpackError::CannotInstallArtifactLoader)?;
    }

    Ok(Some(release_phase_layer))
}

// Load a table of Build Plan [requires.metadata] from context.
// When a key is defined multiple times,
// * for arrays: append the new array value to the existing array value
// * for other value types: the values overwrite, so the last one defined wins
fn generate_build_plan_config(
    context: &BuildContext<ReleasePhaseBuildpack>,
) -> toml::map::Map<String, toml::Value> {
    let mut build_plan_config = Table::new();
    context.buildpack_plan.entries.iter().for_each(|e| {
        if e.name == BUILD_PLAN_ID {
            e.metadata.iter().for_each(|(k, v)| {
                if let Some(new_values) = v.as_array() {
                    if let Some(existing_values) =
                        build_plan_config.get(k).and_then(|ev| ev.as_array())
                    {
                        let mut all_values = existing_values.clone();
                        all_values.append(new_values.clone().as_mut());
                        build_plan_config.insert(k.to_owned(), all_values.into());
                    } else {
                        build_plan_config.insert(k.to_owned(), v.to_owned());
                    }
                } else {
                    build_plan_config.insert(k.to_owned(), v.to_owned());
                }
            });
        }
    });
    build_plan_config
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, path::PathBuf};

    use libcnb::{
        build::BuildContext,
        data::{
            buildpack::{Buildpack, BuildpackApi, BuildpackVersion, ComponentBuildpackDescriptor},
            buildpack_id,
            buildpack_plan::{BuildpackPlan, Entry},
        },
        generic::GenericPlatform,
        Env, Target,
    };
    use toml::toml;

    use crate::{ReleasePhaseBuildpack, BUILD_PLAN_ID};

    use super::generate_build_plan_config;

    #[test]
    fn generate_build_plan_config_from_one_entry() {
        let test_build_plan = vec![Entry {
            name: BUILD_PLAN_ID.to_string(),
            metadata: toml! {
                [[release]]
                command = "test"

                [release-build]
                command = "testbuild"
            },
        }];
        let test_context = create_test_context(test_build_plan);
        let result = generate_build_plan_config(&test_context);

        let result_release_commands = result
            .get("release")
            .expect("should contain release commands");
        let result_array = result_release_commands
            .as_array()
            .expect("should contain an array");
        assert_eq!(result_array.len(), 1);
        let result_executable = result_array[0].as_table().expect("should contain a table");
        assert_eq!(
            result_executable.get("command"),
            Some(&toml::Value::String("test".to_string()))
        );

        let result_release_build = result
            .get("release-build")
            .expect("should contain release build command");
        let result_command = result_release_build
            .as_table()
            .expect("should contain a table");
        assert_eq!(
            result_command.get("command"),
            Some(&toml::Value::String("testbuild".to_string()))
        );
    }

    #[test]
    fn generate_build_plan_config_collects_release_commands_from_entries() {
        let test_build_plan = vec![
            Entry {
                name: BUILD_PLAN_ID.to_string(),
                metadata: toml! {
                    [[release]]
                    command = "test1"
                },
            },
            Entry {
                name: BUILD_PLAN_ID.to_string(),
                metadata: toml! {
                    [[release]]
                    command = "test2"

                    [[release]]
                    command = "test3"
                },
            },
            Entry {
                name: BUILD_PLAN_ID.to_string(),
                metadata: toml! {
                    [[release]]
                    command = "test4"
                },
            },
        ];
        let test_context = create_test_context(test_build_plan);
        let result = generate_build_plan_config(&test_context);

        let result_release_commands = result
            .get("release")
            .expect("should contain release commands");
        let result_array = result_release_commands
            .as_array()
            .expect("should contain an array");
        assert_eq!(result_array.len(), 4);
        let result_executable_1 = result_array[0].as_table().expect("should contain a table");
        let result_executable_2 = result_array[1].as_table().expect("should contain a table");
        let result_executable_3 = result_array[2].as_table().expect("should contain a table");
        let result_executable_4 = result_array[3].as_table().expect("should contain a table");
        assert_eq!(
            result_executable_1.get("command"),
            Some(&toml::Value::String("test1".to_string()))
        );
        assert_eq!(
            result_executable_2.get("command"),
            Some(&toml::Value::String("test2".to_string()))
        );
        assert_eq!(
            result_executable_3.get("command"),
            Some(&toml::Value::String("test3".to_string()))
        );
        assert_eq!(
            result_executable_4.get("command"),
            Some(&toml::Value::String("test4".to_string()))
        );
    }

    #[test]
    fn generate_build_plan_config_captures_last_release_build_command_from_entries() {
        let test_build_plan = vec![
            Entry {
                name: BUILD_PLAN_ID.to_string(),
                metadata: toml! {
                    [release-build]
                    command = "testbuild1"
                },
            },
            Entry {
                name: BUILD_PLAN_ID.to_string(),
                metadata: toml! {
                    [release-build]
                    command = "testbuild2"
                },
            },
        ];
        let test_context = create_test_context(test_build_plan);
        let result = generate_build_plan_config(&test_context);

        let result_release_build = result
            .get("release-build")
            .expect("should contain release build command");
        let result_command = result_release_build
            .as_table()
            .expect("should contain a table");
        assert_eq!(
            result_command.get("command"),
            Some(&toml::Value::String("testbuild2".to_string()))
        );
    }

    #[test]
    fn generate_build_plan_config_empty() {
        let test_build_plan = vec![];
        let test_context = create_test_context(test_build_plan);
        let result = generate_build_plan_config(&test_context);
        assert!(result.is_empty());
    }

    fn create_test_context(build_plan: Vec<Entry>) -> BuildContext<ReleasePhaseBuildpack> {
        let test_context: BuildContext<ReleasePhaseBuildpack> = BuildContext {
            layers_dir: PathBuf::new(),
            app_dir: PathBuf::new(),
            buildpack_dir: PathBuf::new(),
            target: Target {
                os: "test".to_string(),
                arch: "test".to_string(),
                arch_variant: None,
                distro_name: "test".to_string(),
                distro_version: "test".to_string(),
            },
            platform: GenericPlatform::new(<Env as std::default::Default>::default()),
            buildpack_plan: BuildpackPlan {
                entries: build_plan,
            },
            buildpack_descriptor: ComponentBuildpackDescriptor {
                api: BuildpackApi { major: 0, minor: 0 },
                buildpack: Buildpack {
                    id: buildpack_id!("heroku/test"),
                    name: None,
                    version: BuildpackVersion::new(0, 0, 0),
                    homepage: None,
                    clear_env: false,
                    description: None,
                    keywords: vec![],
                    licenses: vec![],
                    sbom_formats: HashSet::new(),
                },
                stacks: vec![],
                targets: vec![],
                metadata: None,
            },
            store: None,
        };
        test_context
    }
}
