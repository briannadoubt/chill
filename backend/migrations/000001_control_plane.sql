CREATE SCHEMA control;

CREATE TABLE control.organizations (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  slug text NOT NULL,
  name text NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  deleted_at timestamptz,
  CONSTRAINT organizations_slug_format CHECK (
    slug ~ '^[a-z][a-z0-9-]{1,62}[a-z0-9]$'
  ),
  CONSTRAINT organizations_name_bound CHECK (
    char_length(name) BETWEEN 1 AND 160
  ),
  CONSTRAINT organizations_status_value CHECK (
    status IN ('active', 'suspended', 'deleted')
  ),
  CONSTRAINT organizations_deletion_state CHECK (
    (status = 'deleted') = (deleted_at IS NOT NULL)
  ),
  CONSTRAINT organizations_time_order CHECK (updated_at >= created_at),
  CONSTRAINT organizations_slug_unique UNIQUE (slug),
  CONSTRAINT organizations_id_scope_unique UNIQUE (id, slug)
);

CREATE TABLE control.users (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  email text NOT NULL,
  display_name text NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  CONSTRAINT users_email_bound CHECK (
    char_length(email) BETWEEN 3 AND 320 AND email = lower(email)
  ),
  CONSTRAINT users_display_name_bound CHECK (
    char_length(display_name) BETWEEN 1 AND 160
  ),
  CONSTRAINT users_status_value CHECK (
    status IN ('invited', 'active', 'suspended', 'deleted')
  ),
  CONSTRAINT users_time_order CHECK (updated_at >= created_at)
);

CREATE UNIQUE INDEX users_email_unique ON control.users (lower(email));

CREATE TABLE control.organization_memberships (
  organization_id uuid NOT NULL REFERENCES control.organizations(id),
  user_id uuid NOT NULL REFERENCES control.users(id),
  role text NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  PRIMARY KEY (organization_id, user_id),
  CONSTRAINT memberships_role_value CHECK (
    role IN ('owner', 'admin', 'developer', 'analyst', 'viewer')
  ),
  CONSTRAINT memberships_status_value CHECK (
    status IN ('invited', 'active', 'suspended')
  ),
  CONSTRAINT memberships_time_order CHECK (updated_at >= created_at)
);

CREATE INDEX memberships_user_index
  ON control.organization_memberships (user_id, organization_id);

CREATE TABLE control.projects (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL REFERENCES control.organizations(id),
  slug text NOT NULL,
  name text NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  CONSTRAINT projects_slug_format CHECK (
    slug ~ '^[a-z][a-z0-9-]{1,62}[a-z0-9]$'
  ),
  CONSTRAINT projects_name_bound CHECK (char_length(name) BETWEEN 1 AND 160),
  CONSTRAINT projects_status_value CHECK (
    status IN ('active', 'suspended', 'deleted')
  ),
  CONSTRAINT projects_time_order CHECK (updated_at >= created_at),
  CONSTRAINT projects_slug_unique UNIQUE (organization_id, slug),
  CONSTRAINT projects_id_scope_unique UNIQUE (id, organization_id)
);

CREATE TABLE control.environments (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  slug text NOT NULL,
  name text NOT NULL,
  kind text NOT NULL,
  status text NOT NULL DEFAULT 'active',
  retention_days integer NOT NULL DEFAULT 30,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (project_id, organization_id)
    REFERENCES control.projects(id, organization_id),
  CONSTRAINT environments_slug_format CHECK (
    slug ~ '^[a-z][a-z0-9-]{1,62}[a-z0-9]$'
  ),
  CONSTRAINT environments_name_bound CHECK (
    char_length(name) BETWEEN 1 AND 160
  ),
  CONSTRAINT environments_kind_value CHECK (
    kind IN ('production', 'staging', 'development', 'test')
  ),
  CONSTRAINT environments_status_value CHECK (
    status IN ('active', 'suspended', 'deleted')
  ),
  CONSTRAINT environments_retention_bound CHECK (
    retention_days BETWEEN 1 AND 3650
  ),
  CONSTRAINT environments_time_order CHECK (updated_at >= created_at),
  CONSTRAINT environments_slug_unique UNIQUE (project_id, slug),
  CONSTRAINT environments_id_scope_unique UNIQUE (
    id, organization_id, project_id
  )
);

