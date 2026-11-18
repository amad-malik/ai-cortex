CREATE SCHEMA records;
REVOKE ALL ON SCHEMA records FROM PUBLIC;

-- The two functions behind the generated search columns (nix, 5 Nov 20:51Z). Both are IMMUTABLE, as a generation expression
-- requires. 200000 is the cut in characters; if the insert-safety corpus (mod-search MS-13) fails after the freeze, it drops to
-- 100000 in these two functions only. The exception handler catches 54000 program_limit_exceeded, the code Postgres raises when a
-- tsvector would exceed 1 MiB. SMOKE-TESTED on a throwaway Postgres 18.4 (00-INDEX.md section 7): 200,000-character CJK bodies insert, an
-- oversized tsvector raises 54000, and a copy of this fallback shape returns the summary vector. Nix's corpus MS-13 runs at the authoritative apply.
CREATE FUNCTION records.search_vector(summary text, body text) RETURNS tsvector
    LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE AS $$
BEGIN
    RETURN setweight(to_tsvector('english'::regconfig, summary), 'A') ||
           setweight(to_tsvector('english'::regconfig, left(coalesce(body, ''), 200000)), 'B');
EXCEPTION WHEN program_limit_exceeded THEN
    RETURN setweight(to_tsvector('english'::regconfig, summary), 'A');
END $$;

CREATE FUNCTION records.search_vector_partial(summary text, body text) RETURNS boolean
    LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE AS $$
BEGIN
    IF char_length(coalesce(body, '')) > 200000 THEN
        RETURN true;
    END IF;
    PERFORM setweight(to_tsvector('english'::regconfig, summary), 'A') ||
            setweight(to_tsvector('english'::regconfig, coalesce(body, '')), 'B');
    RETURN false;
EXCEPTION WHEN program_limit_exceeded THEN
    RETURN true;
END $$;

