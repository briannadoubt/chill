CREATE SCHEMA lake;

GRANT USAGE ON SCHEMA lake TO chill_app;

CREATE TABLE lake.export_batches (
  batch_id text PRIMARY KEY,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  batch_kind text NOT NULL,
  partition_day date NOT NULL,
  partition_hour smallint NOT NULL,
  envelope_kind text NOT NULL,
  schema_version text NOT NULL DEFAULT '1',
  status text NOT NULL DEFAULT 'pending',
  attempt_count integer NOT NULL DEFAULT 0,
  available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  lease_owner text,
  lease_expires_at timestamptz,
  object_key text,
  manifest_key text,
  object_sha256 bytea,
  manifest_sha256 bytea,
  byte_count bigint,
  row_count integer NOT NULL,
  min_server_received_at_unix_nano numeric(20, 0) NOT NULL,
  max_server_received_at_unix_nano numeric(20, 0) NOT NULL,
  min_effective_occurred_at_unix_nano numeric(20, 0) NOT NULL,
  max_effective_occurred_at_unix_nano numeric(20, 0) NOT NULL,
  last_error_code text,
  last_error_message text,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  published_at timestamptz,
  superseded_at timestamptz,
  CONSTRAINT lake_batch_id_format CHECK (batch_id ~ '^[0-9a-f]{64}$'),
  CONSTRAINT lake_batch_kind_value CHECK (batch_kind IN ('micro', 'compaction')),
  CONSTRAINT lake_partition_hour_bound CHECK (partition_hour BETWEEN 0 AND 23),
  CONSTRAINT lake_envelope_kind_value CHECK (
    envelope_kind IN ('behavior', 'otel.log', 'otel.span', 'otel.metric', 'replay')
  ),
  CONSTRAINT lake_schema_version_value CHECK (schema_version = '1'),
  CONSTRAINT lake_status_value CHECK (
    status IN ('pending', 'leased', 'committed', 'superseded', 'dead_letter')
  ),
  CONSTRAINT lake_attempt_count_bound CHECK (attempt_count BETWEEN 0 AND 1000),
  CONSTRAINT lake_lease_state CHECK (
    (status = 'leased') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
  ),
  CONSTRAINT lake_lease_owner_bound CHECK (
    lease_owner IS NULL OR char_length(lease_owner) BETWEEN 1 AND 128
  ),
  CONSTRAINT lake_row_count_bound CHECK (row_count BETWEEN 1 AND 1000000),
  CONSTRAINT lake_time_bounds CHECK (
    min_server_received_at_unix_nano BETWEEN 0 AND 18446744073709551615
    AND max_server_received_at_unix_nano BETWEEN min_server_received_at_unix_nano AND 18446744073709551615
    AND min_effective_occurred_at_unix_nano BETWEEN 0 AND 18446744073709551615
    AND max_effective_occurred_at_unix_nano BETWEEN min_effective_occurred_at_unix_nano AND 18446744073709551615
  ),
  CONSTRAINT lake_object_key_bound CHECK (
    object_key IS NULL OR (
      char_length(object_key) BETWEEN 1 AND 1024
      AND object_key !~ '(^|/)\.\.(/|$)'
      AND left(object_key, 1) <> '/'
    )
  ),
  CONSTRAINT lake_manifest_key_bound CHECK (
    manifest_key IS NULL OR (
      char_length(manifest_key) BETWEEN 1 AND 1024
      AND manifest_key !~ '(^|/)\.\.(/|$)'
      AND left(manifest_key, 1) <> '/'
    )
  ),
  CONSTRAINT lake_digest_sizes CHECK (
    (object_sha256 IS NULL OR octet_length(object_sha256) = 32)
    AND (manifest_sha256 IS NULL OR octet_length(manifest_sha256) = 32)
  ),
  CONSTRAINT lake_error_bounds CHECK (
    (last_error_code IS NULL OR last_error_code ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$')
    AND (last_error_message IS NULL OR char_length(last_error_message) BETWEEN 1 AND 512)
  ),
  CONSTRAINT lake_publish_state CHECK (
    (status IN ('committed', 'superseded')) = (
      object_key IS NOT NULL
      AND manifest_key IS NOT NULL
      AND object_sha256 IS NOT NULL
      AND manifest_sha256 IS NOT NULL
      AND byte_count > 0
      AND published_at IS NOT NULL
    )
  ),
  CONSTRAINT lake_superseded_state CHECK (
    (status = 'superseded') = (superseded_at IS NOT NULL)
  ),
  CONSTRAINT lake_batch_scope_unique UNIQUE (
    batch_id, organization_id, project_id, environment_id
  )
);

ALTER TABLE ingest.canonical_envelopes
  ADD CONSTRAINT canonical_id_scope_unique UNIQUE (
    id, organization_id, project_id, environment_id
  );

CREATE TABLE lake.batch_records (
  batch_id text NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  canonical_envelope_id bigint NOT NULL,
  row_ordinal integer NOT NULL,
  PRIMARY KEY (batch_id, canonical_envelope_id),
  CONSTRAINT lake_batch_records_ordinal_unique UNIQUE (batch_id, row_ordinal),
  CONSTRAINT lake_batch_records_ordinal_bound CHECK (row_ordinal BETWEEN 0 AND 999999),
  FOREIGN KEY (batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id)
    ON DELETE CASCADE,
  FOREIGN KEY (canonical_envelope_id, organization_id, project_id, environment_id)
    REFERENCES ingest.canonical_envelopes(id, organization_id, project_id, environment_id)
);

CREATE TABLE lake.source_claims (
  canonical_envelope_id bigint PRIMARY KEY,
  batch_id text NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  FOREIGN KEY (batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id)
    ON DELETE CASCADE,
  FOREIGN KEY (canonical_envelope_id, organization_id, project_id, environment_id)
    REFERENCES ingest.canonical_envelopes(id, organization_id, project_id, environment_id)
);

CREATE TABLE lake.compaction_sources (
  compaction_batch_id text NOT NULL,
  source_batch_id text NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  PRIMARY KEY (compaction_batch_id, source_batch_id),
  CONSTRAINT lake_compaction_not_self CHECK (compaction_batch_id <> source_batch_id),
  FOREIGN KEY (compaction_batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id)
    ON DELETE CASCADE,
  FOREIGN KEY (source_batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id)
);

CREATE INDEX lake_batch_lease_index
  ON lake.export_batches (organization_id, available_at, created_at, batch_id)
  WHERE status IN ('pending', 'leased');

CREATE INDEX lake_committed_partition_index
  ON lake.export_batches (
    organization_id, project_id, environment_id,
    partition_day, partition_hour, envelope_kind, published_at
  )
  WHERE status = 'committed';

CREATE INDEX lake_batch_record_source_index
  ON lake.batch_records (canonical_envelope_id, batch_id);

ALTER TABLE lake.export_batches ENABLE ROW LEVEL SECURITY;
ALTER TABLE lake.export_batches FORCE ROW LEVEL SECURITY;
CREATE POLICY lake_export_batches_tenant_policy ON lake.export_batches
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lake.batch_records ENABLE ROW LEVEL SECURITY;
ALTER TABLE lake.batch_records FORCE ROW LEVEL SECURITY;
CREATE POLICY lake_batch_records_tenant_policy ON lake.batch_records
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lake.source_claims ENABLE ROW LEVEL SECURITY;
ALTER TABLE lake.source_claims FORCE ROW LEVEL SECURITY;
CREATE POLICY lake_source_claims_tenant_policy ON lake.source_claims
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lake.compaction_sources ENABLE ROW LEVEL SECURITY;
ALTER TABLE lake.compaction_sources FORCE ROW LEVEL SECURITY;
CREATE POLICY lake_compaction_sources_tenant_policy ON lake.compaction_sources
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

GRANT SELECT, INSERT, UPDATE, DELETE ON
  lake.export_batches,
  lake.batch_records,
  lake.source_claims,
  lake.compaction_sources
TO chill_app;

REVOKE TRUNCATE, REFERENCES, TRIGGER ON
  lake.export_batches,
  lake.batch_records,
  lake.source_claims,
  lake.compaction_sources
FROM chill_app;
