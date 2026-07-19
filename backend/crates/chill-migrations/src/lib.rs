//! Immutable, checksummed `PostgreSQL` migrations shared by Chill server binaries.

use std::{borrow::Cow, sync::LazyLock};

use regex::Regex;
use sha2::{Digest, Sha256};
use sqlx::{Acquire, PgPool, Row};
use thiserror::Error;

static MIGRATION_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([0-9]{6})_([a-z0-9_]+)\.sql$")
        .unwrap_or_else(|error| unreachable!("static migration regex is invalid: {error}"))
});

static RUNTIME_ROLE_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-z][a-z0-9-]{1,62}$")
        .unwrap_or_else(|error| unreachable!("static runtime-role regex is invalid: {error}"))
});

static RUNTIME_ROLE_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bchill_app\b")
        .unwrap_or_else(|error| unreachable!("static runtime-role token regex is invalid: {error}"))
});

const WORKER_TABLE_FORCE_RLS: [&str; 2] = [
    "ALTER TABLE product.saved_queries FORCE ROW LEVEL SECURITY;",
    "ALTER TABLE product.alerts FORCE ROW LEVEL SECURITY;",
];
const WORKER_TABLE_OWNER_RLS: [&str; 2] = [
    "ALTER TABLE product.saved_queries NO FORCE ROW LEVEL SECURITY;",
    "ALTER TABLE product.alerts NO FORCE ROW LEVEL SECURITY;",
];
const PROVIDER_SCHEMA_ADMIN_POLICIES: &str =
    include_str!("../sql/provider_schema_admin_policies.sql");

const MIGRATION_SOURCES: &[(&str, &str)] = &[
    (
        "000001_control_plane.sql",
        include_str!("../../../migrations/000001_control_plane.sql"),
    ),
    (
        "000002_tenant_security.sql",
        include_str!("../../../migrations/000002_tenant_security.sql"),
    ),
    (
        "000003_ingest_inbox.sql",
        include_str!("../../../migrations/000003_ingest_inbox.sql"),
    ),
    (
        "000004_canonical_normalization.sql",
        include_str!("../../../migrations/000004_canonical_normalization.sql"),
    ),
    (
        "000005_parquet_lake.sql",
        include_str!("../../../migrations/000005_parquet_lake.sql"),
    ),
    (
        "000006_retention_deletion.sql",
        include_str!("../../../migrations/000006_retention_deletion.sql"),
    ),
    (
        "000007_tenant_access_control.sql",
        include_str!("../../../migrations/000007_tenant_access_control.sql"),
    ),
    (
        "000008_consent_export_workflows.sql",
        include_str!("../../../migrations/000008_consent_export_workflows.sql"),
    ),
    (
        "000009_analytics_workspace.sql",
        include_str!("../../../migrations/000009_analytics_workspace.sql"),
    ),
    (
        "000010_policy_version_bigint.sql",
        include_str!("../../../migrations/000010_policy_version_bigint.sql"),
    ),
    (
        "000011_identity_organization_lookup.sql",
        include_str!("../../../migrations/000011_identity_organization_lookup.sql"),
    ),
];

/// A migration embedded into the server binary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Migration {
    /// Monotonically increasing migration number.
    pub version: i64,
    /// Stable filename recorded in `PostgreSQL`.
    pub name: &'static str,
    /// SQL body applied transactionally.
    pub sql: &'static str,
    /// SHA-256 digest used to reject changes to released migrations.
    pub checksum: [u8; 32],
}

/// Errors raised while validating or applying migrations.
#[derive(Debug, Error)]
pub enum MigrationError {
    /// An embedded migration does not follow the immutable naming contract.
    #[error("invalid migration filename {0:?}")]
    InvalidName(String),
    /// A migration has an invalid or duplicate version.
    #[error("invalid or duplicate migration version {0:06}")]
    InvalidVersion(i64),
    /// A migration body is empty.
    #[error("migration {0:?} is empty")]
    Empty(String),
    /// No migration was embedded in the binary.
    #[error("no migrations found")]
    None,
    /// A provider-managed runtime role name is unsafe or unsupported.
    #[error("invalid provider-managed runtime role {0:?}")]
    InvalidRuntimeRole(String),
    /// A provider-managed runtime role is absent or has privileged attributes.
    #[error("provider-managed runtime role {0:?} is absent or privileged")]
    UnsafeRuntimeRole(String),
    /// A released migration differs from the database ledger.
    #[error(
        "migration {version:06} drifted (database name={database_name:?} checksum={database_checksum}, local name={local_name:?} checksum={local_checksum})"
    )]
    Drift {
        /// Migration version that drifted.
        version: i64,
        /// Name stored in `PostgreSQL`.
        database_name: String,
        /// Digest stored in `PostgreSQL`.
        database_checksum: String,
        /// Name compiled into this binary.
        local_name: String,
        /// Digest compiled into this binary.
        local_checksum: String,
    },
    /// `PostgreSQL` rejected a migration operation.
    #[error("PostgreSQL migration operation failed: {0}")]
    Database(#[from] sqlx::Error),
}

