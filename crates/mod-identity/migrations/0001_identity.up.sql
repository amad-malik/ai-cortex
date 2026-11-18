-- DATA CONTRACT PACK, piece 2 `mod-identity` (owner sophi; contract author nemo). DRAFT for mike's one pass.
-- Source: S1 §3.3 (tables), §3.4 (rename/retire), §9 (roles, RLS); S3 §3 (job_roles, mode: kai's ruling 5 Nov 20:10Z).
-- Plain tables, no partitioning. Every project-scoped table: ENABLE + FORCE row-level security, policy by_project
-- (the platform's nullif form, L-7), plus the host path: cortex_install_admin sees all rows (S1 §9).
-- Postgres apply evidence: see 00-INDEX.md section 7.
-- Assumes: schema platform exists (platform.sql); roles cortex_owner (runs this file), cortex_m_identity and
-- cortex_install_admin exist (the socket's provisioning creates them before this file runs).
CREATE SCHEMA identity;
REVOKE ALL ON SCHEMA identity FROM PUBLIC;

-- One row per install (S1 §3.3).
CREATE TABLE identity.installations (
    id         uuid        PRIMARY KEY,
    label      text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT platform.now()
);
-- sophi F-4: one installation per database in 0.2.001; a second row is refused.
CREATE UNIQUE INDEX installations_singleton ON identity.installations ((true));

-- A project; its lead is permanent once set (trigger below). The lead row is inserted in the same transaction,
-- so the foreign key is deferred.
CREATE TABLE identity.projects (
    id                  uuid        PRIMARY KEY,
    installation_id     uuid        NOT NULL REFERENCES identity.installations (id) ON DELETE RESTRICT,
    lead_worker_id      uuid        NOT NULL,
    token_lifetime_days integer     NOT NULL DEFAULT 90 CHECK (token_lifetime_days BETWEEN 7 AND 365),
    state               text        NOT NULL DEFAULT 'open' CHECK (state IN ('open', 'sealed', 'restoring', 'finishing')),
    created_at          timestamptz NOT NULL DEFAULT platform.now()
);

CREATE TABLE identity.teams (
    id         uuid        PRIMARY KEY,
    project_id uuid        NOT NULL REFERENCES identity.projects (id) ON DELETE RESTRICT,
    state      text        NOT NULL DEFAULT 'active',   -- S1 §3.3 leaves the value list open; only 'active' in 0.2.001
    created_at timestamptz NOT NULL DEFAULT platform.now(),
    UNIQUE (id, project_id)                                 -- target of the composite foreign key from team_members (sophi F-8)
);

CREATE TABLE identity.workers (
    id              uuid        PRIMARY KEY,
    project_id      uuid        NOT NULL REFERENCES identity.projects (id) ON DELETE RESTRICT,
    kind            text        NOT NULL CHECK (kind IN ('worker', 'service')),
    state           text        NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'retired')),
    created_at      timestamptz NOT NULL DEFAULT platform.now(),
    retired_at      timestamptz,
    -- sophi F-9 (revision 5): the reason the retire route requires, free text, 1 to 512 bytes; audit holds codes only. It exists only on a
    -- retired worker (pack choice, sophi to confirm). Nullable for the worker who is not retired.
    retired_reason  text        CHECK (retired_reason IS NULL OR (state = 'retired' AND octet_length(retired_reason) BETWEEN 1 AND 512)),
    perm_gen        bigint      NOT NULL DEFAULT 0,
    token_path_hint text,                                -- services: where the host writes the token (S1 §3.3)
    -- ADDED (kai, 5 Nov 20:10Z; SPEC Q5, S3 §3): job roles and mode. Job roles grant no scope; they decide who may claim work
    -- addressed to a role. Labels match ^[a-z][a-z0-9-]{0,31}$; changed only by team:assign, with an audit record.
    job_roles       text[]      NOT NULL DEFAULT '{}'
        CHECK (array_position(job_roles, NULL) IS NULL
               AND (cardinality(job_roles) = 0
                    OR (array_to_string(job_roles, ',') ~ '^[a-z][a-z0-9-]{0,31}(,[a-z][a-z0-9-]{0,31})*$'
                        -- sophi F-12 (revision 6): the test above runs on the roles JOINED with commas, so ONE element 'qa,dev' passed it.
                        -- Splitting the joined text back must give the same number of elements, i.e. no element holds a comma.
                        AND cardinality(string_to_array(array_to_string(job_roles, ','), ',')) = cardinality(job_roles))))
        CHECK (cardinality(job_roles) <= 8),                -- sophi ID-12 (cox 2389cc45): at most 8 job roles per worker
    mode            text        NOT NULL DEFAULT 'model' CHECK (mode IN ('model', 'deterministic')),   -- default is the pack's choice (S3 §3 names no default)
    CHECK ((state = 'retired') = (retired_at IS NOT NULL)),
    UNIQUE (id, project_id)                                 -- target of the composite foreign key from team_members (sophi F-8)
);

ALTER TABLE identity.projects
    ADD CONSTRAINT projects_lead_fk FOREIGN KEY (lead_worker_id) REFERENCES identity.workers (id)
    ON DELETE RESTRICT DEFERRABLE INITIALLY DEFERRED;

-- project_id is ADDED to team_members and names (S1 §3.3 omits it) so that row-level security has a column to filter on.
-- COMPOSITE foreign keys (sophi F-8, mike): a membership in project A cannot point at project B's worker or team, because the
-- (id, project_id) pair must exist in the referenced table.
CREATE TABLE identity.team_members (
    worker_id  uuid        PRIMARY KEY,
    team_id    uuid        NOT NULL,
    project_id uuid        NOT NULL REFERENCES identity.projects (id) ON DELETE RESTRICT,
    role       text        NOT NULL CHECK (role IN ('member', 'team_lead')),
    since      timestamptz NOT NULL DEFAULT platform.now(),
    FOREIGN KEY (worker_id, project_id) REFERENCES identity.workers (id, project_id) ON DELETE RESTRICT,
    FOREIGN KEY (team_id, project_id)   REFERENCES identity.teams (id, project_id)   ON DELETE RESTRICT
);
-- sophi F-1 (SPEC section 15 Q4): exactly one team lead per team.
CREATE UNIQUE INDEX one_team_lead ON identity.team_members (team_id) WHERE role = 'team_lead';

-- Names: a project, team or worker's name is the row with current = true; old names stay reserved (S1 §3.4).
-- entity_kind: 'project', 'team' or 'worker'. scope_id: the installation (for projects) or the project (for teams and workers).
-- project_id: the entity's project (for a project name, the project itself).
CREATE TABLE identity.names (
    id          bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,   -- ADDED: S1 §3.3 names no key; a surrogate survives a rename in the same transaction
    entity_kind text        NOT NULL CHECK (entity_kind IN ('project', 'team', 'worker')),
    entity_id   uuid        NOT NULL,
    scope_id    uuid        NOT NULL,
    project_id  uuid        NOT NULL,
    name_key    text        NOT NULL CHECK (name_key ~ '^[a-z][a-z0-9-]{0,31}$'),
    label       text        NOT NULL CHECK (lower(label) = name_key),
    -- sophi F-10 (revision 5): the label is ASCII. A Unicode lower() maps the Kelvin sign (U+212A) to 'k', so lower(label) = name_key alone lets
    -- a non-ASCII label through; this check refuses it. Case may differ from name_key; nothing else may.
    CONSTRAINT names_label_ascii CHECK (label ~ '^[A-Za-z][A-Za-z0-9-]{0,31}$'),
    current     boolean     NOT NULL,
    valid_from  timestamptz NOT NULL DEFAULT platform.now(),
    valid_to    timestamptz,
    reserved    boolean     NOT NULL DEFAULT false
);
CREATE UNIQUE INDEX names_unique ON identity.names (entity_kind, scope_id, name_key) WHERE current OR reserved;
CREATE INDEX names_by_entity ON identity.names (entity_kind, entity_id) WHERE current;

CREATE INDEX workers_by_project ON identity.workers (project_id, state);
CREATE INDEX team_members_by_team ON identity.team_members (team_id);
CREATE INDEX teams_by_project ON identity.teams (project_id);

-- Permanence (S1 §3.3, C2): the project lead is set once; a lead cannot be retired; nothing is deleted.
CREATE FUNCTION identity.refuse_lead_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.lead_worker_id IS DISTINCT FROM OLD.lead_worker_id THEN
        RAISE EXCEPTION 'identity.projects.lead_worker_id is permanent' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER projects_lead_permanent BEFORE UPDATE ON identity.projects
    FOR EACH ROW EXECUTE FUNCTION identity.refuse_lead_change();

CREATE FUNCTION identity.refuse_lead_retire() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.state = 'retired' AND OLD.state <> 'retired'
       AND EXISTS (SELECT 1 FROM identity.projects p WHERE p.lead_worker_id = NEW.id) THEN
        RAISE EXCEPTION 'a project lead cannot be retired' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER workers_lead_not_retired BEFORE UPDATE ON identity.workers
    FOR EACH ROW EXECUTE FUNCTION identity.refuse_lead_retire();

-- sophi F-7: retirement is one-way; a retired worker is never set back to active.
CREATE FUNCTION identity.refuse_unretire() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.state = 'retired' AND NEW.state <> 'retired' THEN
        RAISE EXCEPTION 'a retired worker cannot be reactivated' USING ERRCODE = '23514';
    END IF;
    -- sophi's point (2), revision 5: once retired, the time and the reason are fixed (no route changes them; this keeps it so).
    IF OLD.state = 'retired' AND (NEW.retired_at IS DISTINCT FROM OLD.retired_at OR NEW.retired_reason IS DISTINCT FROM OLD.retired_reason) THEN
        RAISE EXCEPTION 'the retirement time and reason of a retired worker cannot change' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER workers_retire_is_one_way BEFORE UPDATE ON identity.workers
    FOR EACH ROW EXECUTE FUNCTION identity.refuse_unretire();

-- sophi F-2 (SPEC section 15 Q4): the project lead stays outside teams; it has no team_members row.
CREATE FUNCTION identity.refuse_lead_in_team() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM identity.projects p WHERE p.lead_worker_id = NEW.worker_id) THEN
        RAISE EXCEPTION 'the project lead cannot be a team member' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER team_members_not_the_lead BEFORE INSERT OR UPDATE OF worker_id ON identity.team_members
    FOR EACH ROW EXECUTE FUNCTION identity.refuse_lead_in_team();

