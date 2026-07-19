# syntax=docker/dockerfile:1.7

ARG NODE_IMAGE=node:22.23.1-alpine3.24@sha256:16e22a550f3863206a3f701448c45f7912c6896a62de43add43bb9c86130c3e2
ARG CADDY_IMAGE=caddy:2.11.4-alpine@sha256:5f5c8640aae01df9654968d946d8f1a56c497f1dd5c5cda4cf95ab7c14d58648

FROM ${NODE_IMAGE} AS build
WORKDIR /src
COPY console/package.json console/package-lock.json ./
RUN --mount=type=cache,target=/root/.npm npm ci
COPY console/ ./
RUN npm run build

FROM ${CADDY_IMAGE}
COPY console/Caddyfile /etc/caddy/Caddyfile
COPY --from=build --chown=65532:65532 /src/dist/ /srv/
COPY --chown=65532:65532 LICENSE NOTICE /usr/local/share/licenses/chill/
USER 65532:65532
EXPOSE 8080
