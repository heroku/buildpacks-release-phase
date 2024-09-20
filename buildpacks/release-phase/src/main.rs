mod errors;

use crate::errors::{on_error, ReleasePhaseBuildpackError};
use libcnb::build::{BuildContext, BuildResult, BuildResultBuilder};
use libcnb::detect::{DetectContext, DetectResult, DetectResultBuilder};
use libcnb::generic::{GenericMetadata, GenericPlatform};
use libcnb::{buildpack_main, Buildpack, Error};
use libherokubuildpack::log::log_header;

// Silence unused dependency warning for
// dependencies only used in tests
#[cfg(test)]
use libcnb_test as _;
#[cfg(test)]
use test_support as _;

const BUILDPACK_NAME: &str = "Heroku Release Phase Buildpack";

pub(crate) struct ReleasePhaseBuildpack;

impl Buildpack for ReleasePhaseBuildpack {
    type Platform = GenericPlatform;
    type Metadata = GenericMetadata;
    type Error = ReleasePhaseBuildpackError;

    fn detect(&self, _context: DetectContext<Self>) -> libcnb::Result<DetectResult, Self::Error> {
        DetectResultBuilder::pass().build()
    }

    fn build(&self, _context: BuildContext<Self>) -> libcnb::Result<BuildResult, Self::Error> {
        log_header(BUILDPACK_NAME);
        BuildResultBuilder::new().build()
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
