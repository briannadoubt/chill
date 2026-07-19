CREATE SCHEMA lifecycle;

GRANT USAGE ON SCHEMA lifecycle TO chill_app;

ALTER TABLE control.environments
  ADD COLUMN replay_retention_days integer NOT NULL DEFAULT 7,
  ADD CONSTRAINT environments_replay_retention_bound CHECK (
    replay_retention_days BETWEEN 1 AND 3650
  );

CREATE TABLE lifecycle.deletion_requests (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES control.organizations(id),
  project_id uuid,
  environment_id uuid,
  kind text NOT NULL,
  target_kind text,
  target_value text,
  target_sha256 bytea,
  cutoff_unix_nano numeric(20, 0),
  status text NOT NULL DEFAULT 'pending',
  attempt_count integer NOT NULL DEFAULT 0,
  available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  lease_owner text,
  lease_expires_at timestamptz,
  requested_by text NOT NULL,
  reason_code text NOT NULL,
  idempotency_key text NOT NULL,
  deleted_rows bigint NOT NULL DEFAULT 0,
  deleted_inbox_bytes bigint NOT NULL DEFAULT 0,
  deleted_object_bytes bigint NOT NULL DEFAULT 0,
  rewritten_files integer NOT NULL DEFAULT 0,
  deleted_files integer NOT NULL DEFAULT 0,
  last_error_code text,
  last_error_message text,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  started_at timestamptz,
  completed_at timestamptz,
  completion_sha256 bytea,
  FOREIGN KEY (project_id, organization_id)
    REFERENCES control.projects(id, organization_id),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT deletion_request_kind_value CHECK (
    kind IN ('retention', 'replay_expiry', 'data_subject', 'environment', 'tenant')
  ),
  CONSTRAINT deletion_request_scope CHECK (
    (kind = 'tenant' AND project_id IS NULL AND environment_id IS NULL)
    OR (kind <> 'tenant' AND project_id IS NOT NULL AND environment_id IS NOT NULL)
  ),
  CONSTRAINT deletion_request_target CHECK (
    (kind = 'data_subject'
      AND target_kind IN ('installation_id', 'session_id', 'replay_id')
      AND char_length(target_value) BETWEEN 1 AND 256
      AND octet_length(target_sha256) = 32)
    OR (kind <> 'data_subject'
      AND target_kind IS NULL AND target_value IS NULL AND target_sha256 IS NULL)
  ),
  CONSTRAINT deletion_request_cutoff CHECK (
    (kind IN ('retention', 'replay_expiry')
      AND cutoff_unix_nano BETWEEN 0 AND 18446744073709551615)
    OR (kind NOT IN ('retention', 'replay_expiry') AND cutoff_unix_nano IS NULL)
  ),
  CONSTRAINT deletion_request_status_value CHECK (
    status IN ('pending', 'leased', 'completed', 'failed', 'cancelled')
  ),
  CONSTRAINT deletion_request_attempt_bound CHECK (attempt_count BETWEEN 0 AND 1000),
  CONSTRAINT deletion_request_lease_state CHECK (
    (status = 'leased') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
  ),
  CONSTRAINT deletion_request_lease_owner_bound CHECK (
    lease_owner IS NULL OR char_length(lease_owner) BETWEEN 1 AND 128
  ),
  CONSTRAINT deletion_request_actor_bound CHECK (
    char_length(requested_by) BETWEEN 1 AND 160
  ),
  CONSTRAINT deletion_request_reason_format CHECK (
    reason_code ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$'
  ),
  CONSTRAINT deletion_request_idempotency_bound CHECK (
    char_length(idempotency_key) BETWEEN 1 AND 128
  ),
  CONSTRAINT deletion_request_counts_nonnegative CHECK (
    deleted_rows >= 0 AND deleted_inbox_bytes >= 0
    AND deleted_object_bytes >= 0 AND rewritten_files >= 0 AND deleted_files >= 0
  ),
  CONSTRAINT deletion_request_error_bounds CHECK (
    (last_error_code IS NULL OR last_error_code ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$')
    AND (last_error_message IS NULL OR char_length(last_error_message) BETWEEN 1 AND 512)
  ),
  CONSTRAINT deletion_request_completion_state CHECK (
    (status = 'completed') = (completed_at IS NOT NULL AND octet_length(completion_sha256) = 32)
  ),
  CONSTRAINT deletion_request_target_erasure CHECK (
    status <> 'completed' OR target_value IS NULL
  ),
  CONSTRAINT deletion_request_idempotency_unique UNIQUE (organization_id, idempotency_key),
  CONSTRAINT deletion_request_scope_unique UNIQUE (id, organization_id)
);

CREATE INDEX deletion_requests_lease_index
  ON lifecycle.deletion_requests (available_at, created_at, id)
  WHERE status IN ('pending', 'leased');

CREATE TABLE lifecycle.subject_tombstones (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  request_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  target_kind text NOT NULL,
  target_sha256 bytea NOT NULL,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  FOREIGN KEY (request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT subject_tombstone_target_kind CHECK (
    target_kind IN ('installation_id', 'session_id', 'replay_id')
  ),
  CONSTRAINT subject_tombstone_digest_size CHECK (octet_length(target_sha256) = 32),
  CONSTRAINT subject_tombstone_unique UNIQUE (
    environment_id, target_kind, target_sha256
  )
);

CREATE TABLE lifecycle.deletion_objects (
  request_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  batch_id text NOT NULL,
  object_kind text NOT NULL,
  object_key text NOT NULL,
  object_sha256 bytea NOT NULL,
  byte_count bigint NOT NULL,
  status text NOT NULL DEFAULT 'pending',
  deleted_at timestamptz,
  last_error_message text,
  PRIMARY KEY (request_id, object_key),
  FOREIGN KEY (request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id),
  FOREIGN KEY (batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id),
  CONSTRAINT deletion_object_kind_value CHECK (object_kind IN ('parquet', 'manifest')),
  CONSTRAINT deletion_object_key_bound CHECK (
    char_length(object_key) BETWEEN 1 AND 1024
    AND object_key !~ '(^|/)\.\.(/|$)' AND left(object_key, 1) <> '/'
  ),
  CONSTRAINT deletion_object_digest_size CHECK (octet_length(object_sha256) = 32),
  CONSTRAINT deletion_object_byte_count CHECK (byte_count >= 0),
  CONSTRAINT deletion_object_status_value CHECK (status IN ('pending', 'deleted')),
  CONSTRAINT deletion_object_state CHECK (
    (status = 'deleted') = (deleted_at IS NOT NULL)
  ),
  CONSTRAINT deletion_object_error_bound CHECK (
    last_error_message IS NULL OR char_length(last_error_message) BETWEEN 1 AND 512
  )
);

CREATE TABLE lifecycle.deletion_batches (
  request_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  source_batch_id text NOT NULL,
  replacement_batch_id text,
  removed_rows integer NOT NULL,
  surviving_rows integer NOT NULL,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  PRIMARY KEY (request_id, source_batch_id),
  FOREIGN KEY (request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id),
  FOREIGN KEY (source_batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id),
  FOREIGN KEY (replacement_batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id),
  CONSTRAINT deletion_batch_counts CHECK (
    removed_rows >= 0 AND surviving_rows >= 0 AND removed_rows + surviving_rows > 0
  )
);

CREATE TABLE lifecycle.environment_generations (
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid PRIMARY KEY,
  generation bigint NOT NULL DEFAULT 0,
  reason_code text NOT NULL DEFAULT 'initial',
  request_id uuid,
  invalidated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  FOREIGN KEY (request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id),
  CONSTRAINT environment_generation_nonnegative CHECK (generation >= 0),
  CONSTRAINT environment_generation_reason_format CHECK (
    reason_code = 'initial' OR reason_code ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$'
  )
);

CREATE TABLE lifecycle.derived_rebuilds (
  request_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  dataset_kind text NOT NULL,
  generation bigint NOT NULL,
  status text NOT NULL DEFAULT 'pending',
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  completed_at timestamptz,
  PRIMARY KEY (request_id, environment_id, dataset_kind),
  FOREIGN KEY (request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT derived_rebuild_kind_value CHECK (
    dataset_kind IN ('query', 'trace_index', 'replay_index', 'funnel', 'cohort', 'aggregate')
  ),
  CONSTRAINT derived_rebuild_generation_nonnegative CHECK (generation >= 0),
  CONSTRAINT derived_rebuild_status_value CHECK (status IN ('pending', 'completed')),
  CONSTRAINT derived_rebuild_completion_state CHECK (
    (status = 'completed') = (completed_at IS NOT NULL)
  )
);

CREATE TABLE lake.dataset_schemas (
  version integer PRIMARY KEY,
  definition jsonb NOT NULL,
  digest bytea NOT NULL,
  compatibility text NOT NULL,
  status text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  CONSTRAINT dataset_schema_version_positive CHECK (version > 0),
  CONSTRAINT dataset_schema_definition_object CHECK (jsonb_typeof(definition) = 'object'),
  CONSTRAINT dataset_schema_digest_size CHECK (octet_length(digest) = 32),
  CONSTRAINT dataset_schema_compatibility_value CHECK (
    compatibility IN ('backward', 'forward', 'full')
  ),
  CONSTRAINT dataset_schema_status_value CHECK (status IN ('draft', 'active', 'retired'))
);

INSERT INTO lake.dataset_schemas (version, definition, digest, compatibility, status)
VALUES (
  1,
  '{"name":"chill.lake.Row","required_columns":["dataset_schema_version","batch_id","row_ordinal","canonical_envelope_id","organization_id","project_id","environment_id","data_source_id","envelope_version","envelope_kind","record_id","record_sha256","server_received_at_unix_nano","effective_occurred_at_unix_nano","clock_skew_nano","timing_class","late_arrival","canonical_json","normalized_at_unix_nano"]}'::jsonb,
  decode('94edc07f8960e37390e5b87c14a859e50bee17685acf915a1f5125413014e028', 'hex'),
  'backward',
  'active'
);

ALTER TABLE lake.export_batches
  DROP CONSTRAINT lake_batch_kind_value,
  DROP CONSTRAINT lake_status_value,
  DROP CONSTRAINT lake_publish_state,
  DROP CONSTRAINT lake_superseded_state,
  ADD COLUMN tombstoned_at timestamptz,
  ADD COLUMN deletion_request_id uuid,
  ADD CONSTRAINT lake_batch_kind_value CHECK (
    batch_kind IN ('micro', 'compaction', 'rewrite')
  ),
  ADD CONSTRAINT lake_status_value CHECK (
    status IN ('pending', 'leased', 'committed', 'superseded', 'tombstoned', 'dead_letter')
  ),
  ADD CONSTRAINT lake_publish_state CHECK (
    (status IN ('committed', 'superseded', 'tombstoned')) = (
      object_key IS NOT NULL AND manifest_key IS NOT NULL
      AND object_sha256 IS NOT NULL AND manifest_sha256 IS NOT NULL
      AND byte_count > 0 AND published_at IS NOT NULL
    )
  ),
  ADD CONSTRAINT lake_superseded_state CHECK (
    (status <> 'superseded' OR superseded_at IS NOT NULL)
    AND (status IN ('superseded', 'tombstoned') OR superseded_at IS NULL)
  ),
  ADD CONSTRAINT lake_tombstoned_state CHECK (
    (status = 'tombstoned') = (
      tombstoned_at IS NOT NULL AND deletion_request_id IS NOT NULL
    )
  ),
  ADD FOREIGN KEY (deletion_request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id);

CREATE TABLE lake.rewrite_sources (
  rewrite_batch_id text NOT NULL,
  source_batch_id text NOT NULL,
  request_id uuid NOT NULL,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  PRIMARY KEY (rewrite_batch_id, source_batch_id),
  FOREIGN KEY (rewrite_batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id),
  FOREIGN KEY (source_batch_id, organization_id, project_id, environment_id)
    REFERENCES lake.export_batches(batch_id, organization_id, project_id, environment_id),
  FOREIGN KEY (request_id, organization_id)
    REFERENCES lifecycle.deletion_requests(id, organization_id),
  CONSTRAINT lake_rewrite_not_self CHECK (rewrite_batch_id <> source_batch_id)
);

ALTER TABLE lifecycle.deletion_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifecycle.deletion_requests FORCE ROW LEVEL SECURITY;
CREATE POLICY deletion_requests_tenant_policy ON lifecycle.deletion_requests
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lifecycle.subject_tombstones ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifecycle.subject_tombstones FORCE ROW LEVEL SECURITY;
CREATE POLICY subject_tombstones_tenant_policy ON lifecycle.subject_tombstones
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lifecycle.deletion_objects ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifecycle.deletion_objects FORCE ROW LEVEL SECURITY;
CREATE POLICY deletion_objects_tenant_policy ON lifecycle.deletion_objects
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lifecycle.deletion_batches ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifecycle.deletion_batches FORCE ROW LEVEL SECURITY;
CREATE POLICY deletion_batches_tenant_policy ON lifecycle.deletion_batches
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lifecycle.environment_generations ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifecycle.environment_generations FORCE ROW LEVEL SECURITY;
CREATE POLICY environment_generations_tenant_policy ON lifecycle.environment_generations
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lifecycle.derived_rebuilds ENABLE ROW LEVEL SECURITY;
ALTER TABLE lifecycle.derived_rebuilds FORCE ROW LEVEL SECURITY;
CREATE POLICY derived_rebuilds_tenant_policy ON lifecycle.derived_rebuilds
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE lake.rewrite_sources ENABLE ROW LEVEL SECURITY;
ALTER TABLE lake.rewrite_sources FORCE ROW LEVEL SECURITY;
CREATE POLICY lake_rewrite_sources_tenant_policy ON lake.rewrite_sources
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

GRANT SELECT, INSERT, UPDATE, DELETE ON
  lifecycle.deletion_requests,
  lifecycle.subject_tombstones,
  lifecycle.deletion_objects,
  lifecycle.deletion_batches,
  lifecycle.environment_generations,
  lifecycle.derived_rebuilds,
  lake.rewrite_sources
TO chill_app;

GRANT SELECT ON lake.dataset_schemas TO chill_app;

REVOKE TRUNCATE, REFERENCES, TRIGGER ON
  lifecycle.deletion_requests,
  lifecycle.subject_tombstones,
  lifecycle.deletion_objects,
  lifecycle.deletion_batches,
  lifecycle.environment_generations,
  lifecycle.derived_rebuilds,
  lake.rewrite_sources,
  lake.dataset_schemas
FROM chill_app;

CREATE FUNCTION control.list_lifecycle_organization_ids()
RETURNS SETOF uuid
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, lifecycle
AS $$
  SELECT DISTINCT organization_id
  FROM lifecycle.deletion_requests
  WHERE status = 'pending'
     OR (status = 'leased' AND lease_expires_at <= statement_timestamp())
  ORDER BY organization_id
$$;

REVOKE ALL ON FUNCTION control.list_lifecycle_organization_ids() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.list_lifecycle_organization_ids() TO chill_app;

CREATE FUNCTION control.list_retention_scopes()
RETURNS TABLE (
  organization_id uuid,
  project_id uuid,
  environment_id uuid,
  retention_days integer,
  replay_retention_days integer
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT organization_id, project_id, id, retention_days, replay_retention_days
  FROM control.environments
  WHERE status = 'active'
  ORDER BY organization_id, project_id, id
$$;

REVOKE ALL ON FUNCTION control.list_retention_scopes() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.list_retention_scopes() TO chill_app;
