use std::fs;

use crate::{ReleasePhaseBuildpack, ReleasePhaseBuildpackError};
use libcnb::additional_buildpack_binary_path;
use libcnb::data::layer_name;
use libcnb::layer::LayerRef;
use libcnb::{build::BuildContext, layer::UncachedLayerDefinition};
use release_phase_utils::{read_project_config, write_commands_config};

pub(crate) fn setup_release_phase(
    context: &BuildContext<ReleasePhaseBuildpack>,
) -> Result<LayerRef<ReleasePhaseBuildpack, (), ()>, libcnb::Error<ReleasePhaseBuildpackError>> {
    let release_phase_layer = context.uncached_layer(
        layer_name!("main"),
        UncachedLayerDefinition {
            build: false,
            launch: true,
        },
    )?;

    let project_config = read_project_config(&context.app_dir.join("project.toml"))
        .map_err(ReleasePhaseBuildpackError::ConfigurationFailed)?;

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

    Ok(release_phase_layer)
}
