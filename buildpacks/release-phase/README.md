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

### Release Build command

```toml
[com.heroku.phase.release-build]
command = "bash"
args = ["-c", "npm build"]
```

This command must output release artifacts into `/workspace/static-artifacts/`. The content of this directory will be stored during Release Phase by the `RELEASE_ID`, and then automatically retrieved for `web` processes, during start-up.

### Release commands

```toml
[[com.heroku.phase.release]]
command = "bash"
args = ["-c", "rake db:migrate"]

[[com.heroku.phase.release]]
command = "bash"
args = ["-c", "./bin/purge-cache"]
```

These commands are ephemeral. No changes to the filesystem are persisted.

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