CREATE TABLE records.records (
    id             uuid        NOT NULL,                     -- UUIDv7 made by the server (monotonic generator: F-1A6, not needed in 0.2.001)
    project_id     uuid        NOT NULL,
    created_at     timestamptz NOT NULL,                     -- the timestamp inside id, so an id alone says when it was written
    type           text        NOT NULL CHECK (type IN ('decision', 'lesson', 'note', 'status', 'question', 'blocker', 'knowledge',
                                                         'conversation', 'work-product', 'human-message', 'drill-canary', 'drill')),
    subtype        text,
    provenance     text        NOT NULL CHECK (provenance IN ('worker_written', 'kos_attested', 'host_attested', 'imported', 'system_written')),
    author_kind    text        NOT NULL CHECK (author_kind IN ('worker', 'service', 'human', 'system')),
    author_id      text        NOT NULL,                     -- worker uuid, '<source>:<id>' for a human, module id for the system
    written_by     uuid        NOT NULL,                     -- the principal that submitted the record
    summary        text        NOT NULL CHECK (octet_length(summary) BETWEEN 1 AND 512),
    body           text        CHECK (body IS NULL OR (body <> '' AND octet_length(body) <=
                                      CASE WHEN type = 'conversation' THEN 4194304 ELSE 262144 END)),   -- an empty body is stored as NULL; 256 KiB, 4 MiB for conversation
    summary_sha256 bytea       NOT NULL CHECK (length(summary_sha256) = 32),     -- all three set by mod-records at insert (S2 §3.9)
    body_sha256    bytea       NOT NULL CHECK (length(body_sha256) = 32),        -- sha256 of zero bytes when there is no body
    fields_sha256  bytea       NOT NULL CHECK (length(fields_sha256) = 32),      -- sha256 of RFC 8785 canonical JSON of fields
    fields         jsonb       NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(fields) = 'object' AND octet_length(fields::text) <= 65536),
    reply_to       uuid,
    handoff_id     uuid,
    section        text        CHECK (section IS NULL OR octet_length(section) BETWEEN 1 AND 128),
    section_revision integer   CHECK (section_revision IS NULL OR section_revision >= 1),   -- ruling Q-2: the number is stored at write time under the section's row lock
    valid_until    timestamptz,
    -- Knowledge source (kai ruling 2a7de258, D-13, relayed by cox d5dabddc): 0.2.001 requires a MINIMAL source {kind, ref} with no span.
    -- A recorded DEVIATION from S2 §3.1/§3.6 (which require a span; the release that adds span validation follows mod-import-v1 (kai places it right after it; the ladder renumbers, r4, after the freeze, so it is cited by name)). source_span
    -- is the nullable column the later release fills with S2 §3.4's string (rec:<id>#<part><start>-<end>@<hash16>), so it adds validation, not columns. The app writes
    -- NULL until then. kind 'worker': ref is a worker id; 'conversation': a conversation record id; 'file': a path; 'url': a URL.
    -- The reserved sections (rules-project, rules-worker:*, and from revision 6 orientation) are the one exception that waives the source (ruling Q-2), checked in the constraint below.
    source_kind    text        CHECK (source_kind IN ('conversation', 'file', 'url', 'worker')),
    source_ref     text        CHECK (source_ref IS NULL OR octet_length(source_ref) BETWEEN 1 AND 2048),
    source_span    text        CHECK (source_span IS NULL OR octet_length(source_span) BETWEEN 1 AND 512),
    -- Knowledge owner (kai 5c219116, S3 r2.1; revision 5): S2 §3.1/§3.6 require an active owner worker. In 0.2.001 the AUTHOR is the owner and
    -- the app writes NULL here, so an explicit owner later is additive (kai: revisit with the span work). A worker id; no foreign key (header).
    owner          uuid,
    -- Full-text vector for mod-search (nix): summary weighted A, the first 200,000 characters of the body weighted B. Truncated on
    -- purpose: a tsvector cannot exceed 1 MiB and a record body may be 256 KiB (a 4 MiB conversation would fail on insert).
    -- INSERT-SAFE (nix condition b, 5 Nov 20:51Z): the vector comes from records.search_vector(), which falls back to the summary
    -- alone if even the cut text would overflow the limit, so a generated column can never turn a valid record into an error.
    -- tsv_truncated (nix condition a) is true when the body is longer than the cut OR the fallback was used, so search can say how
    -- many records are only partly searched. Chunked indexing of the rest is a later release.
    tsv            tsvector    GENERATED ALWAYS AS (records.search_vector(summary, body)) STORED,
    tsv_truncated  boolean     GENERATED ALWAYS AS (records.search_vector_partial(summary, body)) STORED,
    PRIMARY KEY (project_id, id),
    CHECK (created_at = uuid_extract_timestamp(id)),
    CHECK ((section IS NULL) = (section_revision IS NULL)),
    CHECK ((source_kind IS NULL) = (source_ref IS NULL)),
    CHECK (source_span IS NULL OR source_kind IS NOT NULL),
    CHECK (type = 'knowledge' OR source_kind IS NULL),
    CHECK (type = 'knowledge' OR owner IS NULL),
    CHECK (type <> 'knowledge' OR source_kind IS NOT NULL
           OR coalesce(section = 'rules-project' OR section LIKE 'rules-worker:%' OR section = 'orientation', false))   -- revision 6: orientation waives the source too (cox f1995402, S3 CHANGES section 19)
);
CREATE INDEX records_by_time    ON records.records (project_id, created_at, id);
CREATE INDEX records_by_handoff ON records.records (project_id, handoff_id) WHERE handoff_id IS NOT NULL;
CREATE UNIQUE INDEX records_section_revision ON records.records (project_id, section, section_revision) WHERE section IS NOT NULL;
CREATE INDEX records_tsv ON records.records USING gin (tsv);

CREATE FUNCTION records.refuse_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'records.records rows are immutable' USING ERRCODE = '42501';
END $$;
CREATE TRIGGER records_immutable BEFORE UPDATE ON records.records
    FOR EACH ROW EXECUTE FUNCTION records.refuse_update();

-- Lineage state (S2 §3.2, §3.5): a missing row means active. After an undo the row stays, with state 'active'.
-- No foreign key to records.records: what retention does to lineage rows is open (M-5, ruling Q-9).
CREATE TABLE records.state (
    project_id    uuid        NOT NULL,
    record_id     uuid        NOT NULL,
    state         text        NOT NULL CHECK (state IN ('active', 'superseded', 'invalidated')),
    superseded_by uuid,
    reason        text        NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 512),   -- revision 5 (cox 4074afd1, kai): free text, required for supersede, invalidate AND undo
    by_kind       text        NOT NULL CHECK (by_kind IN ('worker', 'service', 'human', 'system', 'host')),
    by_id         text        NOT NULL,
    at            timestamptz NOT NULL DEFAULT platform.now(),
    PRIMARY KEY (project_id, record_id),
    CHECK ((state = 'superseded') = (superseded_by IS NOT NULL)),
    CHECK (superseded_by IS NULL OR superseded_by <> record_id)
);