/// Discovers and validates every migration embedded in the Rust backend.
///
/// # Errors
///
/// Returns [`MigrationError`] when an embedded filename, version, SQL body, or
/// version sequence violates the migration contract.
pub fn discover() -> Result<Vec<Migration>, MigrationError> {
    discover_sources(MIGRATION_SOURCES)
}

fn discover_sources(
    sources: &[(&'static str, &'static str)],
) -> Result<Vec<Migration>, MigrationError> {
    let mut migrations = Vec::with_capacity(sources.len());
    for &(name, sql) in sources {
        let captures = MIGRATION_NAME
            .captures(name)
            .ok_or_else(|| MigrationError::InvalidName(name.to_owned()))?;
        let version = captures[1]
            .parse::<i64>()
            .map_err(|_| MigrationError::InvalidName(name.to_owned()))?;
        if version <= 0 {
            return Err(MigrationError::InvalidVersion(version));
        }
        if sql.trim().is_empty() {
            return Err(MigrationError::Empty(name.to_owned()));
        }
        migrations.push(Migration {
            version,
            name,
            sql,
            checksum: Sha256::digest(sql.as_bytes()).into(),
        });
    }
    migrations.sort_unstable_by_key(|migration| migration.version);
    if migrations.is_empty() {
        return Err(MigrationError::None);
    }
    for pair in migrations.windows(2) {
        if pair[0].version == pair[1].version {
            return Err(MigrationError::InvalidVersion(pair[0].version));
        }
    }
    Ok(migrations)
}

/// Applies all pending migrations under the same advisory lock and ledger used
/// by the original backend. Already-applied migrations are checksum-verified.
///
/// # Errors
///
/// Returns [`MigrationError`] when discovery fails, `PostgreSQL` is
/// unavailable, applying SQL fails, or the database ledger reveals drift.
pub async fn migrate(pool: &PgPool) -> Result<Vec<Migration>, MigrationError> {
    migrate_for_runtime_role(pool, None).await
}

/// Applies migrations while granting released `chill_app` permissions to an
/// existing, provider-managed runtime role.
///
/// The source SQL and checksums remain immutable. The fixed `chill_app`
/// identifier is rendered at execution after validating the replacement and
/// proving the existing role cannot bypass tenant isolation. Provider-managed
/// schema owners also receive the narrow `alerts` and `saved_queries` owner
/// exceptions required by the cross-tenant security-definer alert worker; the
/// runtime role remains subject to both tables' tenant policies.
///
/// # Errors
///
/// Returns [`MigrationError`] for an invalid or privileged runtime role, source
/// drift, locking failures, or rejected `PostgreSQL` operations.
pub async fn migrate_for_runtime_role(
    pool: &PgPool,
    runtime_role: Option<&str>,
) -> Result<Vec<Migration>, MigrationError> {
    let migrations = discover()?;
    let runtime_role = runtime_role.map(validate_runtime_role).transpose()?;
    let mut connection = pool.acquire().await?;
    if let Some(role) = runtime_role {
        verify_runtime_role(&mut connection, role).await?;
    }
    sqlx::query("SELECT pg_advisory_lock(hashtext('chill-schema-migrations'))")
        .execute(&mut *connection)
        .await?;

    let mut result = migrate_locked(&mut connection, &migrations, runtime_role).await;
    if result.is_ok()
        && runtime_role.is_some()
        && let Err(error) = sqlx::raw_sql(PROVIDER_SCHEMA_ADMIN_POLICIES)
            .execute(&mut *connection)
            .await
    {
        result = Err(MigrationError::Database(error));
    }
    let unlock = sqlx::query("SELECT pg_advisory_unlock(hashtext('chill-schema-migrations'))")
        .execute(&mut *connection)
        .await;

    match (result, unlock) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(MigrationError::Database(error)),
        (Ok(applied), Ok(_)) => Ok(applied),
    }
}

