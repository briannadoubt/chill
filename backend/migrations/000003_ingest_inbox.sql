CREATE SCHEMA ingest;

ALTER TABLE control.sdk_keys
  ADD CONSTRAINT sdk_keys_id_scope_unique UNIQUE (
    id, organization_id, project_id, environment_id, data_source_id
  );

CREATE TABLE ingest.inbox (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  data_source_id uuid NOT NULL,
  sdk_key_id uuid NOT NULL,
  request_id text NOT NULL,
  signal_kind text NOT NULL,
  payload_format text NOT NULL,
  payload bytea NOT NULL,
  payload_sha256 bytea NOT NULL,
  record_count integer NOT NULL,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  status text NOT NULL DEFAULT 'pending',
  attempt_count integer NOT NULL DEFAULT 0,
  server_received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  available_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  completed_at timestamptz,
  FOREIGN KEY (data_source_id, organization_id, project_id, environment_id)
    REFERENCES control.data_sources(
      id, organization_id, project_id, environment_id
    ),
  FOREIGN KEY (
    sdk_key_id, organization_id, project_id, environment_id, data_source_id
  ) REFERENCES control.sdk_keys(
    id, organization_id, project_id, environment_id, data_source_id
  ),
  CONSTRAINT inbox_request_id_bound CHECK (
    char_length(request_id) BETWEEN 1 AND 128
  ),
  CONSTRAINT inbox_signal_kind_value CHECK (
    signal_kind IN ('logs', 'traces', 'metrics', 'replay')
  ),
  CONSTRAINT inbox_payload_format_value CHECK (
    payload_format IN ('protobuf', 'json', 'replay-v1')
  ),
  CONSTRAINT inbox_payload_bound CHECK (
    octet_length(payload) BETWEEN 1 AND 16777216
  ),
  CONSTRAINT inbox_payload_digest_size CHECK (
    octet_length(payload_sha256) = 32
  ),
  CONSTRAINT inbox_record_count_bound CHECK (
    record_count BETWEEN 1 AND 1000000
  ),
  CONSTRAINT inbox_metadata_object CHECK (jsonb_typeof(metadata) = 'object'),
  CONSTRAINT inbox_status_value CHECK (
    status IN ('pending', 'leased', 'completed', 'dead_letter')
  ),
  CONSTRAINT inbox_attempt_count_bound CHECK (
    attempt_count BETWEEN 0 AND 1000
  ),
  CONSTRAINT inbox_completion_state CHECK (
    (status = 'completed') = (completed_at IS NOT NULL)
  ),
  CONSTRAINT inbox_request_unique UNIQUE (
    environment_id, signal_kind, request_id
  )
);

CREATE INDEX inbox_pending_index
  ON ingest.inbox (available_at, id)
  WHERE status = 'pending';

CREATE INDEX inbox_tenant_received_index
  ON ingest.inbox (
    organization_id, project_id, environment_id, server_received_at DESC, id DESC
  );

CREATE TABLE ingest.rate_buckets (
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  minute_start timestamptz NOT NULL,
  request_count integer NOT NULL,
  record_count bigint NOT NULL,
  replay_bytes bigint NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  PRIMARY KEY (environment_id, minute_start),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT rate_buckets_request_nonnegative CHECK (request_count >= 0),
  CONSTRAINT rate_buckets_record_nonnegative CHECK (record_count >= 0),
  CONSTRAINT rate_buckets_replay_nonnegative CHECK (replay_bytes >= 0)
);

CREATE TABLE ingest.daily_buckets (
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  day_start date NOT NULL,
  replay_bytes bigint NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  PRIMARY KEY (environment_id, day_start),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT daily_buckets_replay_nonnegative CHECK (replay_bytes >= 0)
);

ALTER TABLE ingest.inbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE ingest.inbox FORCE ROW LEVEL SECURITY;
CREATE POLICY inbox_tenant_policy ON ingest.inbox
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE ingest.rate_buckets ENABLE ROW LEVEL SECURITY;
ALTER TABLE ingest.rate_buckets FORCE ROW LEVEL SECURITY;
CREATE POLICY rate_buckets_tenant_policy ON ingest.rate_buckets
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE ingest.daily_buckets ENABLE ROW LEVEL SECURITY;
ALTER TABLE ingest.daily_buckets FORCE ROW LEVEL SECURITY;
CREATE POLICY daily_buckets_tenant_policy ON ingest.daily_buckets
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

GRANT USAGE ON SCHEMA ingest TO chill_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON
  ingest.inbox, ingest.rate_buckets, ingest.daily_buckets
  TO chill_app;
GRANT USAGE, SELECT ON SEQUENCE ingest.inbox_id_seq TO chill_app;

REVOKE TRUNCATE, REFERENCES, TRIGGER ON ALL TABLES IN SCHEMA ingest
  FROM chill_app;
