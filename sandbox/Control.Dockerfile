FROM node:22-bookworm-slim
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
