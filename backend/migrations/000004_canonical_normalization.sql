ALTER TABLE ingest.inbox
  ADD COLUMN lease_owner text,
  ADD COLUMN lease_expires_at timestamptz,
  ADD COLUMN last_error_code text,
  ADD COLUMN last_error_message text,
  ADD CONSTRAINT inbox_lease_state CHECK (
    (status = 'leased') = (lease_owner IS NOT NULL AND lease_expires_at IS NOT NULL)
  ),
  ADD CONSTRAINT inbox_lease_owner_bound CHECK (
    lease_owner IS NULL OR char_length(lease_owner) BETWEEN 1 AND 128
  ),
  ADD CONSTRAINT inbox_error_code_bound CHECK (
    last_error_code IS NULL
    OR last_error_code ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$'
  ),
  ADD CONSTRAINT inbox_error_message_bound CHECK (
    last_error_message IS NULL
    OR char_length(last_error_message) BETWEEN 1 AND 512
  ),
  ADD CONSTRAINT inbox_id_scope_unique UNIQUE (
    id, organization_id, project_id, environment_id, data_source_id
  );

CREATE TABLE ingest.canonical_envelopes (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  data_source_id uuid NOT NULL,
  inbox_id bigint NOT NULL,
  ordinal integer NOT NULL,
  envelope_version text NOT NULL DEFAULT '1.0.0',
  envelope_kind text NOT NULL,
  record_id text NOT NULL,
  record_sha256 bytea NOT NULL,
  installation_id text,
  session_id text,
  replay_id text,
  replay_chunk_id text,
  trace_id text,
  span_id text,
  occurred_at_unix_nano numeric(20, 0),
  source_observed_at_unix_nano numeric(20, 0),
  server_received_at_unix_nano numeric(20, 0) NOT NULL,
  effective_occurred_at_unix_nano numeric(20, 0) NOT NULL,
  monotonic_nano numeric(20, 0),
  boot_id text,
  sequence_number numeric(20, 0),
  clock_skew_nano numeric(21, 0),
  timing_class text NOT NULL,
  late_arrival boolean NOT NULL,
  canonical jsonb NOT NULL,
  normalized_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  FOREIGN KEY (
    inbox_id, organization_id, project_id, environment_id, data_source_id
  ) REFERENCES ingest.inbox(
    id, organization_id, project_id, environment_id, data_source_id
  ),
  CONSTRAINT canonical_envelope_version_value CHECK (
    envelope_version = '1.0.0'
  ),
  CONSTRAINT canonical_envelope_kind_value CHECK (
    envelope_kind IN (
      'behavior', 'otel.log', 'otel.span', 'otel.metric', 'replay'
    )
  ),
  CONSTRAINT canonical_ordinal_bound CHECK (
    ordinal BETWEEN 0 AND 999999
  ),
  CONSTRAINT canonical_record_id_bound CHECK (
    char_length(record_id) BETWEEN 1 AND 256
  ),
  CONSTRAINT canonical_digest_size CHECK (
    octet_length(record_sha256) = 32
  ),
  CONSTRAINT canonical_uint64_times CHECK (
    (occurred_at_unix_nano IS NULL OR occurred_at_unix_nano BETWEEN 0 AND 18446744073709551615)
    AND (source_observed_at_unix_nano IS NULL OR source_observed_at_unix_nano BETWEEN 0 AND 18446744073709551615)
    AND server_received_at_unix_nano BETWEEN 0 AND 18446744073709551615
    AND effective_occurred_at_unix_nano BETWEEN 0 AND 18446744073709551615
    AND (monotonic_nano IS NULL OR monotonic_nano BETWEEN 0 AND 18446744073709551615)
    AND (sequence_number IS NULL OR sequence_number BETWEEN 0 AND 9007199254740991)
  ),
  CONSTRAINT canonical_monotonic_epoch CHECK (
    (monotonic_nano IS NULL) = (boot_id IS NULL)
  ),
  CONSTRAINT canonical_trace_id_format CHECK (
    trace_id IS NULL OR (
      trace_id ~ '^[0-9a-f]{32}$' AND trace_id <> repeat('0', 32)
    )
  ),
  CONSTRAINT canonical_span_id_format CHECK (
    span_id IS NULL OR (
      span_id ~ '^[0-9a-f]{16}$' AND span_id <> repeat('0', 16)
    )
  ),
  CONSTRAINT canonical_timing_class_value CHECK (
    timing_class IN (
      'on_time', 'late', 'future_clock_skew', 'missing_source_time'
    )
  ),
  CONSTRAINT canonical_late_state CHECK (
    late_arrival = (timing_class = 'late')
  ),
  CONSTRAINT canonical_document_object CHECK (
    jsonb_typeof(canonical) = 'object'
  ),
  CONSTRAINT canonical_inbox_ordinal_unique UNIQUE (inbox_id, ordinal),
  CONSTRAINT canonical_record_unique UNIQUE (
    environment_id, envelope_kind, record_id
  )
);

CREATE INDEX canonical_tenant_order_index
  ON ingest.canonical_envelopes (
    organization_id,
    project_id,
    environment_id,
    effective_occurred_at_unix_nano,
    installation_id,
    boot_id,
    sequence_number,
    record_id
  );

CREATE INDEX inbox_expired_lease_index
  ON ingest.inbox (lease_expires_at, id)
  WHERE status = 'leased';

CREATE INDEX canonical_trace_index
  ON ingest.canonical_envelopes (
    organization_id, project_id, environment_id, trace_id, span_id
  )
  WHERE trace_id IS NOT NULL;

CREATE INDEX canonical_session_replay_index
  ON ingest.canonical_envelopes (
    organization_id, project_id, environment_id, session_id, replay_id,
    replay_chunk_id
  )
  WHERE session_id IS NOT NULL;

ALTER TABLE ingest.canonical_envelopes ENABLE ROW LEVEL SECURITY;
ALTER TABLE ingest.canonical_envelopes FORCE ROW LEVEL SECURITY;
CREATE POLICY canonical_envelopes_tenant_policy
  ON ingest.canonical_envelopes
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

CREATE FUNCTION control.list_active_organization_ids()
RETURNS SETOF uuid
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT id FROM control.organizations WHERE status = 'active' ORDER BY id
$$;

REVOKE ALL ON FUNCTION control.list_active_organization_ids() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.list_active_organization_ids() TO chill_app;

GRANT SELECT, INSERT, UPDATE, DELETE ON ingest.canonical_envelopes
  TO chill_app;
GRANT USAGE, SELECT ON SEQUENCE ingest.canonical_envelopes_id_seq
  TO chill_app;
REVOKE TRUNCATE, REFERENCES, TRIGGER ON ingest.canonical_envelopes
  FROM chill_app;
