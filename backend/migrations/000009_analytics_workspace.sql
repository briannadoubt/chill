CREATE SCHEMA product;

CREATE TABLE product.saved_queries (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  owner_user_id uuid NOT NULL REFERENCES control.users(id),
  name text NOT NULL,
  description text NOT NULL DEFAULT '',
  plan jsonb NOT NULL,
  visualization text NOT NULL DEFAULT 'table',
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT saved_query_name_bound CHECK (char_length(name) BETWEEN 1 AND 160),
  CONSTRAINT saved_query_description_bound CHECK (char_length(description) <= 2000),
  CONSTRAINT saved_query_plan_object CHECK (jsonb_typeof(plan) = 'object' AND octet_length(plan::text) <= 65536),
  CONSTRAINT saved_query_visualization_value CHECK (visualization IN ('table', 'line', 'bar', 'funnel', 'retention', 'path')),
  CONSTRAINT saved_query_status_value CHECK (status IN ('active', 'archived')),
  CONSTRAINT saved_query_name_unique UNIQUE (environment_id, name),
  CONSTRAINT saved_query_tenant_identity UNIQUE (id, organization_id, project_id, environment_id)
);

CREATE TABLE product.dashboards (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  owner_user_id uuid NOT NULL REFERENCES control.users(id),
  name text NOT NULL,
  description text NOT NULL DEFAULT '',
  sharing text NOT NULL DEFAULT 'organization',
  layout jsonb NOT NULL DEFAULT '[]'::jsonb,
  status text NOT NULL DEFAULT 'active',
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  CONSTRAINT dashboard_name_bound CHECK (char_length(name) BETWEEN 1 AND 160),
  CONSTRAINT dashboard_description_bound CHECK (char_length(description) <= 2000),
  CONSTRAINT dashboard_sharing_value CHECK (sharing IN ('private', 'organization')),
  CONSTRAINT dashboard_layout_array CHECK (jsonb_typeof(layout) = 'array' AND octet_length(layout::text) <= 65536),
  CONSTRAINT dashboard_status_value CHECK (status IN ('active', 'archived')),
  CONSTRAINT dashboard_name_unique UNIQUE (environment_id, name)
);

CREATE TABLE product.alerts (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  organization_id uuid NOT NULL,
  project_id uuid NOT NULL,
  environment_id uuid NOT NULL,
  saved_query_id uuid NOT NULL,
  owner_user_id uuid NOT NULL REFERENCES control.users(id),
  name text NOT NULL,
  operator text NOT NULL,
  threshold double precision NOT NULL,
  schedule_minutes integer NOT NULL,
  status text NOT NULL DEFAULT 'active',
  next_evaluation_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  last_evaluated_at timestamptz,
  last_value double precision,
  last_state text,
  created_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  updated_at timestamptz NOT NULL DEFAULT transaction_timestamp(),
  FOREIGN KEY (environment_id, organization_id, project_id)
    REFERENCES control.environments(id, organization_id, project_id),
  FOREIGN KEY (saved_query_id, organization_id, project_id, environment_id)
    REFERENCES product.saved_queries(id, organization_id, project_id, environment_id),
  CONSTRAINT alert_name_bound CHECK (char_length(name) BETWEEN 1 AND 160),
  CONSTRAINT alert_operator_value CHECK (operator IN ('gt', 'gte', 'lt', 'lte', 'eq')),
  CONSTRAINT alert_threshold_finite CHECK (
    threshold NOT IN ('NaN'::double precision, 'Infinity'::double precision, '-Infinity'::double precision)
  ),
  CONSTRAINT alert_schedule_bound CHECK (schedule_minutes BETWEEN 5 AND 10080),
  CONSTRAINT alert_status_value CHECK (status IN ('active', 'paused', 'archived')),
  CONSTRAINT alert_last_state_value CHECK (last_state IS NULL OR last_state IN ('ok', 'triggered', 'error')),
  CONSTRAINT alert_name_unique UNIQUE (environment_id, name)
);

CREATE INDEX saved_queries_scope_index ON product.saved_queries (organization_id, project_id, environment_id, updated_at DESC) WHERE status = 'active';
CREATE INDEX dashboards_scope_index ON product.dashboards (organization_id, project_id, environment_id, updated_at DESC) WHERE status = 'active';
CREATE INDEX alerts_due_index ON product.alerts (next_evaluation_at, id) WHERE status = 'active';

