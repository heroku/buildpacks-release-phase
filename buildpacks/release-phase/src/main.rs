mod errors;
mod setup_release_phase;

use crate::errors::{on_error, ReleasePhaseBuildpackError};
use libcnb::build::{BuildContext, BuildResult, BuildResultBuilder};
use libcnb::data::build_plan::{BuildPlanBuilder, Require};
use libcnb::data::launch::{LaunchBuilder, ProcessBuilder};
use libcnb::data::process_type;
use libcnb::detect::{DetectContext, DetectResult, DetectResultBuilder};
use libcnb::generic::{GenericMetadata, GenericPlatform};
use libcnb::{buildpack_main, Buildpack, Error};
use libherokubuildpack::log::log_header;
use setup_release_phase::setup_release_phase;

// Silence unused dependency warning for
// dependencies only used in tests
#[cfg(test)]
use libcnb_test as _;
#[cfg(test)]
use tempfile as _;
#[cfg(test)]
use test_support as _;
#[cfg(test)]
use uuid as _;

// Silence unused dependency warning for
// dependencies used in bin/ executables
use release_artifacts as _;
use tokio as _;

const BUILDPACK_NAME: &str = "Heroku Release Phase Buildpack";
const BUILD_PLAN_ID: &str = "release-phase";

pub(crate) struct ReleasePhaseBuildpack;

impl Buildpack for ReleasePhaseBuildpack {
    type Platform = GenericPlatform;
    type Metadata = GenericMetadata;
    type Error = ReleasePhaseBuildpackError;

    fn detect(&self, _context: DetectContext<Self>) -> libcnb::Result<DetectResult, Self::Error> {
        let plan_builder = BuildPlanBuilder::new()
            .provides(BUILD_PLAN_ID)
            .requires(Require::new(BUILD_PLAN_ID));

        DetectResultBuilder::pass()
            .build_plan(plan_builder.build())
            .build()
    }

    fn build(&self, context: BuildContext<Self>) -> libcnb::Result<BuildResult, Self::Error> {
        log_header(BUILDPACK_NAME);

        match setup_release_phase(&context)? {
            Some(release_phase_layer) => BuildResultBuilder::new()
                .launch(
                    LaunchBuilder::new()
                        .process(
                            ProcessBuilder::new(
                                process_type!("release"),
                                [
                                    "exec-release-commands",
                                    &release_phase_layer
                                        .path()
                                        .join("release-commands.toml")
                                        .to_string_lossy(),
                                ],
                            )
                            .build(),
                        )
                        .build(),
                )
                .build(),
            None => BuildResultBuilder::new().build(),
        }
    }

    fn on_error(&self, error: Error<Self::Error>) {
        on_error(error);
    }
}

impl From<ReleasePhaseBuildpackError> for libcnb::Error<ReleasePhaseBuildpackError> {
    fn from(value: ReleasePhaseBuildpackError) -> Self {
        libcnb::Error::BuildpackError(value)
    }
}

buildpack_main!(ReleasePhaseBuildpack);
