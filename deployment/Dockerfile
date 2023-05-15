FROM alpine:latest AS builder
LABEL maintainer = "Golden Beatles <goldenbeatles@zextras.com>"
LABEL description = "Reposync image"

RUN \
  apk add --no-cache cargo monit openssl-dev pkgconf

COPY . /project
WORKDIR /project
RUN \
  cargo build --release

FROM alpine:latest
COPY --from=builder /project/target/release/reposync /usr/bin/reposync
COPY --from=builder /project/deployment/monitrc /etc/monitrc
RUN chmod 700 /etc/monitrc
RUN \
  apk add --no-cache ca-certificates libgcc monit openssl
ENTRYPOINT ["monit"]
