# Official LocalSend Rust core

This directory is an unmodified copy of the official LocalSend Rust package:

- Repository: <https://github.com/localsend/localsend>
- Upstream path: `packages/core`
- Commit: `e3963655a1465143183d2d37f1940aed9272205b`
- License: Apache-2.0 (see `LICENSE`)

It is committed as a repository-relative dependency because Cargo cannot fetch
only one package from the upstream monorepo. Keeping the reviewed 580 KB core
here makes Docker and GitHub Actions builds fast and reproducible. Updates must
replace this directory from a reviewed upstream commit and update this file.
