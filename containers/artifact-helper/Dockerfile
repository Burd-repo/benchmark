ARG BURD_RUST_BUILDER_IMAGE
FROM ${BURD_RUST_BUILDER_IMAGE} AS builder

WORKDIR /src
COPY crates/burd-artifact-helper/src/main.rs /src/main.rs
RUN rustc --edition=2024 -C opt-level=3 -C strip=symbols \
        -o /burd-artifact-helper /src/main.rs

FROM scratch
COPY --from=builder /burd-artifact-helper /burd-artifact-helper
COPY --chown=1000:1000 containers/artifact-helper/rootfs/burd/staging/.burd-placeholder /burd/staging/.burd-placeholder
ENTRYPOINT ["/burd-artifact-helper"]
