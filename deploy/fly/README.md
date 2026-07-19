# Chill on Fly.io

This is a provider example, not a ready-to-run production configuration. Copy
`fly.toml`, choose globally unique resource names, replace every
`replace-with-...` and `example.com` value, and review the resulting
configuration before deploying.

The example keeps the client-only console on a separate origin and runs only
the Rust `chilld` process on a Fly Machine. PostgreSQL can be provided by Fly
Managed Postgres and immutable lake objects can be stored in a private
S3-compatible bucket such as Tigris. No MinIO, Caddy, or console container runs
on the application Machine.

## Resource layout

- Fly app: a globally unique name, region near the database, 1 shared CPU and
  2 GiB RAM as a starting point
- Managed Postgres: a supported PostgreSQL release and plan sized for the
  workload
- Postgres database: an operator-selected database
- Migration user: `chill-migrator` with the Managed Postgres schema-admin role
- Runtime user: `chill-app` with the Managed Postgres writer role
- Private S3-compatible bucket: an operator-selected bucket
- Browser origin: the exact HTTPS origin hosting the console

The runtime receives the pooled writer URL as `CHILL_DATABASE_URL`. Fly release
Machines receive a separate pooled schema-admin URL as
`CHILL_ADMIN_DATABASE_URL` and run the immutable migration ledger before an
application rollout. Managed Postgres defaults to session pooling, which
preserves Chill's advisory migration lock. Replace the migration URL with the
provider's direct URL before enabling transaction pool mode. Both Rust pools
recycle connections after ten minutes and close idle connections after five,
matching the managed proxy's connection-lifetime guidance. The
`chill-app` role is created through Managed Postgres before migrations so the
released tenant-security migration grants the existing managed role its narrow
Chill permissions instead of trying to create a PostgreSQL role itself.
The provider renderer also leaves `product.alerts` and `product.saved_queries`
owner-bypassable solely so the security-definer alert worker can claim,
evaluate, and complete due alerts across tenants; `chill-app` owns neither
table and remains subject to both tenant RLS policies.
After the immutable ledger is current, the migration runner creates an explicit
policy for the provider's non-login `schema_admin` group on forced-RLS tables
that group owns. This permits first-tenant bootstrap and administrative checks
without giving the login-capable `chill-app` role any cross-tenant access.

An object-storage provider may provision `AWS_ACCESS_KEY_ID` and
`AWS_SECRET_ACCESS_KEY` directly as Fly app secrets. The remaining
S3-compatible settings and the exact console CORS origin are non-secret values
in `fly.toml`. If the optional Sites identity adapter is used,
`CHILL_SITES_AUTH_SECRET` is a dedicated random secret shared only by the Rust
API and the private worker; it must never be exposed to browser JavaScript.

## Deploy and operate

The builder cooks locked Cargo dependencies in a manifest-keyed Docker layer
before copying application source. Fly/Depot can therefore reuse DuckDB and
the rest of the dependency graph across source-only releases without relying
on an ephemeral BuildKit cache mount.

Validate, build, and push an immutable image from the repository root. The
image build is separate from the release so a failed migration or health check
never triggers another Rust build:

```sh
flyctl config validate -c fly.toml
FLY_APP="replace-with-your-chill-app"
IMAGE_LABEL="commit-$(git rev-parse --short=12 HEAD)"
flyctl deploy -c fly.toml --remote-only --build-only --push \
  --image-label "$IMAGE_LABEL" \
  --build-arg CARGO_BUILD_JOBS=4 \
  --build-arg CHILL_COMMIT="$(git rev-parse HEAD)" \
  --build-arg CHILL_BUILD_TIME="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
flyctl deploy -c fly.toml \
  --image "registry.fly.io/$FLY_APP:$IMAGE_LABEL"
curl --fail "https://$FLY_APP.fly.dev/readyz"
curl --fail "https://$FLY_APP.fly.dev/version"
```

The runtime image intentionally uses Docker `CMD`, not `ENTRYPOINT`, so Fly's
release command can replace the default server process with `chillctl migrate`.

Run privileged administration in an ephemeral Machine. Override the database
URL so administration uses the direct schema-admin connection:

```sh
flyctl ssh console -a "$FLY_APP" -C \
  'sh -ec '\''export CHILL_DATABASE_URL="$CHILL_ADMIN_DATABASE_URL"; chillctl verify'\'''
```

Bootstrap prints the SDK key exactly once. `issue-verified-session` likewise
prints a short-lived `ch_us_` credential exactly once and only after an operator
has verified the mapped identity outside Chill. Keep both out of logs and shell
history. The canonical bootstrap schema is available inside the image at
`/usr/local/share/chill/envelope.schema.json`.

Managed Postgres owns database backups, recovery, failover, encryption, and
connection pooling. Tigris owns replicated object durability. Periodically
verify both recovery paths through the provider controls; the single-node
backup and restore scripts do not apply to this topology.
