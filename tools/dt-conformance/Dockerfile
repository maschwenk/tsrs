FROM node:24.20.0-bookworm-slim AS node
FROM ubuntu:24.04
COPY --from=node /usr/local/ /usr/local/
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates git python3 libatomic1 \
    && npm install --global --ignore-scripts corepack@0.36.0 \
    && rm -rf /var/lib/apt/lists/* /root/.npm