-- One row per lineage change, append-only (ruling Q-5). seq is gapless per project and comes from lineage_counters. REVISION 8 (hale Q-1,
-- cox W1-36): every lineage change takes the project's counter row FIRST (so lineage changes of a project serialise and a replacement
-- cannot be made inactive between its check and the write), then the record's state row, then the section pointer row, the audit row last;
-- a section WRITE takes only the section row, so there is no cycle.
CREATE TABLE records.state_log (
    project_id    uuid        NOT NULL,
    seq           bigint      NOT NULL CHECK (seq >= 1),
    record_id     uuid        NOT NULL,
    from_state    text        NOT NULL CHECK (from_state IN ('active', 'superseded', 'invalidated')),
    to_state      text        NOT NULL CHECK (to_state IN ('active', 'superseded', 'invalidated')),
    superseded_by uuid,
    reason        text        NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 512),   -- revision 5 (cox 4074afd1, kai): free text, required for supersede, invalidate AND undo
    by_kind       text        NOT NULL CHECK (by_kind IN ('worker', 'service', 'human', 'system', 'host')),
    by_id         text        NOT NULL,
    at            timestamptz NOT NULL DEFAULT platform.now(),
    request_id    uuid,
    PRIMARY KEY (project_id, seq),
    CHECK (from_state <> to_state),
    CHECK ((to_state = 'superseded') = (superseded_by IS NOT NULL))
);
CREATE INDEX state_log_by_record ON records.state_log (project_id, record_id, seq);

CREATE TABLE records.lineage_counters (
    project_id uuid   PRIMARY KEY,
    last_seq   bigint NOT NULL DEFAULT 0 CHECK (last_seq >= 0)
);

-- Sections (S2 §3.5): the current revision of a named section. The row is created with INSERT ... ON CONFLICT DO NOTHING and
-- locked FOR UPDATE by a section write (ruling Q-8). current_record_id is NULL when the current revision was invalidated
-- (ruling Q-4: no fallback to an older revision). last_revision gives the next revision number.
CREATE TABLE records.sections (
    project_id        uuid    NOT NULL,
    section           text    NOT NULL CHECK (octet_length(section) BETWEEN 1 AND 128),
    current_record_id uuid,
    last_revision     integer NOT NULL DEFAULT 0 CHECK (last_revision >= 0),
    PRIMARY KEY (project_id, section)
);

CREATE FUNCTION records.refuse_log_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'records.state_log is append-only' USING ERRCODE = '42501';
END $$;
CREATE TRIGGER state_log_append_only BEFORE UPDATE OR DELETE ON records.state_log
    FOR EACH ROW EXECUTE FUNCTION records.refuse_log_change();

-- Row-level security: every table, the platform's nullif form (L-7). Unset or empty setting: no rows.
ALTER TABLE records.records          ENABLE ROW LEVEL SECURITY;  ALTER TABLE records.records          FORCE ROW LEVEL SECURITY;
ALTER TABLE records.state            ENABLE ROW LEVEL SECURITY;  ALTER TABLE records.state            FORCE ROW LEVEL SECURITY;
ALTER TABLE records.state_log        ENABLE ROW LEVEL SECURITY;  ALTER TABLE records.state_log        FORCE ROW LEVEL SECURITY;
ALTER TABLE records.lineage_counters ENABLE ROW LEVEL SECURITY;  ALTER TABLE records.lineage_counters FORCE ROW LEVEL SECURITY;
ALTER TABLE records.sections         ENABLE ROW LEVEL SECURITY;  ALTER TABLE records.sections         FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON records.records
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON records.state
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON records.state_log
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON records.lineage_counters
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON records.sections
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);

