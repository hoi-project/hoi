# The observation package contains real clients and services; production builds never import it.
FROM rust:1.98-slim-bookworm@sha256:ff521445a372125ed4f76e1453a1f8098f2d05332d1601d30db1c1f62757e730 AS core
WORKDIR /src/core
COPY core .
COPY release.json /src/release.json
COPY vendor /src/vendor
RUN cargo build --release --locked -p hoi-node
RUN rustup target add wasm32-unknown-unknown && cargo build --locked -p hoi-wasm --target wasm32-unknown-unknown --profile wasm
RUN cargo install --locked wasm-bindgen-cli --version 0.2.129
RUN wasm-bindgen --target web --out-dir /wasm --out-name hoi_wasm target/wasm32-unknown-unknown/wasm/hoi_wasm.wasm

FROM node:25-bookworm-slim@sha256:81db02c4b671288a03915da9534dbd54f96d0e7c24d80ccc54f5b36b2e684370
WORKDIR /srv
# Node 25 no longer bundles Corepack; install the workspace's pinned package manager.
RUN npm install --global pnpm@10.28.0
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json publication.json release.json ./
COPY apps ./apps
COPY packages ./packages
COPY design ./design
COPY vendor ./vendor
COPY scripts ./scripts
COPY registry ./registry
COPY relay ./relay
COPY deploy ./deploy
COPY --from=core /wasm ./apps/web/src/lib/core/wasm/pkg
COPY --from=core /src/core/target/release/hoi-node /usr/local/bin/hoi-node
ENV PLAYWRIGHT_BROWSERS_PATH=/ms-playwright
RUN pnpm install --frozen-lockfile && pnpm tokens && pnpm exec playwright install --with-deps chromium
COPY sandbox ./sandbox
# Vite serves the unmodified app only to the internal managed browser clients.
RUN mkdir -p /data /srv/apps/web/node_modules/.vite /srv/apps/web/node_modules/.vite-temp && chown -R node:node /srv /data /ms-playwright
USER node
EXPOSE 7080
CMD ["node", "sandbox/walkthrough-server.mjs"]
