-- The platform schema (CONTRACTS §3; S2 §5, §6): the migration ledger, platform.now(),
-- platform.idempotency, platform.objects and platform.refs. Run as cortex_owner.
CREATE SCHEMA platform;
REVOKE ALL ON SCHEMA platform FROM PUBLIC;

CREATE TABLE platform.migrations (
    module     text        NOT NULL,
    version    integer     NOT NULL,
    name       text        NOT NULL,
    checksum   text        NOT NULL,
    applied_by text        NOT NULL DEFAULT current_user,
    applied_at timestamptz NOT NULL DEFAULT pg_catalog.now(),
    PRIMARY KEY (module, version)
);

-- The injected clock: ModuleTx::begin sets cortex.now (microseconds since the epoch) for the
-- transaction from the pool's Clock (TestClock in tests; F1-16). Outside a ModuleTx it is the
-- transaction's own time.
CREATE FUNCTION platform.now() RETURNS timestamptz
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$ SELECT coalesce(
        'epoch'::timestamptz
            + nullif(pg_catalog.current_setting('cortex.now', true), '')::bigint * interval '1 microsecond',
        pg_catalog.now()) $$;

-- Idempotency (S2 §5): one row per principal and key, bound to the project, the route and the
-- body hash. Reached only through the helper, under cortex_m_platform (L-7).
CREATE TABLE platform.idempotency (
    id             bigint      GENERATED ALWAYS AS IDENTITY UNIQUE,
    principal_id   uuid        NOT NULL,
    key            text        NOT NULL,
    project_id     uuid,
    route          text        NOT NULL,
    request_sha256 bytea       NOT NULL CHECK (length(request_sha256) = 32),
    status         integer,
    response       jsonb,
    created_at     timestamptz NOT NULL DEFAULT platform.now(),
    PRIMARY KEY (principal_id, key)
);
CREATE INDEX idempotency_created_at ON platform.idempotency (created_at);

-- The object index and references (S2 §6): project-scoped, under the ruled nullif policy (L-7).
CREATE TABLE platform.objects (
    id         uuid        PRIMARY KEY,
    project_id uuid        NOT NULL,
    kind       text        NOT NULL,
    module     text        NOT NULL,
    created_at timestamptz NOT NULL DEFAULT platform.now()
);
CREATE TABLE platform.refs (
    project_id uuid        NOT NULL,
    from_id    uuid        NOT NULL,
    to_id      uuid        NOT NULL,
    kind       text        NOT NULL,
    at         timestamptz NOT NULL DEFAULT platform.now(),
    PRIMARY KEY (from_id, to_id, kind)
);
ALTER TABLE platform.objects ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.objects FORCE ROW LEVEL SECURITY;
ALTER TABLE platform.refs ENABLE ROW LEVEL SECURITY;
ALTER TABLE platform.refs FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON platform.objects
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON platform.refs
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);

-- Exact grants (rework 1, vera's F1B-GREEN-DEFAULT-ACL). cortex_owner's default privileges, for
-- PUBLIC or for a named role, may have added grants to what this step created. So every grant to
-- anyone but the owner, on the schema and on its tables and sequences, is revoked first; the allowed
-- grants follow; and the result is read back below and must match exactly.
DO $$
DECLARE r record;
BEGIN
    FOR r IN SELECT DISTINCT c.oid::pg_catalog.regclass::text AS rel, c.relkind,
                    CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE n.nspname = 'platform' AND a.grantee <> c.relowner LOOP
        EXECUTE format('REVOKE ALL ON %s %s FROM %s',
                       CASE WHEN r.relkind = 'S' THEN 'SEQUENCE' ELSE 'TABLE' END, r.rel, r.who);
    END LOOP;
    FOR r IN SELECT DISTINCT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_namespace n
             CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE n.nspname = 'platform' AND a.grantee <> n.nspowner LOOP
        EXECUTE format('REVOKE ALL ON SCHEMA platform FROM %s', r.who);
    END LOOP;
END $$;

-- Module roles (S1 §9, cortex_m_<module>): INSERT and SELECT on objects and refs, never UPDATE or
-- DELETE (F1-14). The roles that exist when this runs; a module provisioned later gets the same
-- grants from its provisioning step (I-2; kai's ruling on rework 1).
DO $$
DECLARE r text;
BEGIN
    FOR r IN SELECT rolname FROM pg_catalog.pg_roles
             WHERE rolname ~ '^cortex_m_[a-z][a-z0-9_]{0,31}$' ORDER BY rolname LOOP
        EXECUTE format('GRANT USAGE ON SCHEMA platform TO %I', r);
        EXECUTE format('GRANT SELECT, INSERT ON platform.objects, platform.refs TO %I', r);
    END LOOP;
END $$;
GRANT SELECT, INSERT, UPDATE, DELETE ON platform.idempotency TO cortex_m_platform;
-- The migration ledger is readable by ONE role, cortex_m_platform, and only to read (W1-41, cox 14:35Z): the server's `serve` checks it in
-- a transaction under the platform module role. cortex_app and every other module role get nothing; no definer function.
GRANT SELECT ON platform.migrations TO cortex_m_platform;

-- The read-back: every grant to anyone but the owner, on the schema and its tables and sequences,
-- is exactly the allowed set, none grantable; otherwise the step fails and rolls back.
DO $$
DECLARE wrong bigint;
BEGIN
    WITH actual AS (
        SELECT c.relname::text AS obj, a.grantee, a.privilege_type AS priv, a.is_grantable AS grantable
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
        WHERE n.nspname = 'platform' AND a.grantee <> c.relowner
        UNION ALL
        SELECT 'schema', a.grantee, a.privilege_type, a.is_grantable
        FROM pg_catalog.pg_namespace n
        CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a
        WHERE n.nspname = 'platform' AND a.grantee <> n.nspowner
    ),
    modules AS (
        SELECT oid FROM pg_catalog.pg_roles WHERE rolname ~ '^cortex_m_[a-z][a-z0-9_]{0,31}$'
    ),
    allowed AS (
        SELECT 'schema'::text AS obj, m.oid AS grantee, 'USAGE'::text AS priv, false AS grantable FROM modules m
        UNION ALL
        SELECT t.obj, m.oid, p.priv, false
        FROM modules m, (VALUES ('objects'), ('refs')) t(obj), (VALUES ('SELECT'), ('INSERT')) p(priv)
        UNION ALL
        SELECT 'idempotency', 'cortex_m_platform'::regrole::oid, p.priv, false
        FROM (VALUES ('SELECT'), ('INSERT'), ('UPDATE'), ('DELETE')) p(priv)
        UNION ALL
        SELECT 'migrations', 'cortex_m_platform'::regrole::oid, 'SELECT'::text, false
    )
    SELECT count(*) INTO wrong FROM (
        (SELECT * FROM actual EXCEPT SELECT * FROM allowed)
        UNION ALL
        (SELECT * FROM allowed EXCEPT SELECT * FROM actual)
    ) d;
    IF wrong <> 0 THEN
        RAISE EXCEPTION 'platform grants are not exactly the allowed set';
    END IF;
END $$;