async fn migrate_locked(
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    migrations: &[Migration],
    runtime_role: Option<&str>,
) -> Result<Vec<Migration>, MigrationError> {
    sqlx::query(
        r"
        CREATE TABLE IF NOT EXISTS public.chill_schema_migrations (
            version bigint PRIMARY KEY,
            name text NOT NULL UNIQUE,
            checksum bytea NOT NULL CHECK (octet_length(checksum) = 32),
            applied_at timestamptz NOT NULL DEFAULT clock_timestamp()
        )
        ",
    )
    .execute(&mut **connection)
    .await?;

    let mut applied = Vec::new();
    for migration in migrations {
        let existing = sqlx::query(
            "SELECT name, checksum FROM public.chill_schema_migrations WHERE version = $1",
        )
        .bind(migration.version)
        .fetch_optional(&mut **connection)
        .await?;
        if let Some(row) = existing {
            let name: String = row.try_get("name")?;
            let checksum: Vec<u8> = row.try_get("checksum")?;
            if name != migration.name || checksum.as_slice() != migration.checksum {
                return Err(MigrationError::Drift {
                    version: migration.version,
                    database_name: name,
                    database_checksum: hex::encode(checksum),
                    local_name: migration.name.to_owned(),
                    local_checksum: hex::encode(migration.checksum),
                });
            }
            continue;
        }

        let mut transaction = connection.begin().await?;
        let sql = render_migration_sql(migration.sql, runtime_role);
        // The only dynamic token is a regex-constrained PostgreSQL role name,
        // rendered as a quoted literal or identifier above.
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql.as_ref()))
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO public.chill_schema_migrations (version, name, checksum) VALUES ($1, $2, $3)",
        )
        .bind(migration.version)
        .bind(migration.name)
        .bind(migration.checksum.as_slice())
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        applied.push(migration.clone());
    }
    Ok(applied)
}

fn validate_runtime_role(role: &str) -> Result<&str, MigrationError> {
    if RUNTIME_ROLE_NAME.is_match(role) {
        Ok(role)
    } else {
        Err(MigrationError::InvalidRuntimeRole(role.to_owned()))
    }
}

async fn verify_runtime_role(
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    role: &str,
) -> Result<(), MigrationError> {
    let attributes = sqlx::query_as::<_, (bool, bool, bool, bool, bool, bool)>(
        "SELECT rolcanlogin, rolsuper, rolcreaterole, rolcreatedb, rolreplication, rolbypassrls \
         FROM pg_roles WHERE rolname = $1",
    )
    .bind(role)
    .fetch_optional(&mut **connection)
    .await?;
    match attributes {
        Some((true, false, false, false, false, false)) => Ok(()),
        _ => Err(MigrationError::UnsafeRuntimeRole(role.to_owned())),
    }
}

