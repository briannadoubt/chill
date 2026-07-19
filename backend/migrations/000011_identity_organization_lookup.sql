CREATE FUNCTION control.lookup_identity_organizations(
  identity_issuer text,
  identity_subject text
)
RETURNS TABLE (organization_id uuid)
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = pg_catalog, control
AS $$
  SELECT membership.organization_id
  FROM control.user_identities AS identity
  JOIN control.users AS account ON account.id = identity.user_id
  JOIN control.organization_memberships AS membership
    ON membership.user_id = identity.user_id
  JOIN control.organizations AS organization
    ON organization.id = membership.organization_id
  WHERE identity.issuer = identity_issuer
    AND identity.subject = identity_subject
    AND account.status = 'active'
    AND membership.status = 'active'
    AND organization.status = 'active'
  ORDER BY membership.organization_id
$$;

REVOKE ALL ON FUNCTION control.lookup_identity_organizations(text, text)
  FROM PUBLIC;
GRANT EXECUTE ON FUNCTION control.lookup_identity_organizations(text, text)
  TO chill_app;
