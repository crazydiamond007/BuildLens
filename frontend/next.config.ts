import type { NextConfig } from "next";

// The gateway is a private service; the browser never reaches it directly. These
// two paths are the exception - the OAuth sign-in/callback/setup dance and the
// GitHub App webhook - and they are proxied through the frontend's own origin so
// the gateway's host-only session cookie is set on, and sent back to, the same
// domain the app runs on. Everything else the gateway serves (/me,
// /organizations/*, ...) is fetched server-side over this same internal URL by
// the Server Components in src/lib/api.ts, never through these rewrites.
//
// On Railway this is http://gateway.railway.internal:8080; in the dev/compose
// stack the browser talks to the gateway directly, so these rewrites lie dormant.
const gatewayInternalUrl =
  process.env.GATEWAY_INTERNAL_URL ?? "http://localhost:8080";

const nextConfig: NextConfig = {
  output: "standalone",
  async rewrites() {
    return {
      beforeFiles: [
        {
          source: "/auth/:path*",
          destination: `${gatewayInternalUrl}/auth/:path*`,
        },
        {
          source: "/webhooks/:path*",
          destination: `${gatewayInternalUrl}/webhooks/:path*`,
        },
      ],
    };
  },
};

export default nextConfig;
