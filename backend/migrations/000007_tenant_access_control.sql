ALTER TABLE control.sdk_keys
  ADD COLUMN rotated_from_id uuid REFERENCES control.sdk_keys(id),
  ADD COLUMN replaced_by_id uuid REFERENCES control.sdk_keys(id),
  ADD CONSTRAINT sdk_keys_rotation_distinct CHECK (
    rotated_from_id IS NULL OR rotated_from_id <> id
  ),
  ADD CONSTRAINT sdk_keys_replacement_distinct CHECK (
    replaced_by_id IS NULL OR replaced_by_id <> id
  ),
  ADD CONSTRAINT sdk_keys_rotated_from_unique UNIQUE (rotated_from_id),
  ADD CONSTRAINT sdk_keys_replaced_by_unique UNIQUE (replaced_by_id);

CREATE TABLE control.user_identities (
  user_id uuid NOT NULL REFERENCES control.users(id),
  issuer text NOT NULL,
  subject text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  PRIMARY KEY (issuer, subject),
  CONSTRAINT user_identities_issuer_format CHECK (
    issuer ~ '^[a-z][a-z0-9_.-]{1,62}[a-z0-9]$'
  ),
  CONSTRAINT user_identities_subject_bound CHECK (
    char_length(subject) BETWEEN 1 AND 512
  ),
  CONSTRAINT user_identities_user_issuer_unique UNIQUE (user_id, issuer)
);

CREATE INDEX user_identities_user_index
  ON control.user_identities (user_id);

CREATE TABLE control.user_sessions (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  user_id uuid NOT NULL,
  prefix text NOT NULL,
  secret_digest bytea NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  last_used_at timestamptz,
  rotated_from_id uuid REFERENCES control.user_sessions(id),
  replaced_by_id uuid REFERENCES control.user_sessions(id),
  FOREIGN KEY (organization_id, user_id)
    REFERENCES control.organization_memberships(organization_id, user_id),
  CONSTRAINT user_sessions_prefix_format CHECK (
    prefix ~ '^ch_us_[0-9a-f]{16}$'
  ),
  CONSTRAINT user_sessions_digest_size CHECK (
    octet_length(secret_digest) = 32
  ),
  CONSTRAINT user_sessions_status_value CHECK (
    status IN ('active', 'revoked', 'expired')
  ),
  CONSTRAINT user_sessions_revocation_state CHECK (
    (status = 'revoked') = (revoked_at IS NOT NULL)
  ),
  CONSTRAINT user_sessions_expiry_order CHECK (expires_at > created_at),
  CONSTRAINT user_sessions_rotation_distinct CHECK (
    rotated_from_id IS NULL OR rotated_from_id <> id
  ),
  CONSTRAINT user_sessions_replacement_distinct CHECK (
    replaced_by_id IS NULL OR replaced_by_id <> id
  ),
  CONSTRAINT user_sessions_prefix_unique UNIQUE (prefix),
  CONSTRAINT user_sessions_rotated_from_unique UNIQUE (rotated_from_id),
  CONSTRAINT user_sessions_replaced_by_unique UNIQUE (replaced_by_id)
);

CREATE INDEX user_sessions_tenant_user_index
  ON control.user_sessions (organization_id, user_id, created_at DESC);

CREATE TABLE control.service_credentials (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  name text NOT NULL,
  prefix text NOT NULL,
  secret_digest bytea NOT NULL,
  scopes text[] NOT NULL,
  status text NOT NULL DEFAULT 'active',
  created_by uuid NOT NULL REFERENCES control.users(id),
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  expires_at timestamptz,
  revoked_at timestamptz,
  last_used_at timestamptz,
  rotated_from_id uuid REFERENCES control.service_credentials(id),
  replaced_by_id uuid REFERENCES control.service_credentials(id),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT service_credentials_name_bound CHECK (
    char_length(name) BETWEEN 1 AND 160
  ),
  CONSTRAINT service_credentials_prefix_format CHECK (
    prefix ~ '^ch_sv_[0-9a-f]{16}$'
  ),
  CONSTRAINT service_credentials_digest_size CHECK (
    octet_length(secret_digest) = 32
  ),
  CONSTRAINT service_credentials_scopes_bound CHECK (
    cardinality(scopes) BETWEEN 1 AND 8
    AND scopes <@ ARRAY[
      'control:read', 'control:write', 'credentials:manage',
      'data:read', 'data:delete'
    ]::text[]
  ),
  CONSTRAINT service_credentials_status_value CHECK (
    status IN ('active', 'revoked', 'expired')
  ),
  CONSTRAINT service_credentials_revocation_state CHECK (
    (status = 'revoked') = (revoked_at IS NOT NULL)
  ),
  CONSTRAINT service_credentials_expiry_order CHECK (
    expires_at IS NULL OR expires_at > created_at
  ),
  CONSTRAINT service_credentials_rotation_distinct CHECK (
    rotated_from_id IS NULL OR rotated_from_id <> id
  ),
  CONSTRAINT service_credentials_replacement_distinct CHECK (
    replaced_by_id IS NULL OR replaced_by_id <> id
  ),
  CONSTRAINT service_credentials_prefix_unique UNIQUE (prefix),
  CONSTRAINT service_credentials_rotated_from_unique UNIQUE (rotated_from_id),
  CONSTRAINT service_credentials_replaced_by_unique UNIQUE (replaced_by_id)
);

CREATE INDEX service_credentials_tenant_scope_index
  ON control.service_credentials (
    organization_id, project_id, environment_id, created_at DESC
  );

