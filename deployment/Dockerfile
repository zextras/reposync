FROM rust:1.93-alpine AS builder
LABEL maintainer = "Golden Beatles <goldenbeatles@zextras.com>"
LABEL description = "Reposync image"

RUN \
  apk add --no-cache musl-dev openssl-dev pkgconf cmake make perl gcc

COPY . /project
WORKDIR /project
RUN \
  cargo build --release

FROM alpine:latest
COPY --from=builder /project/target/release/reposync /usr/bin/reposync
RUN \
  apk add --no-cache ca-certificates libgcc openssl
ENTRYPOINT ["reposync"]