fn render_migration_sql<'a>(sql: &'a str, runtime_role: Option<&str>) -> Cow<'a, str> {
    let Some(role) = runtime_role else {
        return Cow::Borrowed(sql);
    };
    let quoted_literal = format!("'{role}'");
    let quoted_identifier = format!("\"{role}\"");
    let owner_rls_rendered = WORKER_TABLE_FORCE_RLS
        .iter()
        .zip(WORKER_TABLE_OWNER_RLS)
        .fold(sql.to_owned(), |rendered, (from, to)| {
            rendered.replace(from, to)
        });
    let literals_rendered = owner_rls_rendered.replace("'chill_app'", &quoted_literal);
    Cow::Owned(
        RUNTIME_ROLE_TOKEN
            .replace_all(&literals_rendered, quoted_identifier.as_str())
            .into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_migrations_in_version_order() {
        let result = discover_sources(&[
            ("000002_second.sql", "SELECT 2;\n"),
            ("000001_first.sql", "SELECT 1;\n"),
        ]);
        assert!(result.is_ok(), "migration discovery failed: {result:?}");
        let migrations = result.unwrap_or_default();
        assert_eq!(migrations.len(), 2);
        assert_eq!(migrations[0].version, 1);
        assert_eq!(migrations[0].name, "000001_first.sql");
        assert_ne!(migrations[0].checksum, [0; 32]);
    }

    #[test]
    fn rejects_invalid_migration_sources() {
        assert!(discover_sources(&[("1_bad.sql", "SELECT 1;")]).is_err());
        assert!(discover_sources(&[("000001_empty.sql", " \n")]).is_err());
        assert!(
            discover_sources(&[
                ("000001_first.sql", "SELECT 1;"),
                ("000001_second.sql", "SELECT 2;"),
            ])
            .is_err()
        );
        assert!(discover_sources(&[]).is_err());
    }

    #[test]
    fn released_migration_checksums_are_immutable() {
        let expected = [
            (
                "000001_control_plane.sql",
                "604e9815085d423c914bc57dc7dca1feca36c99f779a6ecfaf2bcaea37e0cbf8",
            ),
            (
                "000002_tenant_security.sql",
                "b0fc670596a53f80547082154945f5d4227674341ce7572f62524773b0506fcc",
            ),
            (
                "000003_ingest_inbox.sql",
                "1deb34006a23ef550a8834387e44057c6e35638af3b35d3af74d8e612010ada6",
            ),
            (
                "000004_canonical_normalization.sql",
                "1ad28edc0bfadd725d7d4a0c94e784704d2f8a447cbe766ba83a2070244f29e3",
            ),
            (
                "000005_parquet_lake.sql",
                "b3d601de33b9f03ba832946aace457751074c8c90d561e817d779f51f71ac604",
            ),
            (
                "000006_retention_deletion.sql",
                "eea4dcf6f80ec2085dcbe36b69121581d5f79b3ea88a622d55159fa46d65e2d8",
            ),
            (
                "000007_tenant_access_control.sql",
                "adcc8394a2a5590d5e4f2345c8dc75caf173eaa120507c854d968057ead208ab",
            ),
            (
                "000008_consent_export_workflows.sql",
                "b7735752ee3f7791c581794a1e0b2278c1fcb88b357b41e68b948116f40e8434",
            ),
            (
                "000009_analytics_workspace.sql",
                "b86898abc63c64b7401acbf43c1e3b6a33432563afdcca44070003706a095500",
            ),
            (
                "000010_policy_version_bigint.sql",
                "91203dbd11f69f3e64e4a0ee3b1672dcddd62fed178e29dadf38d55985eb1b2b",
            ),
            (
                "000011_identity_organization_lookup.sql",
                "e3d9184e0a035bdfd127e5edd33a19da5bf8de010f668d740de41599276797d7",
            ),
        ];
        let result = discover();
        assert!(result.is_ok(), "migration discovery failed: {result:?}");
        let migrations = result.unwrap_or_default();
        assert_eq!(migrations.len(), expected.len());
        for (migration, (name, checksum)) in migrations.iter().zip(expected) {
            assert_eq!(migration.name, name);
            assert_eq!(hex::encode(migration.checksum), checksum);
        }
    }

    #[test]
    fn provider_role_rendering_is_narrow_and_quoted() {
        let source = "SELECT 'chill_app'; GRANT SELECT TO chill_app; SELECT chill_application;";
        let rendered = render_migration_sql(source, Some("chill-app"));
        assert_eq!(
            rendered,
            "SELECT 'chill-app'; GRANT SELECT TO \"chill-app\"; SELECT chill_application;"
        );
        assert_eq!(render_migration_sql(source, None), source);
        assert!(validate_runtime_role("chill-app").is_ok());
        for invalid in ["chill_app", "Chill-app", "chill.app", "chill-app;drop"] {
            assert!(validate_runtime_role(invalid).is_err());
        }
    }

    #[test]
    fn provider_rendering_preserves_alert_tenant_rls_for_runtime_role() {
        let source = concat!(
            "ALTER TABLE product.saved_queries FORCE ROW LEVEL SECURITY;\n",
            "ALTER TABLE product.alerts FORCE ROW LEVEL SECURITY;\n",
            "GRANT SELECT ON product.alerts TO chill_app;"
        );
        let rendered = render_migration_sql(source, Some("chill-app"));
        assert!(rendered.contains("saved_queries NO FORCE ROW LEVEL SECURITY"));
        assert!(rendered.contains("alerts NO FORCE ROW LEVEL SECURITY"));
        assert!(rendered.contains("TO \"chill-app\""));
        for forced in WORKER_TABLE_FORCE_RLS {
            assert!(!rendered.contains(forced));
        }
        assert_eq!(render_migration_sql(source, None), source);
    }
}
