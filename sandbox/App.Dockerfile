# All toolchains live in build stages; the host only needs Docker and Node.
FROM rust:1.98-slim@sha256:4cd829461bd5c4d511c32e269da9cb8929223b666519d8004e35fc8d1d771ab7 AS wasm
WORKDIR /src/core
RUN rustup target add wasm32-unknown-unknown
COPY core .
RUN cargo build --locked -p hoi-wasm --target wasm32-unknown-unknown --profile wasm
RUN cargo install --locked wasm-bindgen-cli --version 0.2.129
RUN wasm-bindgen --target web --out-dir /wasm --out-name hoi_wasm target/wasm32-unknown-unknown/wasm/hoi_wasm.wasm

FROM node:25-bookworm-slim@sha256:81db02c4b671288a03915da9534dbd54f96d0e7c24d80ccc54f5b36b2e684370 AS web
WORKDIR /src
RUN corepack enable && corepack prepare pnpm@10.28.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY apps ./apps
COPY packages ./packages
COPY design ./design
COPY --from=wasm /wasm ./apps/web/src/lib/core/wasm/pkg
RUN pnpm install --frozen-lockfile
RUN pnpm tokens && HOI_SANDBOX=1 pnpm build:web

FROM node:25-bookworm-slim@sha256:81db02c4b671288a03915da9534dbd54f96d0e7c24d80ccc54f5b36b2e684370
WORKDIR /srv
COPY --from=web /src/apps/web/dist /app/dist
COPY sandbox/app-server.mjs /srv/sandbox/app-server.mjs
USER node
EXPOSE 7071 7072 7073
CMD ["node", "/srv/sandbox/app-server.mjs"]
