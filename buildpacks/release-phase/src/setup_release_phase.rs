use std::fs;

use crate::{ReleasePhaseBuildpack, ReleasePhaseBuildpackError};
use libcnb::additional_buildpack_binary_path;
use libcnb::data::layer_name;
use libcnb::layer::LayerRef;
use libcnb::{build::BuildContext, layer::UncachedLayerDefinition};
use libherokubuildpack::log::log_info;
use release_commands::{read_project_config, write_commands_config};

pub(crate) fn setup_release_phase(
    context: &BuildContext<ReleasePhaseBuildpack>,
) -> Result<
    Option<LayerRef<ReleasePhaseBuildpack, (), ()>>,
    libcnb::Error<ReleasePhaseBuildpackError>,
> {
    let project_config = read_project_config(&context.app_dir.join("project.toml"))
        .map_err(ReleasePhaseBuildpackError::ConfigurationFailed)?;

    if project_config.release.is_none() && project_config.release_build.is_none() {
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
    write_commands_config(release_phase_layer.path().as_path(), &project_config)
        .map_err(ReleasePhaseBuildpackError::ConfigurationFailed)?;

    log_info("Installing processes…");
    let exec_destination = release_phase_layer.path().join("bin");
    fs::create_dir_all(&exec_destination)
        .map_err(ReleasePhaseBuildpackError::CannotInstallCommandExecutor)?;

    log_info("  exec-release-commands");
    fs::copy(
        additional_buildpack_binary_path!("exec-release-commands"),
        exec_destination.join("exec-release-commands"),
    )
    .map_err(ReleasePhaseBuildpackError::CannotInstallCommandExecutor)?;

    if project_config.release_build.is_some() {
        log_info("  upload-release-artifacts");
        fs::copy(
            additional_buildpack_binary_path!("upload-release-artifacts"),
            exec_destination.join("upload-release-artifacts"),
        )
        .map_err(ReleasePhaseBuildpackError::CannotInstallArtifactUploader)?;
    }

    Ok(Some(release_phase_layer))
}
