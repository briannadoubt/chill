DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'chill_app') THEN
    CREATE ROLE chill_app NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT;
  END IF;
END
$$;

CREATE FUNCTION control.current_organization_id()
RETURNS uuid
LANGUAGE sql
STABLE
SET search_path = pg_catalog
AS $$
  SELECT nullif(current_setting('app.organization_id', true), '')::uuid
$$;

REVOKE ALL ON FUNCTION control.current_organization_id() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.current_organization_id() TO chill_app;

ALTER TABLE control.organizations ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.organizations FORCE ROW LEVEL SECURITY;
CREATE POLICY organizations_tenant_policy ON control.organizations
  USING (id = control.current_organization_id())
  WITH CHECK (id = control.current_organization_id());

ALTER TABLE control.users ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.users FORCE ROW LEVEL SECURITY;
CREATE POLICY users_tenant_policy ON control.users
  USING (
    EXISTS (
      SELECT 1
      FROM control.organization_memberships AS membership
      WHERE membership.user_id = users.id
        AND membership.organization_id = control.current_organization_id()
    )
  );

ALTER TABLE control.organization_memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.organization_memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY memberships_tenant_policy ON control.organization_memberships
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.projects FORCE ROW LEVEL SECURITY;
CREATE POLICY projects_tenant_policy ON control.projects
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.environments ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.environments FORCE ROW LEVEL SECURITY;
CREATE POLICY environments_tenant_policy ON control.environments
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.data_sources ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.data_sources FORCE ROW LEVEL SECURITY;
CREATE POLICY data_sources_tenant_policy ON control.data_sources
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.behavior_schemas ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.behavior_schemas FORCE ROW LEVEL SECURITY;
CREATE POLICY behavior_schemas_tenant_policy ON control.behavior_schemas
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

-- The table owner must be able to execute the tightly scoped SECURITY DEFINER
-- key lookup before a request has trusted tenant context. Runtime roles remain
-- subject to RLS and do not own this table.
ALTER TABLE control.sdk_keys ENABLE ROW LEVEL SECURITY;
CREATE POLICY sdk_keys_tenant_policy ON control.sdk_keys
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.sampling_policies ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.sampling_policies FORCE ROW LEVEL SECURITY;
CREATE POLICY sampling_policies_tenant_policy ON control.sampling_policies
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.privacy_policies ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.privacy_policies FORCE ROW LEVEL SECURITY;
CREATE POLICY privacy_policies_tenant_policy ON control.privacy_policies
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.quotas ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.quotas FORCE ROW LEVEL SECURITY;
CREATE POLICY quotas_tenant_policy ON control.quotas
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.feature_flags ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.feature_flags FORCE ROW LEVEL SECURITY;
CREATE POLICY feature_flags_tenant_policy ON control.feature_flags
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.audit_log ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.audit_log FORCE ROW LEVEL SECURITY;
CREATE POLICY audit_log_tenant_policy ON control.audit_log
  FOR SELECT
  USING (organization_id = control.current_organization_id());
CREATE POLICY audit_log_insert_policy ON control.audit_log
  FOR INSERT
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.bootstrap_receipts ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.bootstrap_receipts FORCE ROW LEVEL SECURITY;
CREATE POLICY bootstrap_receipts_tenant_policy ON control.bootstrap_receipts
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

CREATE FUNCTION control.lookup_sdk_key(key_prefix text)
RETURNS TABLE (
  key_id uuid,
  organization_id uuid,
  project_id uuid,
  environment_id uuid,
  data_source_id uuid,
  secret_digest bytea,
  scopes text[]
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT
    key.id,
    key.organization_id,
    key.project_id,
    key.environment_id,
    key.data_source_id,
    key.secret_digest,
    key.scopes
  FROM control.sdk_keys AS key
  WHERE key.prefix = key_prefix
    AND key.status = 'active'
    AND (key.expires_at IS NULL OR key.expires_at > statement_timestamp())
$$;

REVOKE ALL ON FUNCTION control.lookup_sdk_key(text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.lookup_sdk_key(text) TO chill_app;

CREATE FUNCTION control.mark_sdk_key_used(key_id uuid)
RETURNS void
LANGUAGE sql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  UPDATE control.sdk_keys
  SET last_used_at = clock_timestamp()
  WHERE id = key_id
    AND organization_id = control.current_organization_id()
    AND status = 'active'
$$;

REVOKE ALL ON FUNCTION control.mark_sdk_key_used(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.mark_sdk_key_used(uuid) TO chill_app;

GRANT USAGE ON SCHEMA control TO chill_app;
GRANT SELECT, INSERT, UPDATE, DELETE ON
  control.organizations,
  control.users,
  control.organization_memberships,
  control.projects,
  control.environments,
  control.data_sources,
  control.behavior_schemas,
  control.sdk_keys,
  control.sampling_policies,
  control.privacy_policies,
  control.quotas,
  control.feature_flags,
  control.bootstrap_receipts
TO chill_app;
GRANT SELECT, INSERT ON control.audit_log TO chill_app;
GRANT USAGE, SELECT ON SEQUENCE control.audit_log_id_seq TO chill_app;

REVOKE TRUNCATE, REFERENCES, TRIGGER ON ALL TABLES IN SCHEMA control
  FROM chill_app;

ALTER DEFAULT PRIVILEGES IN SCHEMA control
  REVOKE ALL ON TABLES FROM PUBLIC;
ALTER DEFAULT PRIVILEGES IN SCHEMA control
  REVOKE ALL ON SEQUENCES FROM PUBLIC;
ALTER DEFAULT PRIVILEGES IN SCHEMA control
  REVOKE ALL ON FUNCTIONS FROM PUBLIC;
