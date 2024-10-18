// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use libcnb_test::assert_contains;
use test_support::{release_phase_integration_test, start_container_entrypoint};

#[test]
#[ignore = "integration test"]
fn project_uses_release() {
    release_phase_integration_test("./fixtures/project_uses_release", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
        assert_contains!(ctx.pack_stdout, "Successfully built image");
        start_container_entrypoint(&ctx, &"release".to_string(), |container| {
            let log_output = container.logs_now();
            assert_contains!(log_output.stderr, "release-phase plan");
            assert_contains!(log_output.stdout, "Hello from Release Phase Buildpack!");
            assert_contains!(
                log_output.stdout,
                "Hello again from Release Phase Buildpack!"
            );
            assert_contains!(log_output.stderr, "release-phase complete.");
        });
    });
}

#[test]
#[ignore = "integration test"]
fn project_uses_release_build() {
    release_phase_integration_test("./fixtures/project_uses_release_build", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
        assert_contains!(ctx.pack_stdout, "Successfully built image");
        start_container_entrypoint(&ctx, &"release".to_string(), |container| {
            let log_output = container.logs_now();
            assert_contains!(log_output.stderr, "release-phase plan");
            assert_contains!(log_output.stdout, "Build in Release Phase Buildpack!");
            // This error is expected because we are not integrating with AWS S3 in this test.
            assert_contains!(
                log_output.stderr,
                "save-release-artifacts failed: StorageURLMissing"
            );
        });
    });
}

#[test]
#[ignore = "integration test"]
fn no_project_toml() {
    release_phase_integration_test("./fixtures/no_project_toml", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
        assert_contains!(ctx.pack_stdout, "Successfully built image");
    });
}
