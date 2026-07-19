CREATE TABLE control.collection_policies (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  revision bigint NOT NULL,
  policy_version text NOT NULL,
  enabled boolean NOT NULL,
  disabled_capture_classes text[] NOT NULL DEFAULT '{}',
  status text NOT NULL DEFAULT 'active',
  effective_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  created_by uuid NOT NULL REFERENCES control.users(id),
  idempotency_key text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT collection_policies_revision_positive CHECK (revision > 0),
  CONSTRAINT collection_policies_version_bound CHECK (
    char_length(policy_version) BETWEEN 1 AND 64
  ),
  CONSTRAINT collection_policies_classes_bound CHECK (
    cardinality(disabled_capture_classes) <= 4
    AND disabled_capture_classes <@ ARRAY[
      'essential', 'analytics', 'diagnostic', 'replay'
    ]::text[]
  ),
  CONSTRAINT collection_policies_status_value CHECK (
    status IN ('active', 'retired')
  ),
  CONSTRAINT collection_policies_idempotency_bound CHECK (
    char_length(idempotency_key) BETWEEN 1 AND 128
  ),
  CONSTRAINT collection_policies_revision_unique UNIQUE (
    environment_id, revision
  ),
  CONSTRAINT collection_policies_idempotency_unique UNIQUE (
    organization_id, idempotency_key
  )
);

CREATE UNIQUE INDEX collection_policies_one_active
  ON control.collection_policies (environment_id)
  WHERE status = 'active';

CREATE INDEX collection_policies_tenant_history_index
  ON control.collection_policies (
    organization_id, project_id, environment_id, revision DESC
  );

CREATE SCHEMA compliance;

CREATE TABLE compliance.export_requests (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES control.organizations(id),
  project_id uuid,
  environment_id uuid,
  kind text NOT NULL,
  target_kind text,
  target_sha256 bytea,
  status text NOT NULL DEFAULT 'running',
  requested_by text NOT NULL,
  reason_code text NOT NULL,
  idempotency_key text NOT NULL,
  artifact_sha256 bytea,
  record_count bigint,
  artifact_bytes bigint,
  source_file_count integer,
  error_code text,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  completed_at timestamptz,
  FOREIGN KEY (project_id, organization_id)
    REFERENCES control.projects(id, organization_id),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT export_requests_kind_value CHECK (
    kind IN ('tenant', 'data_subject')
  ),
  CONSTRAINT export_requests_target_kind_value CHECK (
    target_kind IS NULL
    OR target_kind IN ('installation_id', 'session_id', 'replay_id')
  ),
  CONSTRAINT export_requests_target_digest_size CHECK (
    target_sha256 IS NULL OR octet_length(target_sha256) = 32
  ),
  CONSTRAINT export_requests_scope_shape CHECK (
    (kind = 'tenant' AND project_id IS NULL AND environment_id IS NULL
      AND target_kind IS NULL AND target_sha256 IS NULL)
    OR
    (kind = 'data_subject' AND project_id IS NOT NULL
      AND environment_id IS NOT NULL AND target_kind IS NOT NULL
      AND target_sha256 IS NOT NULL)
  ),
  CONSTRAINT export_requests_status_value CHECK (
    status IN ('running', 'completed', 'failed')
  ),
  CONSTRAINT export_requests_requester_bound CHECK (
    char_length(requested_by) BETWEEN 1 AND 160
  ),
  CONSTRAINT export_requests_reason_format CHECK (
    reason_code ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$'
  ),
  CONSTRAINT export_requests_idempotency_bound CHECK (
    char_length(idempotency_key) BETWEEN 1 AND 128
  ),
  CONSTRAINT export_requests_completion_shape CHECK (
    (status = 'running' AND artifact_sha256 IS NULL
      AND completed_at IS NULL AND error_code IS NULL)
    OR
    (status = 'completed' AND octet_length(artifact_sha256) = 32
      AND record_count >= 0 AND artifact_bytes > 0
      AND source_file_count >= 0 AND completed_at IS NOT NULL
      AND error_code IS NULL)
    OR
    (status = 'failed' AND artifact_sha256 IS NULL
      AND completed_at IS NOT NULL AND error_code IS NOT NULL)
  ),
  CONSTRAINT export_requests_idempotency_unique UNIQUE (
    organization_id, idempotency_key
  )
);

CREATE INDEX export_requests_tenant_time_index
  ON compliance.export_requests (organization_id, created_at DESC, id DESC);

ALTER TABLE control.collection_policies ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.collection_policies FORCE ROW LEVEL SECURITY;
CREATE POLICY collection_policies_tenant_policy
  ON control.collection_policies
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE compliance.export_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE compliance.export_requests FORCE ROW LEVEL SECURITY;
CREATE POLICY export_requests_tenant_policy
  ON compliance.export_requests
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

CREATE FUNCTION control.prevent_collection_policy_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF OLD.status = 'retired'
    OR NEW.organization_id <> OLD.organization_id
    OR NEW.project_id <> OLD.project_id
    OR NEW.environment_id <> OLD.environment_id
    OR NEW.revision <> OLD.revision
    OR NEW.policy_version <> OLD.policy_version
    OR NEW.enabled <> OLD.enabled
    OR NEW.disabled_capture_classes <> OLD.disabled_capture_classes
    OR NEW.effective_at <> OLD.effective_at
    OR NEW.created_by <> OLD.created_by
    OR NEW.idempotency_key <> OLD.idempotency_key
    OR NEW.created_at <> OLD.created_at
    OR NOT (OLD.status = 'active' AND NEW.status = 'retired')
  THEN
    RAISE EXCEPTION 'collection policy history is immutable'
      USING ERRCODE = '55000';
  END IF;
  RETURN NEW;
END
$$;

CREATE TRIGGER collection_policies_immutable
BEFORE UPDATE ON control.collection_policies
FOR EACH ROW EXECUTE FUNCTION control.prevent_collection_policy_mutation();

CREATE FUNCTION compliance.prevent_export_request_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF OLD.status <> 'running'
    OR NEW.organization_id <> OLD.organization_id
    OR NEW.project_id IS DISTINCT FROM OLD.project_id
    OR NEW.environment_id IS DISTINCT FROM OLD.environment_id
    OR NEW.kind <> OLD.kind
    OR NEW.target_kind IS DISTINCT FROM OLD.target_kind
    OR NEW.target_sha256 IS DISTINCT FROM OLD.target_sha256
    OR NEW.requested_by <> OLD.requested_by
    OR NEW.reason_code <> OLD.reason_code
    OR NEW.idempotency_key <> OLD.idempotency_key
    OR NEW.created_at <> OLD.created_at
    OR NEW.status NOT IN ('completed', 'failed')
  THEN
    RAISE EXCEPTION 'export request history is immutable'
      USING ERRCODE = '55000';
  END IF;
  RETURN NEW;
END
$$;

CREATE TRIGGER export_requests_immutable
BEFORE UPDATE ON compliance.export_requests
FOR EACH ROW EXECUTE FUNCTION compliance.prevent_export_request_mutation();

GRANT USAGE ON SCHEMA compliance TO chill_app;
GRANT SELECT, INSERT, UPDATE ON
  control.collection_policies,
  compliance.export_requests
TO chill_app;
REVOKE DELETE, TRUNCATE, REFERENCES, TRIGGER ON
  control.collection_policies,
  compliance.export_requests
FROM chill_app;
