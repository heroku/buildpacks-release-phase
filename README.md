# Heroku Cloud Native Buildpacks: Release Phase

🚧 **This repo is experimental.** Use at your own risk. 🚧

This repository is the home of Heroku Cloud Native Buildpacks for Release Phase, the mechanism offered by Heroku to execute code for each App Release, such as builds, pipeline promotions, and config var changes. Release Phase provides a hook useful for automating deployments, such as database migrations, object storage for caching, and other application-specific operations.

This buildpack enhances Release Phase capabilities to support multiple, ordered `release` commands and `release-build` for generating static web artifacts during Release Phase.

## Included Buildpacks

| ID                       | Name                                                 |
|--------------------------|------------------------------------------------------|
| `heroku/release-phase`   | [Release Phase](buildpacks/release-phase/README.md)  |

## Dev Notes

### Run Tests

```bash
cargo test -- --include-ignored
```

### Package & Run

```bash
cargo libcnb package --target aarch64-unknown-linux-musl

pack build cnb-release-phase-test \
  --buildpack packaged/aarch64-unknown-linux-musl/debug/heroku_release-phase \
  --builder heroku/builder:24 \
  --path buildpacks/release-phase/tests/fixtures/project_uses_release_build

docker run -it cnb-release-phase-test bash
/workspace$ export \
  RELEASE_ID=my-test-1 \
  STATIC_ARTIFACTS_ACCESS_KEY_ID=xxxxx \
  STATIC_ARTIFACTS_SECRET_ACCESS_KEY=xxxxx \
  STATIC_ARTIFACTS_URL=s3://xxxxx \
  STATIC_ARTIFACTS_REGION=us-east-1
/workspace$ mkdir -p static-artifacts; echo "Hello static world!" > static-artifacts/note.txt
/workspace$ save-release-artifacts static-artifacts/
```

### Releasing A New Version

[Action workflows](https://github.com/heroku/buildpacks-release-phase/actions) are used to automate the release process:

1. Run **Prepare Buildpack Releases**.
1. Await completion of the preparation step.
1. Run **Release Buildpacks**.
