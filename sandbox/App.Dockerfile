# All toolchains live in build stages; the host only needs Docker and Node.
FROM rust:1.99-slim@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7 AS wasm
WORKDIR /src/core
RUN rustup target add wasm32-unknown-unknown
COPY core .
RUN cargo build --locked -p hoi-wasm --target wasm32-unknown-unknown --profile wasm
RUN cargo install --locked wasm-bindgen-cli --version 0.2.129
RUN wasm-bindgen --target web --out-dir /wasm --out-name hoi_wasm target/wasm32-unknown-unknown/wasm/hoi_wasm.wasm

FROM node:26-bookworm-slim@sha256:5df79989ed685e09f0a98019db4cebfb22fb9df1a53c0161664a011f7ff6b0f1 AS web
WORKDIR /src
RUN corepack enable && corepack prepare pnpm@10.28.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY apps ./apps
COPY packages ./packages
COPY design ./design
COPY --from=wasm /wasm ./apps/web/src/lib/core/wasm/pkg
RUN pnpm install --frozen-lockfile
RUN pnpm tokens && HOI_SANDBOX=1 pnpm build:web

FROM node:26-bookworm-slim@sha256:5df79989ed685e09f0a98019db4cebfb22fb9df1a53c0161664a011f7ff6b0f1
WORKDIR /srv
COPY --from=web /src/apps/web/dist /app/dist
COPY sandbox/app-server.mjs /srv/sandbox/app-server.mjs
USER node
EXPOSE 7071 7072 7073
CMD ["node", "/srv/sandbox/app-server.mjs"]
