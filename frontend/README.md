# BuildLens frontend

The Phase 7 web application is a Next.js 16 App Router frontend. It renders the
BuildLens dashboard from authenticated gateway APIs and does not connect directly
to Postgres, RabbitMQ, or MinIO.

## Local development

Run the gateway and its dependencies first, then:

```bash
npm ci
npm run dev
```

The application listens on `http://localhost:3000`. Server Components call the
gateway at `GATEWAY_INTERNAL_URL`, which defaults to `http://localhost:8080`.
Browser-facing OAuth and logout links use `GATEWAY_PUBLIC_URL`.
The gateway redirects completed login and logout flows to its `FRONTEND_URL`.

## Checks

```bash
npm run check
npm run build
npm audit
```

Dashboard reads are server-side and forward only the opaque BuildLens session
cookie. API responses are narrow view models produced by the gateway after its
existing organization membership checks.