-- sophi F-8 for names: the entity is polymorphic (project, team or worker), so a composite foreign key cannot express the tie.
-- The row's project_id must be the entity's own project; a project's name has project_id equal to the project itself.
CREATE FUNCTION identity.check_name_entity() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE ok boolean;
BEGIN
    IF NEW.entity_kind = 'project' THEN
        ok := (NEW.entity_id = NEW.project_id);
    ELSIF NEW.entity_kind = 'team' THEN
        ok := EXISTS (SELECT 1 FROM identity.teams t WHERE t.id = NEW.entity_id AND t.project_id = NEW.project_id);
    ELSE
        ok := EXISTS (SELECT 1 FROM identity.workers w WHERE w.id = NEW.entity_id AND w.project_id = NEW.project_id);
    END IF;
    IF NOT ok THEN
        RAISE EXCEPTION 'identity.names.project_id does not match the named entity' USING ERRCODE = '23503';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER names_entity_in_project BEFORE INSERT OR UPDATE OF entity_kind, entity_id, project_id ON identity.names
    FOR EACH ROW EXECUTE FUNCTION identity.check_name_entity();

-- Row-level security. projects is filtered by its own id; installations is install-level (host paths only).
ALTER TABLE identity.installations ENABLE ROW LEVEL SECURITY;  ALTER TABLE identity.installations FORCE ROW LEVEL SECURITY;
ALTER TABLE identity.projects      ENABLE ROW LEVEL SECURITY;  ALTER TABLE identity.projects      FORCE ROW LEVEL SECURITY;
ALTER TABLE identity.teams         ENABLE ROW LEVEL SECURITY;  ALTER TABLE identity.teams         FORCE ROW LEVEL SECURITY;
ALTER TABLE identity.workers       ENABLE ROW LEVEL SECURITY;  ALTER TABLE identity.workers       FORCE ROW LEVEL SECURITY;
ALTER TABLE identity.team_members  ENABLE ROW LEVEL SECURITY;  ALTER TABLE identity.team_members  FORCE ROW LEVEL SECURITY;
ALTER TABLE identity.names         ENABLE ROW LEVEL SECURITY;  ALTER TABLE identity.names         FORCE ROW LEVEL SECURITY;

