# Packaging-only image. The binary comes from the pipeline's `Build` stage
# (`cargo build --release`), which runs earlier in the same pod and shares the
# workspace volume with the buildah container, so there is nothing to compile
# here.
#
# This replaces an in-image `cargo build --release`. Compiling the dependency
# tree inside the buildah container put its peak RSS under that container's
# cgroup limit while `nproc` still reported the whole node, and the build was
# OOM-killed on the aws-sdk crates (0.13.0 #2, exit 137). Packaging the
# artifact removes the second compile entirely rather than trying to fit it.
#
# Consequence: this Dockerfile is not standalone. Run `cargo build --release`
# (or `make docker-build`, which does it for you) before building it locally.
#
# The base must ship a glibc at least as new as the `rust-v1` agent's, because
# the copied binary is dynamically linked against it. A musl-static binary also
# runs here, so this base is correct whichever libc the agent uses. `openssl` is
# `features = ["vendored"]` and `aws-lc-sys` is static, so no TLS library is
# needed at runtime.
FROM debian:trixie-slim

LABEL maintainer="Golden Beatles <goldenbeatles@zextras.com>"
LABEL description="Reposync image"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates vi \
    && rm -rf /var/lib/apt/lists/*

# Built for linux/amd64. dockerStage() is not passed `platforms`, so it builds
# amd64 only; adding another platform requires cross-compiling in the Build
# stage instead of copying one artifact.
COPY target/release/reposync /usr/bin/reposync

# Fail the image build on a libc or architecture mismatch, rather than
# publishing an image that dies on first start.
RUN /usr/bin/reposync --version

ENTRYPOINT ["reposync"]