CREATE INDEX environments_tenant_index
  ON control.environments (organization_id, project_id, id);

CREATE TABLE control.data_sources (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  name text NOT NULL,
  kind text NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT data_sources_name_bound CHECK (
    char_length(name) BETWEEN 1 AND 160
  ),
  CONSTRAINT data_sources_kind_value CHECK (
    kind IN ('apple', 'android', 'web', 'server', 'otlp')
  ),
  CONSTRAINT data_sources_status_value CHECK (
    status IN ('active', 'suspended', 'deleted')
  ),
  CONSTRAINT data_sources_time_order CHECK (updated_at >= created_at),
  CONSTRAINT data_sources_name_unique UNIQUE (environment_id, name),
  CONSTRAINT data_sources_id_scope_unique UNIQUE (
    id, organization_id, project_id, environment_id
  )
);

CREATE TABLE control.behavior_schemas (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  version text NOT NULL,
  schema_url text NOT NULL,
  digest bytea NOT NULL,
  definition jsonb NOT NULL,
  compatibility text NOT NULL DEFAULT 'exact',
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (project_id, organization_id)
    REFERENCES control.projects(id, organization_id),
  CONSTRAINT behavior_schemas_version_format CHECK (
    version ~ '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
  ),
  CONSTRAINT behavior_schemas_url_bound CHECK (
    char_length(schema_url) BETWEEN 1 AND 2048
  ),
  CONSTRAINT behavior_schemas_digest_size CHECK (octet_length(digest) = 32),
  CONSTRAINT behavior_schemas_definition_object CHECK (
    jsonb_typeof(definition) = 'object'
  ),
  CONSTRAINT behavior_schemas_compatibility_value CHECK (
    compatibility IN ('exact', 'backward', 'forward', 'full')
  ),
  CONSTRAINT behavior_schemas_status_value CHECK (
    status IN ('draft', 'active', 'retired')
  ),
  CONSTRAINT behavior_schemas_time_order CHECK (updated_at >= created_at),
  CONSTRAINT behavior_schemas_version_unique UNIQUE (project_id, version),
  CONSTRAINT behavior_schemas_url_unique UNIQUE (project_id, schema_url)
);

CREATE TABLE control.sdk_keys (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  data_source_id uuid NOT NULL,
  name text NOT NULL,
  prefix text NOT NULL,
  secret_digest bytea NOT NULL,
  scopes text[] NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  expires_at timestamptz,
  revoked_at timestamptz,
  last_used_at timestamptz,
  FOREIGN KEY (data_source_id, organization_id, project_id, environment_id)
    REFERENCES control.data_sources(
      id, organization_id, project_id, environment_id
    ),
  CONSTRAINT sdk_keys_name_bound CHECK (char_length(name) BETWEEN 1 AND 160),
  CONSTRAINT sdk_keys_prefix_format CHECK (prefix ~ '^ch_sk_[0-9a-f]{16}$'),
  CONSTRAINT sdk_keys_digest_size CHECK (octet_length(secret_digest) = 32),
  CONSTRAINT sdk_keys_scopes_bound CHECK (
    cardinality(scopes) BETWEEN 1 AND 8
    AND scopes <@ ARRAY['ingest:otlp', 'ingest:replay']::text[]
  ),
  CONSTRAINT sdk_keys_status_value CHECK (
    status IN ('active', 'revoked', 'expired')
  ),
  CONSTRAINT sdk_keys_revocation_state CHECK (
    (status = 'revoked') = (revoked_at IS NOT NULL)
  ),
  CONSTRAINT sdk_keys_expiry_order CHECK (
    expires_at IS NULL OR expires_at > created_at
  ),
  CONSTRAINT sdk_keys_prefix_unique UNIQUE (prefix)
);

CREATE INDEX sdk_keys_scope_index
  ON control.sdk_keys (organization_id, project_id, environment_id);

