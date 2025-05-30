#!/bin/bash

# Optional: Roles
APP_ROLE="app_role"

# --- Run SQL through psql ---
psql -U $POSTGRES_USER -d "$POSTGRES_DB" <<EOF
-- Create roles (if not exist)
DO \$\$
BEGIN
    IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '${APP_ROLE}') THEN
        CREATE ROLE ${APP_ROLE} NOLOGIN;
    END IF;
END
\$\$;

-- Grant app role privileges
GRANT CONNECT ON DATABASE ${POSTGRES_DB} TO ${APP_ROLE};
GRANT USAGE ON SCHEMA public TO ${APP_ROLE};
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO ${APP_ROLE};
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ${APP_ROLE};
EOF