# Building and testing Rust

Learn how to create a continuous integration (CI) workflow to build and test your Rust project.

## Introduction

This guide shows you how to build, test, and publish a Rust package.

GitHub-hosted runners have a tools cache with preinstalled software, which includes the dependencies for Rust. For a full list of up-to-date software and the preinstalled versions of Rust, see [GitHub-hosted runners](/en/actions/using-github-hosted-runners/using-github-hosted-runners/about-github-hosted-runners#preinstalled-software).

## Prerequisites

You should already be familiar with YAML syntax and how it's used with GitHub Actions. For more information, see [Workflow syntax for GitHub Actions](/en/actions/using-workflows/workflow-syntax-for-github-actions).

We recommend that you have a basic understanding of the Rust language. For more information, see [Getting started with Rust](https://www.rust-lang.org/learn).

## Using a Rust workflow template

To get started quickly, add a workflow template to the `.github/workflows` directory of your repository.

GitHub provides a Rust workflow template that should work for most basic Rust projects. The subsequent sections of this guide give examples of how you can customize this workflow template.

1. On GitHub, navigate to the main page of the repository.

2. Under your repository name, click **<svg version="1.1" width="16" height="16" viewBox="0 0 16 16" class="octicon octicon-play" aria-label="play" role="img"><path d="M8 0a8 8 0 1 1 0 16A8 8 0 0 1 8 0ZM1.5 8a6.5 6.5 0 1 0 13 0 6.5 6.5 0 0 0-13 0Zm4.879-2.773 4.264 2.559a.25.25 0 0 1 0 .428l-4.264 2.559A.25.25 0 0 1 6 10.559V5.442a.25.25 0 0 1 .379-.215Z"></path></svg> Actions**.

   ![Screenshot of the tabs for the "github/docs" repository. The "Actions" tab is highlighted with an orange outline.](/assets/images/help/repository/actions-tab-global-nav-update.png)

3. If you already have a workflow in your repository, click **New workflow**.

4. The "Choose a workflow" page shows a selection of recommended workflow templates. Search for "Rust".

5. Filter the selection of workflows by clicking **Continuous integration**.

6. On the "Rust - by GitHub Actions" workflow, click **Configure**.

   ![Screenshot of the "Choose a workflow" page. The "Configure" button on the "Rust" workflow is highlighted with an orange outline.](/assets/images/help/actions/starter-workflow-rust.png)

7. Edit the workflow as required. For example, change the version of Rust.

8. Click **Commit changes**.

   The `rust.yml` workflow file is added to the `.github/workflows` directory of your repository.

## Specifying a Rust version

GitHub-hosted runners include a recent version of the Rust toolchain. You can use rustup to report on the version installed on a runner, override the version, and to install different toolchains. For more information, see [The rustup book](https://rust-lang.github.io/rustup/).

This example shows steps you could use to setup your runner environment to use the nightly build of rust and to report the version.

```yaml copy
- name: Temporarily modify the rust toolchain version
  run: rustup override set nightly
- name: Output rust version for educational purposes
  run: rustup --version
```

### Caching dependencies

You can cache and restore dependencies using the Cache action. This example assumes that your repository contains a `Cargo.lock` file.

```yaml copy
- name: Cache
  uses: actions/cache@v4
  with:
    path: |
      ~/.cargo/registry
      ~/.cargo/git
      target
    key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
```

If you have custom requirements or need finer controls for caching, you should explore other configuration options for the [`cache` action](https://github.com/marketplace/actions/cache). For more information, see [Dependency caching reference](/en/actions/using-workflows/caching-dependencies-to-speed-up-workflows).

## Building and testing your code

You can use the same commands that you use locally to build and test your code. This example workflow demonstrates how to use `cargo build` and `cargo test` in a job:

```yaml copy
jobs:
  build:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        BUILD_TARGET: [release] # refers to a cargo profile
    outputs:
      release_built: ${{ steps.set-output.outputs.release_built }}
    steps:
      - uses: actions/checkout@v5
      - name: Build binaries in "${{ matrix.BUILD_TARGET }}" mode
        run: cargo build --profile ${{ matrix.BUILD_TARGET }}
      - name: Run tests in "${{ matrix.BUILD_TARGET }}" mode
        run: cargo test --profile ${{ matrix.BUILD_TARGET }}
```

The `release` keyword used in this example corresponds to a cargo profile. You can use any [profile](https://doc.rust-lang.org/cargo/reference/profiles.html) you have defined in your `Cargo.toml` file.

## Publishing your package or library to crates.io

Once you have setup your workflow to build and test your code, you can use a secret to login to [crates.io](https://crates.io/) and publish your package.

```yaml copy
- name: Login into crates.io
  run: cargo login ${{ secrets.CRATES_IO }}
- name: Build binaries in "release" mode
  run: cargo build -r
- name: "Package for crates.io"
  run: cargo package # publishes a package as a tarball
- name: "Publish to crates.io"
  run: cargo publish # publishes your crate as a library that can be added as a dependency
```

If there are any errors building and packaging the crate, check the metadata in your manifest, `Cargo.toml` file, see [The Manifest Format](https://doc.rust-lang.org/cargo/reference/manifest.html). You should also check your `Cargo.lock` file, see [Cargo.toml vs Cargo.lock](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html).

## Packaging workflow data as artifacts

After a workflow completes, you can upload the resulting artifacts for analysis or to use in another workflow. You could add these example steps to the workflow to upload an application for use by another workflow.

```yaml copy
- name: Upload release artifact
  uses: actions/upload-artifact@v4
  with:
    name: <my-app>
    path: target/${{ matrix.BUILD_TARGET }}/<my-app>
```

To use the uploaded artifact in a different job, ensure your workflows have the right permissions for the repository, see [Use GITHUB\\\_TOKEN for authentication in workflows](/en/actions/security-for-github-actions/security-guides/automatic-token-authentication). You could use these example steps to download the app created in the previous workflow and publish it on GitHub.

```yaml copy
- uses: actions/checkout@v5
- name: Download release artifact
  uses: actions/download-artifact@v5
  with:
    name: <my-app>
    path: ./<my-app>
- name: Publish built binary to GitHub releases
- run: |
    gh release create --generate-notes ./<my-app>/<my-project>#<my-app>
```