CREATE TABLE control.sampling_policies (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  version integer NOT NULL,
  behavior_numerator bigint NOT NULL,
  behavior_denominator bigint NOT NULL,
  replay_numerator bigint NOT NULL,
  replay_denominator bigint NOT NULL,
  salt_version text NOT NULL,
  status text NOT NULL DEFAULT 'draft',
  effective_at timestamptz,
  created_by uuid NOT NULL REFERENCES control.users(id),
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT sampling_policies_version_positive CHECK (version > 0),
  CONSTRAINT sampling_policies_behavior_rate CHECK (
    behavior_denominator > 0
    AND behavior_numerator BETWEEN 0 AND behavior_denominator
  ),
  CONSTRAINT sampling_policies_replay_rate CHECK (
    replay_denominator > 0
    AND replay_numerator BETWEEN 0 AND replay_denominator
  ),
  CONSTRAINT sampling_policies_salt_bound CHECK (
    char_length(salt_version) BETWEEN 1 AND 64
  ),
  CONSTRAINT sampling_policies_status_value CHECK (
    status IN ('draft', 'active', 'retired')
  ),
  CONSTRAINT sampling_policies_effective_state CHECK (
    (status = 'active') = (effective_at IS NOT NULL)
    OR status = 'retired'
  ),
  CONSTRAINT sampling_policies_version_unique UNIQUE (environment_id, version)
);

CREATE UNIQUE INDEX sampling_policies_one_active
  ON control.sampling_policies (environment_id)
  WHERE status = 'active';

CREATE TABLE control.privacy_policies (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  version integer NOT NULL,
  document jsonb NOT NULL,
  digest bytea NOT NULL,
  status text NOT NULL DEFAULT 'draft',
  effective_at timestamptz,
  created_by uuid NOT NULL REFERENCES control.users(id),
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT privacy_policies_version_positive CHECK (version > 0),
  CONSTRAINT privacy_policies_document_object CHECK (
    jsonb_typeof(document) = 'object'
  ),
  CONSTRAINT privacy_policies_digest_size CHECK (octet_length(digest) = 32),
  CONSTRAINT privacy_policies_status_value CHECK (
    status IN ('draft', 'active', 'retired')
  ),
  CONSTRAINT privacy_policies_effective_state CHECK (
    (status = 'active') = (effective_at IS NOT NULL)
    OR status = 'retired'
  ),
  CONSTRAINT privacy_policies_version_unique UNIQUE (environment_id, version)
);

CREATE UNIQUE INDEX privacy_policies_one_active
  ON control.privacy_policies (environment_id)
  WHERE status = 'active';

CREATE TABLE control.quotas (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  requests_per_minute integer NOT NULL,
  records_per_minute integer NOT NULL,
  replay_bytes_per_day bigint NOT NULL,
  query_concurrency integer NOT NULL,
  query_scan_bytes bigint NOT NULL,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT quotas_request_bound CHECK (
    requests_per_minute BETWEEN 1 AND 1000000
  ),
  CONSTRAINT quotas_record_bound CHECK (
    records_per_minute BETWEEN 1 AND 100000000
  ),
  CONSTRAINT quotas_replay_bound CHECK (
    replay_bytes_per_day BETWEEN 0 AND 1099511627776
  ),
  CONSTRAINT quotas_query_concurrency_bound CHECK (
    query_concurrency BETWEEN 1 AND 128
  ),
  CONSTRAINT quotas_query_scan_bound CHECK (
    query_scan_bytes BETWEEN 1048576 AND 10995116277760
  ),
  CONSTRAINT quotas_time_order CHECK (updated_at >= created_at),
  CONSTRAINT quotas_environment_unique UNIQUE (environment_id)
);

CREATE TABLE control.feature_flags (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  key text NOT NULL,
  enabled boolean NOT NULL DEFAULT false,
  configuration jsonb NOT NULL DEFAULT '{}'::jsonb,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT feature_flags_key_format CHECK (
    key ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$'
  ),
  CONSTRAINT feature_flags_configuration_object CHECK (
    jsonb_typeof(configuration) = 'object'
  ),
  CONSTRAINT feature_flags_time_order CHECK (updated_at >= created_at),
  CONSTRAINT feature_flags_key_unique UNIQUE (environment_id, key)
);

