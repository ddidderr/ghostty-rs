# Agent Instructions

## Commit Policy

Automatically commit changes once a full feature, bugfix, refactor, or other
coherent unit of work is finished. Do not wait for the user to ask for a commit.

## Versioning Policy

Only update the version, when the user explicitly asks for it.

### Guidelines for how to update the version

1. Use `cargo set-version` to bump the version. Decide, based on the actual
   changes, based on semver semantics, if major, minor or patch needs to be
   bumped.
2. Update the CHANGELOG.md file accordingly.
3. Create a release commit.
4. Tag the release commit in the style of previous versioning tags.
