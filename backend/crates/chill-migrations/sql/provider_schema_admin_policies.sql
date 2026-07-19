DO $chill_provider_admin$
DECLARE
  target record;
  administrator name := current_user;
BEGIN
  FOR target IN
    SELECT namespace.nspname AS schema_name, relation.relname AS table_name
    FROM pg_catalog.pg_class AS relation
    JOIN pg_catalog.pg_namespace AS namespace ON namespace.oid = relation.relnamespace
    WHERE relation.relkind IN ('r', 'p')
      AND relation.relrowsecurity
      AND relation.relforcerowsecurity
      AND relation.relowner = (SELECT oid FROM pg_catalog.pg_roles WHERE rolname = administrator)
      AND namespace.nspname IN ('control', 'ingest', 'lake', 'lifecycle', 'compliance', 'product')
      AND NOT EXISTS (
        SELECT 1
        FROM pg_catalog.pg_policy AS policy
        WHERE policy.polrelid = relation.oid
          AND policy.polname = 'chill_schema_admin_policy'
      )
  LOOP
    EXECUTE format(
      'CREATE POLICY chill_schema_admin_policy ON %I.%I TO %I USING (true) WITH CHECK (true)',
      target.schema_name,
      target.table_name,
      administrator
    );
  END LOOP;
END
$chill_provider_admin$;