ALTER TABLE product.saved_queries ENABLE ROW LEVEL SECURITY;
ALTER TABLE product.saved_queries FORCE ROW LEVEL SECURITY;
CREATE POLICY saved_queries_tenant_policy ON product.saved_queries USING (organization_id = control.current_organization_id()) WITH CHECK (organization_id = control.current_organization_id());
ALTER TABLE product.dashboards ENABLE ROW LEVEL SECURITY;
ALTER TABLE product.dashboards FORCE ROW LEVEL SECURITY;
CREATE POLICY dashboards_tenant_policy ON product.dashboards USING (organization_id = control.current_organization_id()) WITH CHECK (organization_id = control.current_organization_id());
ALTER TABLE product.alerts ENABLE ROW LEVEL SECURITY;
ALTER TABLE product.alerts FORCE ROW LEVEL SECURITY;
CREATE POLICY alerts_tenant_policy ON product.alerts USING (organization_id = control.current_organization_id()) WITH CHECK (organization_id = control.current_organization_id());

CREATE FUNCTION product.claim_due_alerts(maximum integer)
RETURNS TABLE (
  alert_id text,
  organization_id text,
  project_id text,
  environment_id text,
  plan jsonb,
  operator text,
  threshold double precision,
  schedule_minutes integer
)
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, product
SET row_security = off
AS $$
  WITH due AS MATERIALIZED (
    SELECT alert.id
    FROM product.alerts AS alert
    WHERE alert.status = 'active' AND alert.next_evaluation_at <= clock_timestamp()
    ORDER BY alert.next_evaluation_at, alert.id
    LIMIT LEAST(GREATEST(maximum, 0), 32)
    FOR UPDATE SKIP LOCKED
  ), claimed AS (
    UPDATE product.alerts AS alert
    SET next_evaluation_at = clock_timestamp() + make_interval(mins => alert.schedule_minutes),
        updated_at = clock_timestamp()
    FROM due
    WHERE alert.id = due.id
    RETURNING alert.id, alert.organization_id, alert.project_id, alert.environment_id,
              alert.saved_query_id, alert.operator, alert.threshold, alert.schedule_minutes
  )
  SELECT claimed.id::text, claimed.organization_id::text, claimed.project_id::text,
         claimed.environment_id::text, saved.plan, claimed.operator, claimed.threshold,
         claimed.schedule_minutes
  FROM claimed
  JOIN product.saved_queries AS saved ON saved.id = claimed.saved_query_id
  WHERE saved.status = 'active';
$$;

CREATE FUNCTION product.complete_alert_evaluation(
  target_id uuid,
  evaluated_value double precision,
  evaluated_state text
)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET search_path = pg_catalog, product
SET row_security = off
AS $$
  WITH changed AS (
    UPDATE product.alerts
    SET last_evaluated_at = clock_timestamp(),
        last_value = CASE WHEN evaluated_state = 'error' THEN NULL ELSE evaluated_value END,
        last_state = evaluated_state,
        updated_at = clock_timestamp()
    WHERE id = target_id
      AND status = 'active'
      AND evaluated_state IN ('ok', 'triggered', 'error')
      AND (evaluated_state = 'error' OR evaluated_value NOT IN (
        'NaN'::double precision, 'Infinity'::double precision, '-Infinity'::double precision
      ))
    RETURNING 1
  )
  SELECT EXISTS(SELECT 1 FROM changed);
$$;

GRANT USAGE ON SCHEMA product TO chill_app;
GRANT SELECT, INSERT, UPDATE ON product.saved_queries, product.dashboards, product.alerts TO chill_app;
REVOKE ALL ON FUNCTION product.claim_due_alerts(integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION product.complete_alert_evaluation(uuid, double precision, text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION product.claim_due_alerts(integer) TO chill_app;
GRANT EXECUTE ON FUNCTION product.complete_alert_evaluation(uuid, double precision, text) TO chill_app;
REVOKE DELETE, TRUNCATE, REFERENCES, TRIGGER ON ALL TABLES IN SCHEMA product FROM chill_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA product REVOKE ALL ON TABLES FROM PUBLIC;
