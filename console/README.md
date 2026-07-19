# Chill Console

The Chill Console is a React application for configuring the Rust control
plane. Its narrow Sites worker exchanges the private site's authenticated
email for a short-lived Rust user session. It has no database, D1/R2 binding,
or backend state of its own.

## Development

Requires Node.js `>=22.13.0`.

```sh
npm ci
npm run dev
```

Production validation:

```sh
npm run lint
npm test
npm run build
```

The production artifact is the `dist/` directory. The hosted worker requires
server-only `CHILL_API_ORIGIN`, `CHILL_SITES_AUTH_SECRET`, and
`CHILL_CONSOLE_SDK_KEY` runtime values. The SDK key must have only the OTLP
ingestion scope needed by the console.
For local or recovery access, start `chilld` with an exact
`CHILL_CONSOLE_ORIGIN` allowlist entry and use the connection screen.

## Authentication and storage

The private hosted site automatically exchanges the platform-authenticated
email for a short-lived `ch_us_` credential. The connection screen remains as
an operator recovery path. The console stores the credential only in memory
and `sessionStorage`; it is cleared when the tab is closed or the user signs
out. It is never placed in `localStorage`, cookies, URLs, or telemetry. The
server-to-server exchange secret is available only to the Sites worker and
Rust API.

After the Rust API validates the console session, the console dogfoods the
real `@chill-observability/browser` SDK. It records session and page lifecycle,
navigation destinations, refreshes, and control-plane action outcomes. The
browser posts to the same-origin `/v1/logs` relay with an inert marker; the
worker discards that marker and injects `CHILL_CONSOLE_SDK_KEY` server-side.
Email, project/environment IDs, credentials, form values, API response bodies,
and replay content are not captured.

The API origin and current project/environment selection also use
`sessionStorage`. Chill configuration remains exclusively in PostgreSQL behind
the Rust API.

## Supported workflows

- Read the tenant-scoped console overview and current role capabilities.
- Create projects and environments.
- Select projects and environments.
- Register data sources.
- Issue, rotate, and revoke SDK keys with one-time credential disclosure.
- Register versioned behavior schemas.
- Activate privacy and sampling policy versions.
- Update environment retention.

Mutating controls are disabled when the current role lacks the required
`control:write` or `credentials:manage` capability.
