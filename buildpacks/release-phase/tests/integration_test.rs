// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use libcnb_test::assert_contains;
use test_support::release_phase_integration_test;

#[test]
#[ignore = "integration test"]
fn project_uses_release() {
    release_phase_integration_test("./fixtures/project_uses_release", |ctx| {
        assert_contains!(ctx.pack_stdout, "Release Phase");
    });
}
