FROM ubuntu:18.04

LABEL maintainer = "Davide Baldo <davide.baldo@zextras.com>"
LABEL description = "Reposync image"

RUN apt-get update \
  && apt-get install -y --no-install-recommends libssl1.1 ca-certificates \
  && rm -rf /var/lib/apt/lists/*

COPY reposync /usr/local/bin/reposync
ENTRYPOINT ["/usr/local/bin/reposync"]
