FROM ubuntu:18.04

LABEL maintainer = "Davide Baldo <davide.baldo@zextras.com>"
LABEL description = "Reposync image"

COPY reposync /usr/local/bin/reposync

RUN apt-get update \
  && apt-get install -y --no-install-recommends libssl1.1 \
  && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["/usr/local/bin/reposync"]
