# Heroku Cloud Native Buildpacks: Release Phase

🚧 **This repo is under-construction,** not yet functional. 🚧

This repository is the home of Heroku Cloud Native Buildpacks for Release Phase, the mechanism offered by Heroku to execute code for each App Release, such as builds, pipeline promotions, and config var changes. Release Phase provides a hook useful for automating deployments, such as database migrations, object storage for caching, and other application-specific operations.

This buildpack enhances Release Phase capabilities to support multiple, ordered `release` commands and `release-build` for [Front-end Web apps](https://github.com/heroku/buildpacks-frontend-web).

## Included Buildpacks

| ID                       | Name                                                 |
|--------------------------|------------------------------------------------------|
| `heroku/release-phase`   | [Release Phase](buildpacks/release-phase/README.md)  |

## Dev Notes

### Run Tests

```bash
cargo test -- --include-ignored
```

### Releasing A New Version

[Action workflows](https://github.com/heroku/buildpacks-release-phase/actions) are used to automate the release process:

1. Run **Prepare Buildpack Releases**.
1. Await completion of the preparation step.
1. ~~Run **Release Buildpacks**.~~ (This will not work until the repo is public, open-source. Until then, [manually pack & release the builder](builder/README.md#internal-release-process)))