ALTER TABLE control.user_identities ENABLE ROW LEVEL SECURITY;
ALTER TABLE control.user_identities FORCE ROW LEVEL SECURITY;
CREATE POLICY user_identities_tenant_policy ON control.user_identities
  USING (
    EXISTS (
      SELECT 1
      FROM control.organization_memberships AS membership
      WHERE membership.user_id = user_identities.user_id
        AND membership.organization_id = control.current_organization_id()
    )
  );

ALTER TABLE control.user_sessions ENABLE ROW LEVEL SECURITY;
CREATE POLICY user_sessions_tenant_policy ON control.user_sessions
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

ALTER TABLE control.service_credentials ENABLE ROW LEVEL SECURITY;
CREATE POLICY service_credentials_tenant_policy ON control.service_credentials
  USING (organization_id = control.current_organization_id())
  WITH CHECK (organization_id = control.current_organization_id());

CREATE FUNCTION control.lookup_user_identity(
  identity_issuer text,
  identity_subject text,
  tenant_id uuid
)
RETURNS TABLE (user_id uuid)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT identity.user_id
  FROM control.user_identities AS identity
  JOIN control.users AS account ON account.id = identity.user_id
  JOIN control.organization_memberships AS membership
    ON membership.user_id = identity.user_id
   AND membership.organization_id = tenant_id
  JOIN control.organizations AS organization ON organization.id = tenant_id
  WHERE identity.issuer = identity_issuer
    AND identity.subject = identity_subject
    AND account.status = 'active'
    AND membership.status = 'active'
    AND organization.status = 'active'
$$;

CREATE FUNCTION control.lookup_user_session(session_prefix text)
RETURNS TABLE (
  session_id uuid,
  organization_id uuid,
  user_id uuid,
  role text,
  secret_digest bytea,
  expires_at timestamptz
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT
    session.id,
    session.organization_id,
    session.user_id,
    membership.role,
    session.secret_digest,
    session.expires_at
  FROM control.user_sessions AS session
  JOIN control.users AS account ON account.id = session.user_id
  JOIN control.organization_memberships AS membership
    ON membership.organization_id = session.organization_id
   AND membership.user_id = session.user_id
  JOIN control.organizations AS organization
    ON organization.id = session.organization_id
  WHERE session.prefix = session_prefix
    AND session.status = 'active'
    AND session.expires_at > statement_timestamp()
    AND account.status = 'active'
    AND membership.status = 'active'
    AND organization.status = 'active'
$$;

CREATE FUNCTION control.lookup_service_credential(credential_prefix text)
RETURNS TABLE (
  credential_id uuid,
  organization_id uuid,
  project_id uuid,
  environment_id uuid,
  secret_digest bytea,
  scopes text[],
  expires_at timestamptz
)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT
    credential.id,
    credential.organization_id,
    credential.project_id,
    credential.environment_id,
    credential.secret_digest,
    credential.scopes,
    credential.expires_at
  FROM control.service_credentials AS credential
  JOIN control.organizations AS organization
    ON organization.id = credential.organization_id
  JOIN control.projects AS project
    ON project.id = credential.project_id
   AND project.organization_id = credential.organization_id
  JOIN control.environments AS environment
    ON environment.id = credential.environment_id
   AND environment.project_id = credential.project_id
   AND environment.organization_id = credential.organization_id
  WHERE credential.prefix = credential_prefix
    AND credential.status = 'active'
    AND (credential.expires_at IS NULL OR credential.expires_at > statement_timestamp())
    AND organization.status = 'active'
    AND project.status = 'active'
    AND environment.status = 'active'
$$;

CREATE FUNCTION control.mark_user_session_used(session_id uuid)
RETURNS void
LANGUAGE sql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  UPDATE control.user_sessions
  SET last_used_at = clock_timestamp()
  WHERE id = session_id
    AND organization_id = control.current_organization_id()
    AND status = 'active'
$$;

CREATE FUNCTION control.mark_service_credential_used(credential_id uuid)
RETURNS void
LANGUAGE sql
VOLATILE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  UPDATE control.service_credentials
  SET last_used_at = clock_timestamp()
  WHERE id = credential_id
    AND organization_id = control.current_organization_id()
    AND status = 'active'
$$;

REVOKE ALL ON FUNCTION control.lookup_user_identity(text, text, uuid)
  FROM PUBLIC;
REVOKE ALL ON FUNCTION control.lookup_user_session(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION control.lookup_service_credential(text) FROM PUBLIC;
REVOKE ALL ON FUNCTION control.mark_user_session_used(uuid) FROM PUBLIC;
REVOKE ALL ON FUNCTION control.mark_service_credential_used(uuid) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION control.lookup_user_identity(text, text, uuid)
  TO chill_app;
GRANT EXECUTE ON FUNCTION control.lookup_user_session(text) TO chill_app;
GRANT EXECUTE ON FUNCTION control.lookup_service_credential(text) TO chill_app;
GRANT EXECUTE ON FUNCTION control.mark_user_session_used(uuid) TO chill_app;
GRANT EXECUTE ON FUNCTION control.mark_service_credential_used(uuid) TO chill_app;

GRANT SELECT, INSERT, UPDATE, DELETE ON
  control.user_identities,
  control.user_sessions,
  control.service_credentials
TO chill_app;

REVOKE TRUNCATE, REFERENCES, TRIGGER ON
  control.user_identities,
  control.user_sessions,
  control.service_credentials
FROM chill_app;
