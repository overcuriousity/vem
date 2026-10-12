# Contributing

`main` is the release channel. Every push to `main` builds the `nightly` prerelease, and a
version bump on `main` publishes a release. Only finished, QA-checked features land there.

- A feature is built on an integration branch `feature/<name>`, cut from `main`.
- Each slice of the feature is a PR into `feature/<name>`. CI runs on those PRs; nothing is
  published from them.
- Keep the feature branch current by merging `main` into it (no rebase, no force-push).
- When the feature is done and QA has passed, it reaches `main` in one PR from
  `feature/<name>`. That PR carries the version bump.
- Small fixes that are complete on their own may PR straight into `main`.
