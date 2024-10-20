// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::fs;

use libcnb_test::{assert_contains, ContainerConfig};
use test_support::{
    release_phase_and_procfile_integration_test, release_phase_integration_test,
    start_container_entrypoint,
};
use uuid::Uuid;

#[test]
#[ignore = "integration test"]
fn project_uses_release() {
    release_phase_integration_test("./fixtures/project_uses_release", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
        assert_contains!(ctx.pack_stdout, "Successfully built image");
        start_container_entrypoint(
            &ctx,
            &mut ContainerConfig::new(),
            &"release".to_string(),
            |container| {
                let log_output = container.logs_now();
                assert_contains!(log_output.stderr, "release-phase plan");
                assert_contains!(log_output.stdout, "Hello from Release Phase Buildpack!");
                assert_contains!(
                    log_output.stdout,
                    "Hello again from Release Phase Buildpack!"
                );
                assert_contains!(log_output.stderr, "release-phase complete.");
            },
        );
    });
}

#[test]
#[ignore = "integration test"]
fn project_uses_release_build() {
    release_phase_integration_test("./fixtures/project_uses_release_build", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
        assert_contains!(ctx.pack_stdout, "Successfully built image");
        start_container_entrypoint(
            &ctx,
            ContainerConfig::new().env("RELEASE_ID", "xyz").env(
                "STATIC_ARTIFACTS_URL",
                "file:///workspace/static-artifacts-storage",
            ),
            &"release".to_string(),
            |container| {
                let log_output = container.logs_now();
                assert_contains!(log_output.stderr, "release-phase plan");
                assert_contains!(log_output.stdout, "Build in Release Phase Buildpack!");
                assert_contains!(
                    log_output.stdout,
                    "save-release-artifacts writing archive: release-xyz.tgz"
                );
                assert_contains!(log_output.stderr, "release-phase complete.");
            },
        );
    });
}

#[test]
#[ignore = "integration test"]
fn project_uses_release_build_and_web_process_loads_artifacts() {
    release_phase_and_procfile_integration_test(
        "./fixtures/project_uses_release_build_with_web_process",
        |ctx| {
            let unique = Uuid::new_v4();
            let local_storage_path = "./.integration-test-static-artifacts-storage";
            let container_volume_path = "/workspace/static-artifacts-storage";
            let container_volume_url = "file://".to_owned() + container_volume_path;
            let volume = local_storage_path.to_owned() + ":" + container_volume_path;

            assert_contains!(ctx.pack_stdout, "Procfile");
            assert_contains!(ctx.pack_stdout, "Release Phase");
            assert_contains!(ctx.pack_stdout, "Successfully built image");
            start_container_entrypoint(
                &ctx,
                ContainerConfig::new()
                    .env("RELEASE_ID", unique)
                    .env("STATIC_ARTIFACTS_URL", &container_volume_url)
                    .volumes([volume.clone()]),
                &"release".to_string(),
                |container| {
                    let log_output = container.logs_now();
                    assert_contains!(log_output.stderr, "release-phase plan");
                    assert_contains!(log_output.stdout, "Build in Release Phase Buildpack!");
                    assert_contains!(
                        log_output.stdout,
                        format!("save-release-artifacts writing archive: release-{unique}.tgz")
                            .as_str()
                    );
                    assert_contains!(log_output.stderr, "release-phase complete.");
                },
            );
            start_container_entrypoint(
                &ctx,
                ContainerConfig::new()
                    .env("RELEASE_ID", unique)
                    .env("STATIC_ARTIFACTS_URL", &container_volume_url)
                    .volumes([volume.clone()]),
                &"web".to_string(),
                |container| {
                    let log_output = container.logs_now();
                    assert_contains!(log_output.stderr, "load-release-artifacts complete.");
                    assert_contains!(
                        log_output.stdout,
                        format!("STATIC_ARTIFACTS_LOADED_FROM_KEY=release-{unique}.tgz").as_str(),
                    );
                    assert_contains!(log_output.stdout, "Hello static world!");
                },
            );
            fs::remove_file(format!("{local_storage_path}/release-{unique}.tgz"))
                .expect("the generated test archive can be deleted");
        },
    );
}

#[test]
#[ignore = "integration test"]
fn project_uses_release_build_missing_env_vars() {
    release_phase_integration_test("./fixtures/project_uses_release_build", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
        assert_contains!(ctx.pack_stdout, "Successfully built image");
        start_container_entrypoint(
            &ctx,
            &mut ContainerConfig::new(),
            &"release".to_string(),
            |container| {
                let log_output = container.logs_now();
                assert_contains!(log_output.stderr, "release-phase plan");
                assert_contains!(log_output.stdout, "Build in Release Phase Buildpack!");
                assert_contains!(
                    log_output.stderr,
                    "save-release-artifacts failed: StorageURLMissing"
                );
            },
        );
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
