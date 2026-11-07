FROM node:22-bookworm-slim@sha256:5ec23b7c622b00547753bc6ffd78085dd868a68aeb41c8e0a1418f033e13d96d
WORKDIR /srv
RUN corepack enable && corepack prepare pnpm@10.28.0 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml ./
COPY packages/tokens ./packages/tokens
COPY design/tokens ./design/tokens
RUN pnpm install --filter @hoi/tokens --prod --frozen-lockfile && pnpm --filter @hoi/tokens build
COPY apps/site/src/styles/site.css ./apps/site/src/styles/site.css
COPY sandbox ./sandbox
EXPOSE 7070
CMD ["node", "/srv/sandbox/server.mjs"]
