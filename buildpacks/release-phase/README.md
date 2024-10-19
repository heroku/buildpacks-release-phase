# Heroku Cloud Native Release Phase Buildpack

Enhances Release Phase capabilities to support multiple, ordered "release" commands and "release-build" for Static Web Apps.

## Configuration: `project.toml`

### Set the buildpack

```toml
[_]
schema-version = "0.2"

[[io.buildpacks.group]]
uri = "heroku/release-phase"
```

### Release commands

*Multiple `release` commands are supported as a TOML array, their entries declared by `[[…]]`.*

```toml
[[com.heroku.phase.release]]
command = "bash"
args = ["-c", "rake db:migrate"]

[[com.heroku.phase.release]]
command = "bash"
args = ["-c", "./bin/purge-cache"]
```

These commands are ephemeral. No changes to the filesystem are persisted.

### Release Build command

*Only a single `release-build` command is supported. The entry must be declared with `[…]`.*

```toml
[com.heroku.phase.release-build]
command = "bash"
args = ["-c", "npm build"]
```

This command must output release artifacts into `/workspace/static-artifacts/`. The content of this directory will be stored during Release Phase by the `RELEASE_ID`, and then automatically retrieved for `web` processes, during start-up.

## Configuration: runtime environment vars

### `RELEASE_ID`

**Required.** Should be provided by the runtime environment, such as a UUID or version number.

Artifacts are stored at the `STATIC_ARTIFACTS_URL` with the name `release-<RELEASE_ID>.tgz`.

### `STATIC_ARTIFACTS_URL`

**Required.** May be a `file:///` or `s3://` URL allowing read, write, & list.

`file` URLs are always interpreted as absolute filesystem path, starting with `/`.

`s3` URLs should refer to an AWS S3-compatible object store. If the hostname follows AWS bucket pattern: `<bucket_name>.s3.<region>.amazonaws.com`, then the region it specifies will override `STATIC_ARTIFACTS_REGION`.

### `STATIC_ARTIFACTS_REGION`

**Required for `s3` URLs.** The region defaulting to `us-east-1`.

### `STATIC_ARTIFACTS_ACCESS_KEY_ID`

**Required for `s3` URLs.** The access key ID.

### `STATIC_ARTIFACTS_SECRET_ACCESS_KEY`

**Required for `s3` URLs.** The access secret.

## Inherited Configuration

Other buildpacks can return a [Build Plan](https://github.com/buildpacks/spec/blob/main/buildpack.md#build-plan-toml) from `detect` for Release Phase configuration.

Configuration defined in an app's `project.toml` takes precedence over this inherited Build Plan configuration.

This example sets a `release` & `release-build` commands in the build plan, following the [project configuration](#configuration-projecttoml):

```toml
[[requires]]
name = "release-phase"

[requires.metadata.release-build]
command = "bash"
args = ["-c", "npm run build"]
source = "My Awesome Buildpack"

[[requires.metadata.release]]
command = "bash"
args = ["-c", "echo 'Hello world!'"]
source = "My Awesome Buildpack"
```

Example using [libcnb.rs]():

```rust
fn detect(&self, context: DetectContext<Self>) -> libcnb::Result<DetectResult, Self::Error> {
    let mut release_phase_req = Require::new("release-phase");
    let _ = release_phase_req.metadata(toml! {
        [release-build]
        command = "bash"
        args = ["-c", "npm run build"]
        source = "My Awesome Buildpack"
    });
    let plan_builder = BuildPlanBuilder::new()
        .requires(release_phase_req);

    DetectResultBuilder::pass()
        .build_plan(plan_builder.build())
        .build()
}
```
