# Burd Artifact Helper Image

This image is the trusted, network-disabled bridge between private Agent files
and Docker-managed artifact volumes. Production configuration must use the
repository digest (`repository@sha256:...`) of a reviewed build.

The final `scratch` image contains a statically linked helper. The build stage
is intentionally supplied by the operator, must support
`target-feature=+crt-static`, and must itself be digest-pinned:

```text
docker build --build-arg BURD_RUST_BUILDER_IMAGE=rust:<version>@sha256:<digest> --tag burd/artifact-helper:test --file containers/artifact-helper/Dockerfile .
```

The helper accepts only the fixed `import`, `export`, and local gate
`roundtrip-test` operations. It has no HTTP client, shell, package manager, CA
bundle, or customer-controlled path argument. The runtime launches it with no
network, no-new-privileges, bounded resources, and Docker-managed volumes only.
All default capabilities are dropped; only the import operation receives
`DAC_READ_SEARCH` so it can read private files staged by `docker cp`. The helper
sees no host bind path. Export receives no added capability.

The normal CI gate builds the already-compiled Linux helper without a base
image and runs a real offline roundtrip:

```text
RUSTFLAGS="-C target-feature=+crt-static" cargo build --locked -p burd-artifact-helper
docker build --file containers/artifact-helper/Dockerfile.test --tag burd/artifact-helper:test .
```

`Dockerfile.test` is only a packaging fixture. Release builds use `Dockerfile`
with a digest-pinned Rust builder and must publish the resulting helper under a
repository digest before provider configuration references it. The root
`.dockerignore` sends only the helper source, packaging files, and explicit test
binary to the Docker daemon; repository state and private local files are not
part of the build context.