-- Grants. The module role cannot UPDATE or DELETE a record, cannot UPDATE or DELETE the log, and deletes nothing.
GRANT USAGE ON SCHEMA records TO cortex_m_records;
GRANT SELECT, INSERT ON records.records, records.state_log TO cortex_m_records;
GRANT SELECT, INSERT, UPDATE ON records.state, records.lineage_counters, records.sections TO cortex_m_records;
-- Public views for mod-search (S2 §3.9, reduced). PROPOSED for 0.2.001, "W14-lite": the accepted design lets S4's keyword
-- query filter on a record's current state inside its own SQL, and the skeleton has no sink hook or events to feed an index.
-- REVISION 7 (cox ruling W1-14, 6 Nov 2026): the two views live INSIDE schema records, as records.pub_sources and
-- records.pub_state (same columns as revision 6's records_pub.sources and records_pub.state). The migration guard (F1-13)
-- refuses a migration step that creates anything outside its module's one schema, so a second schema records_pub cannot be
-- created by the piece's migration. A RECORDED DEVIATION from S2 r9 3.9, which names the schema records_pub.
-- Each view is a security_barrier view owned by the schema owner, filters on the project setting itself (fails closed), and is
-- granted to cortex_m_search only; the search role has USAGE on schema records and SELECT on these two views, nothing else.
CREATE VIEW records.pub_sources WITH (security_barrier = true) AS
    SELECT r.id, r.project_id, r.created_at, r.type, r.subtype, r.provenance, r.author_kind, r.author_id,
           r.summary, r.body, r.fields, r.summary_sha256, r.body_sha256, r.fields_sha256, r.valid_until,
           r.section, r.section_revision, r.source_kind, r.source_ref, r.source_span,
           (r.type = 'drill-canary') AS hidden_by_default, r.tsv, r.tsv_truncated
    FROM records.records r
    WHERE r.project_id = nullif(current_setting('cortex.project_id', true), '')::uuid;
-- One row per record that is NOT active (a missing row means active), as in S2 §3.9.
CREATE VIEW records.pub_state WITH (security_barrier = true) AS
    SELECT s.record_id, s.project_id, s.state, s.superseded_by, s.at AS changed_at
    FROM records.state s
    WHERE s.state <> 'active'
      AND s.project_id = nullif(current_setting('cortex.project_id', true), '')::uuid;
-- Functions (cox condition ii): USAGE on the schema would otherwise let any role that has it call every function through
-- PUBLIC's default EXECUTE. Nothing in the schema is executable through PUBLIC; the module role gets back the two functions the
-- generated columns call (the inserting role needs EXECUTE on them); a trigger function needs no grant to fire.
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA records FROM PUBLIC;
GRANT EXECUTE ON FUNCTION records.search_vector(text, text), records.search_vector_partial(text, text) TO cortex_m_records;
GRANT USAGE ON SCHEMA records TO cortex_m_search;
GRANT SELECT ON records.pub_sources, records.pub_state TO cortex_m_search;

-- Exact grants (REVISION 8: hale F-2, cox W1-36; the pattern of platform 0001, vera's F1B-GREEN-DEFAULT-ACL). cortex_owner's default
-- privileges, for PUBLIC or for a named role, may have added grants to what this step created (tables, views, functions, the schema).
-- Revision 7 gave the search role USAGE on this schema, so the table ACLs are the only barrier. Every grant to anyone but the owner is
-- revoked first; the allowed grants follow; and the result is read back below and must match exactly, or the step fails and rolls back.
DO $$
DECLARE r record;
BEGIN
    FOR r IN SELECT DISTINCT pg_catalog.quote_ident(n.nspname) || '.' || pg_catalog.quote_ident(c.relname) AS rel,
                    CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
             CROSS JOIN LATERAL pg_catalog.aclexplode(c.relacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE n.nspname = 'records' AND c.relkind IN ('r', 'v') AND a.grantee <> c.relowner LOOP
        EXECUTE format('REVOKE ALL ON TABLE %s FROM %s', r.rel, r.who);
    END LOOP;
    FOR r IN SELECT DISTINCT format('%I.%I(%s)', n.nspname, p.proname, pg_catalog.pg_get_function_identity_arguments(p.oid)) AS fn,
                    CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_proc p
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
             CROSS JOIN LATERAL pg_catalog.aclexplode(p.proacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE n.nspname = 'records' AND a.grantee <> p.proowner LOOP
        EXECUTE format('REVOKE ALL ON FUNCTION %s FROM %s', r.fn, r.who);
    END LOOP;
    FOR r IN SELECT DISTINCT CASE WHEN a.grantee = 0 THEN 'PUBLIC' ELSE pg_catalog.quote_ident(g.rolname) END AS who
             FROM pg_catalog.pg_namespace n
             CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a
             LEFT JOIN pg_catalog.pg_roles g ON g.oid = a.grantee
             WHERE n.nspname = 'records' AND a.grantee <> n.nspowner LOOP
        EXECUTE format('REVOKE ALL ON SCHEMA records FROM %s', r.who);
    END LOOP;
END $$;
-- A function whose ACL was never written (NULL) is still executable by PUBLIC: make the revoke explicit for every function.
REVOKE EXECUTE ON ALL FUNCTIONS IN SCHEMA records FROM PUBLIC;
GRANT USAGE ON SCHEMA records TO cortex_m_records, cortex_m_search;
GRANT SELECT, INSERT ON records.records, records.state_log TO cortex_m_records;
GRANT SELECT, INSERT, UPDATE ON records.state, records.lineage_counters, records.sections TO cortex_m_records;
GRANT SELECT ON records.pub_sources, records.pub_state TO cortex_m_search;
GRANT EXECUTE ON FUNCTION records.search_vector(text, text), records.search_vector_partial(text, text) TO cortex_m_records;

-- The read-back: every grant to anyone but the owner, on the schema, its tables, views and functions, is exactly the allowed set, none
-- grantable.
DO $$
DECLARE wrong bigint;
BEGIN
    WITH actual AS (
        SELECT c.relname::text AS obj, a.grantee, a.privilege_type AS priv, a.is_grantable AS grantable
        FROM pg_catalog.pg_class c
        JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
        CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(c.relacl, pg_catalog.acldefault('r', c.relowner))) a
        WHERE n.nspname = 'records' AND c.relkind IN ('r', 'v') AND a.grantee <> c.relowner
        UNION ALL
        SELECT 'schema', a.grantee, a.privilege_type, a.is_grantable
        FROM pg_catalog.pg_namespace n
        CROSS JOIN LATERAL pg_catalog.aclexplode(n.nspacl) a
        WHERE n.nspname = 'records' AND a.grantee <> n.nspowner
        UNION ALL
        SELECT p.proname::text || '()', a.grantee, a.privilege_type, a.is_grantable
        FROM pg_catalog.pg_proc p
        JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace
        CROSS JOIN LATERAL pg_catalog.aclexplode(coalesce(p.proacl, pg_catalog.acldefault('f', p.proowner))) a
        WHERE n.nspname = 'records' AND a.grantee <> p.proowner
    ),
    allowed AS (
        SELECT v.obj, r.oid AS grantee, v.priv
        FROM (VALUES
            ('records', 'cortex_m_records', 'INSERT'), ('records', 'cortex_m_records', 'SELECT'),
            ('state_log', 'cortex_m_records', 'INSERT'), ('state_log', 'cortex_m_records', 'SELECT'),
            ('state', 'cortex_m_records', 'INSERT'), ('state', 'cortex_m_records', 'SELECT'), ('state', 'cortex_m_records', 'UPDATE'),
            ('lineage_counters', 'cortex_m_records', 'INSERT'), ('lineage_counters', 'cortex_m_records', 'SELECT'),
            ('lineage_counters', 'cortex_m_records', 'UPDATE'),
            ('sections', 'cortex_m_records', 'INSERT'), ('sections', 'cortex_m_records', 'SELECT'), ('sections', 'cortex_m_records', 'UPDATE'),
            ('pub_sources', 'cortex_m_search', 'SELECT'), ('pub_state', 'cortex_m_search', 'SELECT'),
            ('schema', 'cortex_m_records', 'USAGE'), ('schema', 'cortex_m_search', 'USAGE'),
            ('search_vector()', 'cortex_m_records', 'EXECUTE'), ('search_vector_partial()', 'cortex_m_records', 'EXECUTE')
        ) AS v(obj, role, priv)
        JOIN pg_catalog.pg_roles r ON r.rolname = v.role
    )
    SELECT count(*) INTO wrong FROM (
        (SELECT obj, grantee, priv FROM actual EXCEPT SELECT obj, grantee, priv FROM allowed)
        UNION ALL
        (SELECT obj, grantee, priv FROM allowed EXCEPT SELECT obj, grantee, priv FROM actual)
        UNION ALL
        (SELECT obj, grantee, priv FROM actual WHERE grantable)
    ) d;
    IF wrong <> 0 THEN
        RAISE EXCEPTION 'records: the grants on the schema, its tables, views or functions differ from the allowed set (% differences)', wrong
            USING ERRCODE = 'insufficient_privilege';
    END IF;
END $$;
