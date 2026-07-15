# Security Policy

BuildLens holds sensitive data - GitHub OAuth tokens, webhook secrets, build logs
- so security reports are taken seriously. Thank you for helping keep it safe.

## Supported versions

BuildLens is pre-1.0 and under active development. Security fixes are applied to
the `main` branch only. There are no long-term support branches yet.

## Reporting a vulnerability

**Please do not report security issues in public GitHub issues, pull requests, or
discussions.** Public disclosure before a fix puts users at risk.

Instead, use one of these private channels:

1. **GitHub private vulnerability reporting (preferred).** On the repository, go to
   the **Security** tab → **Report a vulnerability**. This opens a private advisory
   visible only to the maintainers.
2. **Email.** If private reporting is unavailable, email the maintainer at
   **narcissekabongongandu@gmail.com** with `SECURITY` in the subject line.

Please include, as far as you can:

- A description of the issue and its impact.
- Steps to reproduce, or a proof-of-concept.
- The affected service (`gateway`, `analytics`, `ai-worker`) and commit/branch.
- Any suggested remediation.

**What to expect:** an acknowledgement within a few days, an assessment of
severity and scope, and coordination on a fix and disclosure timeline. Because
this is a volunteer, pre-1.0 project, please allow reasonable time before any
public disclosure. Credit is happily given to reporters who want it.

## Scope

In scope: authentication and session handling, the GitHub OAuth/token flow,
webhook signature verification, the per-service database privilege boundary,
secret handling, and anything that could leak another tenant's data.

Out of scope: issues that require a pre-compromised host or already-leaked
credentials; the intentionally insecure **local-development defaults** in
`docker-compose.yml` and `.env.example` (all-zero encryption key, shared dev
passwords) - these are for local use only and must be replaced in any real
deployment; and vulnerabilities in third-party dependencies (report those
upstream, though we welcome a heads-up).

## Security posture

Some safeguards already built into BuildLens, for context:

- **GitHub OAuth tokens are encrypted** (AES-256-GCM) before storage; API tokens
  are stored as SHA-256 hashes.
- **Sessions are opaque and server-side** (Redis-backed, httpOnly cookie,
  revocable) - not JWTs.
- **Webhook deliveries are verified** with an HMAC-SHA256 signature over the raw
  body before the payload is parsed.
- **Per-service Postgres roles** enforce least privilege: each service can write
  only the tables it owns, and nobody can edit the audit log.
- **The AI worker redacts secrets** and sends only bounded excerpts to the model -
  never raw log archives or repository source.
- `.env` is gitignored; real secrets are never committed.

## Handling secrets responsibly

If you believe a secret (API key, token, password) has been committed to the
repository or exposed in logs, report it privately as above - do not post it in a
public issue. Rotate any exposed credential immediately.
