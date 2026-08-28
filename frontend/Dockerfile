# SvelteKit frontend, self-hosted via @sveltejs/adapter-node.
# Build context is the repo root so the build can resolve the workspace-root
# env dir (svelte.config.js sets env.dir = ".."). Only frontend/ is copied.
FROM node:22-slim AS build
WORKDIR /app/frontend
COPY frontend/ ./

# PUBLIC_WS_URL is consumed at build time by SvelteKit's $env/static/public.
# svelte.config.js reads env from ".." (the repo root), so write it there.
ARG PUBLIC_WS_URL=ws://localhost:8080
RUN echo "PUBLIC_WS_URL=${PUBLIC_WS_URL}" > /app/.env

RUN corepack enable && corepack prepare pnpm@11.9.0 --activate \
    && pnpm install \
    && pnpm build

FROM node:22-slim
WORKDIR /app
ENV NODE_ENV=production
ENV PORT=3000
ENV HOST=0.0.0.0
ENV PROTOCOL_HEADER=x-forwarded-proto
ENV ADDRESS_HEADER=x-forwarded-for

COPY --from=build /app/frontend/build ./build
COPY --from=build /app/frontend/node_modules ./node_modules

EXPOSE 3000
# node:22-slim has no curl; use Node's built-in fetch for the healthcheck.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD node -e "fetch('http://localhost:3000/').then(r => process.exit(r.ok ? 0 : 1), () => process.exit(1))"

CMD ["node", "build"]