CREATE POLICY by_project ON identity.projects
    USING (id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON identity.teams
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON identity.workers
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON identity.team_members
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON identity.names
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
-- Host paths (create the install, create a project, list across projects): S1 §9 cortex_install_admin sees everything.
CREATE POLICY install_admin ON identity.installations TO cortex_install_admin USING (true) WITH CHECK (true);
CREATE POLICY install_admin ON identity.projects      TO cortex_install_admin USING (true) WITH CHECK (true);
CREATE POLICY install_admin ON identity.teams         TO cortex_install_admin USING (true) WITH CHECK (true);
CREATE POLICY install_admin ON identity.workers       TO cortex_install_admin USING (true) WITH CHECK (true);
CREATE POLICY install_admin ON identity.team_members  TO cortex_install_admin USING (true) WITH CHECK (true);
CREATE POLICY install_admin ON identity.names         TO cortex_install_admin USING (true) WITH CHECK (true);
-- The private token-verification lookup (tokens.sql, vera fd2cf846) is a SECURITY DEFINER function owned by cortex_owner, which is
-- subject to FORCE row-level security like everyone else: it needs an explicit, SELECT-only policy on the two tables the lookup reads.
CREATE POLICY owner_read ON identity.workers  FOR SELECT TO cortex_owner USING (true);
CREATE POLICY owner_read ON identity.projects FOR SELECT TO cortex_owner USING (true);

-- Grants (S1 §9: DML on its own schema only; nothing here is ever deleted). UPDATE is granted per COLUMN (sophi F-6), so an immutable
-- column (id, project_id, kind, created_at, installation_id, lead_worker_id) cannot be changed by SQL either.
GRANT USAGE ON SCHEMA identity TO cortex_m_identity, cortex_install_admin;
GRANT SELECT, INSERT ON identity.installations, identity.projects, identity.teams, identity.workers,
      identity.team_members, identity.names TO cortex_m_identity, cortex_install_admin;
GRANT UPDATE (label) ON identity.installations TO cortex_m_identity, cortex_install_admin;
GRANT UPDATE (token_lifetime_days, state) ON identity.projects TO cortex_m_identity, cortex_install_admin;
GRANT UPDATE (state) ON identity.teams TO cortex_m_identity, cortex_install_admin;
GRANT UPDATE (state, retired_at, retired_reason, perm_gen, job_roles, mode, token_path_hint) ON identity.workers TO cortex_m_identity, cortex_install_admin;
GRANT UPDATE (team_id, role) ON identity.team_members TO cortex_m_identity, cortex_install_admin;
GRANT UPDATE ("current", valid_to, reserved) ON identity.names TO cortex_m_identity, cortex_install_admin;