CREATE TABLE control.audit_log (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  organization_id uuid NOT NULL REFERENCES control.organizations(id),
  actor_user_id uuid REFERENCES control.users(id),
  action text NOT NULL,
  target_type text NOT NULL,
  target_id uuid,
  request_id text,
  reason_code text,
  details jsonb NOT NULL DEFAULT '{}'::jsonb,
  occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  CONSTRAINT audit_log_action_format CHECK (
    action ~ '^[a-z][a-z0-9_.-]{1,126}[a-z0-9]$'
  ),
  CONSTRAINT audit_log_target_type_format CHECK (
    target_type ~ '^[a-z][a-z0-9_.-]{1,62}[a-z0-9]$'
  ),
  CONSTRAINT audit_log_request_bound CHECK (
    request_id IS NULL OR char_length(request_id) BETWEEN 1 AND 128
  ),
  CONSTRAINT audit_log_reason_bound CHECK (
    reason_code IS NULL OR char_length(reason_code) BETWEEN 1 AND 128
  ),
  CONSTRAINT audit_log_details_object CHECK (jsonb_typeof(details) = 'object')
);

CREATE INDEX audit_log_tenant_time_index
  ON control.audit_log (organization_id, occurred_at DESC, id DESC);

CREATE TABLE control.bootstrap_receipts (
  idempotency_key text PRIMARY KEY,
  organization_id uuid NOT NULL REFERENCES control.organizations(id),
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  data_source_id uuid NOT NULL,
  sdk_key_id uuid NOT NULL REFERENCES control.sdk_keys(id),
  owner_user_id uuid NOT NULL REFERENCES control.users(id),
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (project_id, organization_id)
    REFERENCES control.projects(id, organization_id),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  FOREIGN KEY (data_source_id, organization_id, project_id, environment_id)
    REFERENCES control.data_sources(
      id, organization_id, project_id, environment_id
    ),
  CONSTRAINT bootstrap_receipts_key_bound CHECK (
    char_length(idempotency_key) BETWEEN 16 AND 128
  )
);

CREATE FUNCTION control.set_updated_at()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  NEW.updated_at := transaction_timestamp();
  RETURN NEW;
END
$$;

CREATE TRIGGER organizations_set_updated_at
BEFORE UPDATE ON control.organizations
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER users_set_updated_at
BEFORE UPDATE ON control.users
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER memberships_set_updated_at
BEFORE UPDATE ON control.organization_memberships
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER projects_set_updated_at
BEFORE UPDATE ON control.projects
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER environments_set_updated_at
BEFORE UPDATE ON control.environments
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER data_sources_set_updated_at
BEFORE UPDATE ON control.data_sources
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER behavior_schemas_set_updated_at
BEFORE UPDATE ON control.behavior_schemas
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER quotas_set_updated_at
BEFORE UPDATE ON control.quotas
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE TRIGGER feature_flags_set_updated_at
BEFORE UPDATE ON control.feature_flags
FOR EACH ROW EXECUTE FUNCTION control.set_updated_at();

CREATE FUNCTION control.preserve_active_owner()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF OLD.role = 'owner'
    AND OLD.status = 'active'
    AND (
      TG_OP = 'DELETE'
      OR NEW.role <> 'owner'
      OR NEW.status <> 'active'
    )
    AND NOT EXISTS (
      SELECT 1
      FROM control.organization_memberships AS other
      WHERE other.organization_id = OLD.organization_id
        AND other.user_id <> OLD.user_id
        AND other.role = 'owner'
        AND other.status = 'active'
    )
  THEN
    RAISE EXCEPTION 'organization must retain one active owner'
      USING ERRCODE = '23514';
  END IF;
  RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;

CREATE TRIGGER memberships_preserve_active_owner
BEFORE UPDATE OR DELETE ON control.organization_memberships
FOR EACH ROW EXECUTE FUNCTION control.preserve_active_owner();

CREATE FUNCTION control.prevent_audit_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
  IF current_setting('app.allow_audit_mutation', true) IS DISTINCT FROM 'on'
  THEN
    RAISE EXCEPTION 'audit log is append-only' USING ERRCODE = '55000';
  END IF;
  RETURN CASE WHEN TG_OP = 'DELETE' THEN OLD ELSE NEW END;
END
$$;

CREATE TRIGGER audit_log_append_only
BEFORE UPDATE OR DELETE ON control.audit_log
FOR EACH ROW EXECUTE FUNCTION control.prevent_audit_mutation();
