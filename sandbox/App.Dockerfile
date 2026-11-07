# All toolchains live in build stages; the host only needs Docker and Node.
FROM rust:1.97-slim AS wasm
WORKDIR /src/core
RUN rustup target add wasm32-unknown-unknown
COPY core .
RUN cargo build --locked -p hoi-wasm --target wasm32-unknown-unknown --profile wasm
RUN cargo install --locked wasm-bindgen-cli --version 0.2.129
RUN wasm-bindgen --target web --out-dir /wasm --out-name hoi_wasm target/wasm32-unknown-unknown/wasm/hoi_wasm.wasm

FROM node:22-bookworm-slim AS web
WORKDIR /src
RUN corepack enable && corepack prepare pnpm@10.28.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY apps ./apps
COPY packages ./packages
COPY design ./design
COPY --from=wasm /wasm ./apps/web/src/lib/core/wasm/pkg
RUN pnpm install --frozen-lockfile
RUN pnpm tokens && HOI_SANDBOX=1 pnpm build:web

FROM node:22-bookworm-slim
WORKDIR /srv
COPY --from=web /src/apps/web/dist /app/dist
COPY sandbox/app-server.mjs /srv/sandbox/app-server.mjs
USER node
EXPOSE 7071 7072 7073
CMD ["node", "/srv/sandbox/app-server.mjs"]
