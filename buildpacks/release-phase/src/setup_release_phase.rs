use std::fs;

use crate::{ReleasePhaseBuildpack, ReleasePhaseBuildpackError};
use libcnb::additional_buildpack_binary_path;
use libcnb::data::layer_name;
use libcnb::layer::LayerRef;
use libcnb::{build::BuildContext, layer::UncachedLayerDefinition};
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
        return Ok(None);
    }

    let release_phase_layer = context.uncached_layer(
        layer_name!("main"),
        UncachedLayerDefinition {
            build: false,
            launch: true,
        },
    )?;

    write_commands_config(release_phase_layer.path().as_path(), &project_config)
        .map_err(ReleasePhaseBuildpackError::ConfigurationFailed)?;

    let exec_destination = release_phase_layer.path().join("bin");
    fs::create_dir_all(&exec_destination)
        .map_err(ReleasePhaseBuildpackError::CannotInstallCommandExecutor)?;
    fs::copy(
        additional_buildpack_binary_path!("exec-release-commands"),
        exec_destination.join("exec-release-commands"),
    )
    .map_err(ReleasePhaseBuildpackError::CannotInstallCommandExecutor)?;

    Ok(Some(release_phase_layer))
}
