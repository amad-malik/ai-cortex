-- DATA CONTRACT PACK, piece 3 `mod-tokens` (owner hue; contract author nemo). DRAFT for mike's one pass.
-- Source: S1 §4.1 (the two tables), §4.2 (verification), §4.3 (lifetime), §9 (roles, RLS).
-- Only the SHA-256 digest of a token is stored, never the token (plan 4e, vera promise 3). No pepper (S1 §4.1).
-- Assumes identity.sql ran first (tokens reference identity.workers and identity.projects); roles cortex_owner (runs this file and
-- owns verify_lookup), cortex_m_tokens and cortex_install_admin exist. The lookup function and the owner policies below were smoke-tested
-- on a throwaway Postgres 18.4 with the pack applied as a non-superuser cortex_owner. Postgres apply evidence: see 00-INDEX.md section 7.
CREATE SCHEMA tokens;
REVOKE ALL ON SCHEMA tokens FROM PUBLIC;

CREATE TABLE tokens.tokens (
    id             uuid        PRIMARY KEY,                         -- the token's UUIDv7; also the first half of the bearer value
    worker_id      uuid        NOT NULL,
    project_id     uuid        NOT NULL REFERENCES identity.projects (id) ON DELETE RESTRICT,
    digest         bytea       NOT NULL CHECK (length(digest) = 32),   -- SHA-256 of the full token string, compared in constant time
    install_gen    bigint      NOT NULL,
    project_gen    bigint      NOT NULL,
    issued_at      timestamptz NOT NULL,
    expires_at     timestamptz NOT NULL,
    renew_after    timestamptz NOT NULL,
    last_used_at   timestamptz,
    predecessor_id uuid        REFERENCES tokens.tokens (id) ON DELETE RESTRICT,
    first_used_at  timestamptz,
    revoked_at     timestamptz,
    revoke_reason  text        CHECK (revoke_reason ~ '^[a-z][a-z_]{0,31}$'),   -- a CODE, never free text; S1 names superseded, reissued, retired, replaced_unused
    issued_by      jsonb       NOT NULL DEFAULT '{}',                -- who issued it (actor kind and id); never a secret
    CHECK (expires_at > issued_at),
    CHECK (renew_after >= issued_at AND renew_after <= expires_at),
    CHECK ((revoked_at IS NULL) = (revoke_reason IS NULL)),
    FOREIGN KEY (worker_id, project_id) REFERENCES identity.workers (id, project_id) ON DELETE RESTRICT   -- a token's project is its worker's project (sophi F-8 applied here too)
);
CREATE INDEX tokens_by_worker ON tokens.tokens (worker_id) WHERE revoked_at IS NULL;
CREATE INDEX tokens_by_project ON tokens.tokens (project_id, expires_at);

-- Generation counters (S1 §4.1): 'install' rows have the installation as scope_id, 'project' rows the project.
CREATE TABLE tokens.generations (
    scope    text   NOT NULL CHECK (scope IN ('install', 'project')),
    scope_id uuid   NOT NULL,
    gen      bigint NOT NULL DEFAULT 0,
    PRIMARY KEY (scope, scope_id)
);

-- Row-level security. ORDINARY reads and writes by cortex_m_tokens are held to the project (by_project): an A context cannot see B's
-- tokens. Verification (S1 §4.2) must find a token row by its id BEFORE any project is known (the project comes FROM the token); that
-- one need is met by a PRIVATE one-row lookup function below (vera, row fd2cf846, replacing the earlier blanket SELECT-all policy).
ALTER TABLE tokens.tokens      ENABLE ROW LEVEL SECURITY;  ALTER TABLE tokens.tokens      FORCE ROW LEVEL SECURITY;
ALTER TABLE tokens.generations ENABLE ROW LEVEL SECURITY;  ALTER TABLE tokens.generations FORCE ROW LEVEL SECURITY;

CREATE POLICY by_project ON tokens.tokens
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY install_admin ON tokens.tokens TO cortex_install_admin USING (true) WITH CHECK (true);
-- The function owner (cortex_owner, who runs this file) is subject to FORCE row-level security too, so its read is an explicit,
-- SELECT-only policy. SECURITY DEFINER alone would not bypass the table's row-level security.
CREATE POLICY owner_read ON tokens.tokens FOR SELECT TO cortex_owner USING (true);

CREATE POLICY by_project ON tokens.generations FOR SELECT
    USING (scope = 'install' OR scope_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY install_admin ON tokens.generations TO cortex_install_admin USING (true) WITH CHECK (true);
CREATE POLICY owner_read ON tokens.generations FOR SELECT TO cortex_owner USING (true);

-- The private verification lookup (vera fd2cf846): ONE token by its typed UUID, the verification fields only, nothing else. It is NOT
-- proof of possession: the verifier still compares the full bearer digest in constant time and completes S1 §4.2's checks, and a missing
-- id and a wrong secret must give the same 401 token_invalid. Fixed, schema-qualified query; no dynamic SQL; a trusted search_path;
-- owned by cortex_owner (the role that creates it); EXECUTE revoked from PUBLIC and granted to cortex_m_tokens only. The token
-- handlers must use the authenticated project and must not call it to return arbitrary token metadata (a code rule, tested in S4).
CREATE FUNCTION tokens.verify_lookup(p_token_id uuid)
RETURNS TABLE (id uuid, worker_id uuid, project_id uuid, digest bytea, install_gen bigint, project_gen bigint,
               issued_at timestamptz, expires_at timestamptz, renew_after timestamptz,
               last_used_at timestamptz, first_used_at timestamptz, predecessor_id uuid,
               revoked_at timestamptz, revoke_reason text,
               worker_state text, current_install_gen bigint, current_project_gen bigint)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, pg_temp AS $$
    SELECT t.id, t.worker_id, t.project_id, t.digest, t.install_gen, t.project_gen,
           t.issued_at, t.expires_at, t.renew_after, t.last_used_at, t.first_used_at, t.predecessor_id,
           t.revoked_at, t.revoke_reason,
           w.state,
           (SELECT gi.gen FROM tokens.generations gi WHERE gi.scope = 'install' AND gi.scope_id = p.installation_id),
           (SELECT gp.gen FROM tokens.generations gp WHERE gp.scope = 'project' AND gp.scope_id = t.project_id)
    FROM tokens.tokens t
    JOIN identity.workers  w ON w.id = t.worker_id
    JOIN identity.projects p ON p.id = t.project_id
    WHERE t.id = p_token_id
$$;
REVOKE ALL ON FUNCTION tokens.verify_lookup(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION tokens.verify_lookup(uuid) TO cortex_m_tokens;

-- Grants. No DELETE anywhere (history stays). The module may change only the four fields S1 §4.2 and §4.3 name; the
-- generation counters advance only on host paths (restore, S1 §8.3), so the module role has SELECT only on them.
GRANT USAGE ON SCHEMA tokens TO cortex_m_tokens, cortex_install_admin;
GRANT SELECT, INSERT ON tokens.tokens TO cortex_m_tokens, cortex_install_admin;
GRANT UPDATE (last_used_at, first_used_at, revoked_at, revoke_reason) ON tokens.tokens TO cortex_m_tokens, cortex_install_admin;
GRANT SELECT ON tokens.generations TO cortex_m_tokens;
GRANT SELECT, INSERT, UPDATE ON tokens.generations TO cortex_install_admin;
