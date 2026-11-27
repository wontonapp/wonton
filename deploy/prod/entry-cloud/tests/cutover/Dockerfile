# Same nginx/python as the production prerequisite (Ubuntu 24.04 apt).
FROM ubuntu:24.04
RUN apt-get update -qq \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends \
      nginx python3 iproute2 curl openssl ca-certificates \
 && rm -rf /var/lib/apt/lists/* /etc/nginx/sites-enabled/default
