FROM node:24-slim@sha256:01d566e8d94d8c619591d78652babfccd3c6992a3dffa9ed5ab39e11cf6c70e9
WORKDIR /app
COPY --chown=node:node release.json ./release.json
COPY --chown=node:node scripts/release-config.mjs scripts/release-schema.mjs ./scripts/
COPY --chown=node:node registry/server.mjs registry/protocol.mjs registry/store.mjs registry/admin.mjs ./registry/
RUN mkdir -p /data && chown node:node /data
USER node
# Set HOI_REGISTRY_URL to the public https:// URL carriers announce to; without it, a registry
# listening on 0.0.0.0 takes no node announcements. The operator's tool runs in the container:
#   docker exec hoi-registry node registry/admin.mjs list
ENV HOI_REGISTRY_HOST=0.0.0.0 \
    HOI_REGISTRY_PORT=7845 \
    HOI_REGISTRY_DB=/data/community-names.db
EXPOSE 7845
VOLUME ["/data"]
CMD ["node", "registry/server.mjs"]
