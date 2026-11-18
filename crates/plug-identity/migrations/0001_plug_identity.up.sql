-- The plug's minimal identity schema (Wave 0 fixture; socket contract section 3). Run as
-- cortex_owner. The role cortex_m_plug_identity comes from the bootstrap (BOOTSTRAP_SQL), not from
-- this migration.
CREATE SCHEMA plug_identity;
REVOKE ALL ON SCHEMA plug_identity FROM PUBLIC;

CREATE TABLE plug_identity.projects (
    id    uuid PRIMARY KEY,
    name  text NOT NULL,
    state text NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'archived')),
    lead  uuid NOT NULL,
    -- Delta 5 (ProjectView.installation): a fixed UUIDv7 default, so a seed that names no
    -- installation is unchanged. plug_identity.FIXTURE_INSTALLATION is the same text.
    installation_id uuid NOT NULL DEFAULT '01930000-0000-7000-8000-000000000001'
);

CREATE TABLE plug_identity.workers (
    id         uuid PRIMARY KEY,
    project_id uuid NOT NULL REFERENCES plug_identity.projects (id),
    name       text NOT NULL,
    kind       text NOT NULL CHECK (kind IN ('worker', 'service')),
    role       text NOT NULL CHECK (role IN ('project_lead', 'team_lead', 'member')),
    team_id    uuid,
    job_roles  text[] NOT NULL DEFAULT '{}',
    active     boolean NOT NULL DEFAULT true,
    CHECK (role <> 'team_lead' OR team_id IS NOT NULL)
);

-- Project isolation: the transaction's cortex.project_id only (the ruled nullif form, L-7).
ALTER TABLE plug_identity.projects ENABLE ROW LEVEL SECURITY;
ALTER TABLE plug_identity.projects FORCE ROW LEVEL SECURITY;
ALTER TABLE plug_identity.workers ENABLE ROW LEVEL SECURITY;
ALTER TABLE plug_identity.workers FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON plug_identity.projects
    USING (id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON plug_identity.workers
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);

-- The piece reads only. Fixture rows are seeded by the test harness.
GRANT USAGE ON SCHEMA plug_identity TO cortex_m_plug_identity;
GRANT SELECT ON plug_identity.projects, plug_identity.workers TO cortex_m_plug_identity;
