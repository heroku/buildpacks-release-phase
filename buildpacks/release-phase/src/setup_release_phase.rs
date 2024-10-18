use std::fs;

use crate::{ReleasePhaseBuildpack, ReleasePhaseBuildpackError};
use libcnb::additional_buildpack_binary_path;
use libcnb::data::layer_name;
use libcnb::layer::LayerRef;
use libcnb::{build::BuildContext, layer::UncachedLayerDefinition};
use libherokubuildpack::log::log_info;
use release_commands::{generate_commands_config, write_commands_config};

pub(crate) fn setup_release_phase(
    context: &BuildContext<ReleasePhaseBuildpack>,
) -> Result<
    Option<LayerRef<ReleasePhaseBuildpack, (), ()>>,
    libcnb::Error<ReleasePhaseBuildpackError>,
> {
    let commands_config = generate_commands_config(&context.app_dir.join("project.toml"))
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
