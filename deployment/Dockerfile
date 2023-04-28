FROM ubuntu:18.04

LABEL maintainer = "Golden Beatles <goldenbeatles@zextras.com>"
LABEL description = "Reposync image"

ENV DEBIAN_FRONTEND=noninteractive
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN \
  apt-get update; \
  apt-get upgrade; \
  apt-get install -y --no-install-recommends ca-certificates libssl1.1 monit; \
  rm -rf /var/lib/apt/lists/*

COPY reposync /usr/local/bin/reposync
COPY monitrc /etc/monitrc
RUN chmod 700 /etc/monitrc
ENTRYPOINT ["monit"]
