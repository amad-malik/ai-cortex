-- DATA CONTRACT PACK, piece 5 `mod-audit` (owner marv; contract author nemo). DRAFT for mike's one pass.
-- Source: S1 §6 (audit.records), plan piece 5: "append-only log of privileged actions (plain, no hash chain yet)".
-- PLAIN means: the S1 columns chain_id, seq, prev_hash and hash are NOT in 0.2.001. They arrive in 0.2.026 (ladder r3) as an additive
-- migration that backfills a chain over the existing rows in (at, id) order; nothing else in this table changes.
-- Append-only is enforced in the database: the module role holds INSERT and SELECT only, and a trigger refuses UPDATE and
-- DELETE for every role (S1 §6). `details` never holds a secret, token or free-text reason (kai ruling Q-7 for lineage; S1 §6).
-- Writers reach this table only through the audit module's AuditSink, inside the caller's transaction (CONTRACTS §9.1), so a
-- failed audit append rolls the whole operation back (vera promise 2). Assumes the roles cortex_m_audit and cortex_install_admin.
CREATE SCHEMA audit;
REVOKE ALL ON SCHEMA audit FROM PUBLIC;

CREATE TABLE audit.records (
    id              uuid        PRIMARY KEY,                       -- UUIDv7 from the application; time-ordered
    installation_id uuid        NOT NULL,
    project_id      uuid,                                          -- NULL for an install-level record (host operations)
    actor_kind      text        NOT NULL CHECK (actor_kind IN ('worker', 'service', 'human', 'system', 'host')),
    actor_id        text,                                          -- worker or service id, '<source>:<id>' for a human, module id for system; NULL for a host operator
    actor_uid       bigint,                                        -- host operator's OS uid (host actions only; a u32, so a bigint)
    action          text        NOT NULL CHECK (action ~ '^[a-z][a-z0-9_.:-]{0,63}$'),   -- a code, e.g. record.supersede, section.write_reserved, token.issue
    target          jsonb       NOT NULL DEFAULT '{}',
    outcome         text        NOT NULL CHECK (outcome IN ('allowed', 'denied')),
    request_id      uuid,
    details         jsonb       NOT NULL DEFAULT '{}',
    at              timestamptz NOT NULL DEFAULT platform.now(),
    CHECK (octet_length(details::text) <= 16384),
    CHECK (octet_length(target::text) <= 4096)
);
CREATE INDEX audit_by_project ON audit.records (project_id, at DESC);
CREATE INDEX audit_by_action  ON audit.records (project_id, action, at DESC);

CREATE FUNCTION audit.refuse_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'audit.records is append-only' USING ERRCODE = '42501';
END $$;
CREATE TRIGGER records_append_only BEFORE UPDATE OR DELETE ON audit.records
    FOR EACH ROW EXECUTE FUNCTION audit.refuse_change();

-- A project's lead reads the project chain (audit:read); the host reads every record, install-level ones included.
ALTER TABLE audit.records ENABLE ROW LEVEL SECURITY;  ALTER TABLE audit.records FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON audit.records
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY install_admin ON audit.records TO cortex_install_admin USING (true) WITH CHECK (true);
-- Install-level records (project_id IS NULL) are written by host-socket code paths running as cortex_install_admin, which
-- the policy above covers; no module role may write or read them.

-- W1-36 / AUDIT-V1: creator defaults can grant extra rights (including
-- TRUNCATE, which ignores row security). Normalize only this piece's objects.
DO $$
DECLARE r record;
BEGIN
    FOR r IN SELECT DISTINCT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_class c
             CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE c.oid = 'audit.records'::regclass AND a.grantee <> c.relowner LOOP
        EXECUTE format('REVOKE ALL ON TABLE audit.records FROM %s', r.who);
    END LOOP;
    FOR r IN SELECT DISTINCT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_proc p
             CROSS JOIN LATERAL pg_catalog.aclexplode(p.proacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE p.oid = 'audit.refuse_change()'::regprocedure AND a.grantee <> p.proowner LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION audit.refuse_change() FROM %s', r.who);
    END LOOP;
    FOR r IN SELECT DISTINCT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_namespace n
             CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE n.nspname = 'audit' AND a.grantee <> n.nspowner LOOP
        EXECUTE format('REVOKE ALL ON SCHEMA audit FROM %s', r.who);
    END LOOP;
END $$;
-- A NULL function ACL grants PUBLIC EXECUTE by default.
REVOKE EXECUTE ON FUNCTION audit.refuse_change() FROM PUBLIC;
GRANT USAGE ON SCHEMA audit TO cortex_m_audit, cortex_install_admin;
GRANT SELECT, INSERT ON audit.records TO cortex_m_audit, cortex_install_admin;

-- Fail closed if any grant is extra, missing, or carries grant option.
DO $$
DECLARE wrong bigint;
BEGIN
    WITH actual AS (
        SELECT 'records'::text AS obj, a.grantee, a.privilege_type AS priv, a.is_grantable AS grantable
        FROM pg_catalog.pg_class c
        CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(c.relacl, pg_catalog.acldefault('r', c.relowner))) a
        WHERE c.oid = 'audit.records'::regclass AND a.grantee <> c.relowner
        UNION ALL
        SELECT 'schema', a.grantee, a.privilege_type, a.is_grantable
        FROM pg_catalog.pg_namespace n
        CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(n.nspacl, pg_catalog.acldefault('n', n.nspowner))) a
        WHERE n.nspname = 'audit' AND a.grantee <> n.nspowner
        UNION ALL
        SELECT 'refuse_change()', a.grantee, a.privilege_type, a.is_grantable
        FROM pg_catalog.pg_proc p
        CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(p.proacl, pg_catalog.acldefault('f', p.proowner))) a
        WHERE p.oid = 'audit.refuse_change()'::regprocedure AND a.grantee <> p.proowner
    ), allowed AS (
        SELECT v.obj, r.oid AS grantee, v.priv
        FROM (VALUES
            ('schema', 'cortex_m_audit', 'USAGE'), ('schema', 'cortex_install_admin', 'USAGE'),
            ('records', 'cortex_m_audit', 'SELECT'), ('records', 'cortex_m_audit', 'INSERT'),
            ('records', 'cortex_install_admin', 'SELECT'), ('records', 'cortex_install_admin', 'INSERT')
        ) AS v(obj, role, priv)
        JOIN pg_catalog.pg_roles r ON r.rolname = v.role
    )
    SELECT count(*) INTO wrong FROM (
        (SELECT obj, grantee, priv FROM actual EXCEPT SELECT obj, grantee, priv FROM allowed)
        UNION ALL
        (SELECT obj, grantee, priv FROM allowed EXCEPT SELECT obj, grantee, priv FROM actual)
        UNION ALL
        (SELECT obj, grantee, priv FROM actual WHERE grantable)
    ) differences;
    IF wrong <> 0 THEN
        RAISE EXCEPTION 'audit: object grants differ from the allowed set (% differences)', wrong
            USING ERRCODE = 'insufficient_privilege';
    END IF;
END $$;
