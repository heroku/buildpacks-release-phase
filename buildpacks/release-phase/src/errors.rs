use crate::BUILDPACK_NAME;
use commons_ruby::output::build_log::{BuildLog, Logger, StartedLogger};
use commons_ruby::output::fmt;
use commons_ruby::output::fmt::DEBUG_INFO;
use indoc::formatdoc;
use std::fmt::Display;
use std::io::stdout;

const SUBMIT_AN_ISSUE: &str = "\
If the issue persists and you think you found a bug in the buildpack then reproduce the issue \
locally with a minimal example and open an issue in the buildpack's GitHub repository with the details.";

#[derive(Debug)]
pub(crate) enum ReleasePhaseBuildpackError {
    CannotInstallCommandExecutor(std::io::Error),
    ConfigurationFailed(release_phase_utils::Error),
}

pub(crate) fn on_error(error: libcnb::Error<ReleasePhaseBuildpackError>) {
    let logger = BuildLog::new(stdout()).without_buildpack_name();
    match error {
        libcnb::Error::BuildpackError(buildpack_error) => {
            on_buildpack_error(buildpack_error, logger);
        }
        framework_error => on_framework_error(&framework_error, logger),
    }
}

fn on_buildpack_error(error: ReleasePhaseBuildpackError, logger: Box<dyn StartedLogger>) {
    match error {
        ReleasePhaseBuildpackError::CannotInstallCommandExecutor(error) => {
            on_unexpected_io_error(&error, logger);
        }
        ReleasePhaseBuildpackError::ConfigurationFailed(error) => {
            on_configuration_error(&error, logger);
        }
    }
}

fn on_unexpected_io_error(error: &std::io::Error, logger: Box<dyn StartedLogger>) {
    print_error_details(logger, &error)
        .announce()
        .error(&formatdoc! {"
        Unexpected IO Error in {buildpack_name}
    ", buildpack_name = fmt::value(BUILDPACK_NAME) });
}

fn on_configuration_error(error: &release_phase_utils::Error, logger: Box<dyn StartedLogger>) {
    print_error_details(logger, &error)
        .announce()
        .error(&formatdoc! {"
        Configuration failed for {buildpack_name}
    ", buildpack_name = fmt::value(BUILDPACK_NAME) });
}

fn on_framework_error(
    error: &libcnb::Error<ReleasePhaseBuildpackError>,
    logger: Box<dyn StartedLogger>,
) {
    print_error_details(logger, &error)
        .announce()
        .error(&formatdoc! {"
            {buildpack_name} internal error.

            The framework used by this buildpack encountered an unexpected error.
            
            If you can't deploy to Heroku due to this issue, check the official Heroku Status page at \
            status.heroku.com for any ongoing incidents. After all incidents resolve, retry your build.

            {SUBMIT_AN_ISSUE}
        ", buildpack_name = fmt::value(BUILDPACK_NAME) });
}

fn print_error_details(
    logger: Box<dyn StartedLogger>,
    error: &impl Display,
) -> Box<dyn StartedLogger> {
    logger
        .section(DEBUG_INFO)
        .step(&error.to_string())
        .end_section()
}
