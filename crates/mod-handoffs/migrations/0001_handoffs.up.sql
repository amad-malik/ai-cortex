-- DATA CONTRACT PACK, piece 8 `mod-handoffs` (owner bob; contract author nemo). DRAFT for mike's one pass.
-- Source: S3 §4.1 (packet), §4.2 (incarnation), §4.3 (transitions), §4.4 (return and review), §4.9 (transition log); plan 4a-b
-- (acceptance and verify text shown to the recipient); vera's promise 2 (a claim has one winner; operation and audit roll back together).
-- Plain tables. NOT in 0.2.001 (additive later): checkpoints (suspend/resume), the lease-expiry task's view of token last-seen, human
-- consults' answers, follow-ups, signals, workboard. The state list keeps 'suspended' so those arrive without a migration of the CHECK.
-- No foreign key to identity or records: the project is held by row-level security and the token.
-- CANCEL (kai 5 Nov 20:10Z; cox's CORRECTION 20:31Z, bob confirms): it follows S3 itself, so this file adds no cancelled_* columns:
--   state = 'cancelled', closed_at (when), close_reason (why, required), and the handoffs.transitions row (who: actor_kind and
--   actor_id; why: reason; when: at). Allowed from any open state in 0.2.001 (pending, claimed, returned), by the creator or a lead
--   (the project lead or the team lead concerned), route POST /v2/handoffs/{id}/cancel: rules for bob's code, not DDL.
--   The reasons are TEXT up to 512 bytes, as in S3 (`cancel --reason ...`); an earlier draft of this file forced a code.
-- Assumes platform.sql ran first and the role cortex_m_handoffs exists. Postgres apply evidence: see 00-INDEX.md section 7.
CREATE SCHEMA handoffs;
REVOKE ALL ON SCHEMA handoffs FROM PUBLIC;

CREATE TABLE handoffs.handoffs (
    id               uuid        PRIMARY KEY,
    project_id       uuid        NOT NULL,
    kind             text        NOT NULL CHECK (kind IN ('task', 'consult')),
    state            text        NOT NULL DEFAULT 'pending'
                                 CHECK (state IN ('pending', 'claimed', 'suspended', 'returned', 'accepted', 'cancelled')),
    created_by       uuid        NOT NULL,                       -- a worker or service principal
    to_kind          text        NOT NULL CHECK (to_kind IN ('worker', 'team', 'job_role', 'human')),
    to_ref           text        NOT NULL CHECK (octet_length(to_ref) BETWEEN 1 AND 256),   -- a worker id, a team id, a job-role label or '<source>:<id>'
    holder           uuid,                                       -- NULL while pending
    -- ADDED (bob Q-2, via cox 3a6f3b98): S3 sends a rework back to the SAME worker, but holder is NULL while pending and to_ref must keep
    -- the original addressing. rework_holder is the worker who returned the work; it is set when a return is sent back (round > 1) and
    -- is the one worker who may claim it. NULL on a handoff that was never sent back.
    rework_holder    uuid,
    incarnation      bigint      NOT NULL DEFAULT 0,             -- +1 on every claim; the fence (S3 §4.2)
    lease_ttl_s      integer     NOT NULL DEFAULT 7200 CHECK (lease_ttl_s BETWEEN 300 AND 86400),
    lease_renewed_at timestamptz,
    priority         integer     NOT NULL DEFAULT 50 CHECK (priority BETWEEN 1 AND 100),
    due_at           timestamptz,
    task_id          uuid,
    summary          text        NOT NULL CHECK (octet_length(summary) BETWEEN 1 AND 2048),
    next_action      text        NOT NULL CHECK (octet_length(next_action) BETWEEN 1 AND 8192),
    acceptance       text        NOT NULL CHECK (octet_length(acceptance) BETWEEN 1 AND 8192),
    -- ADDED for v1 command parity (plan 4a-b: `show` displays acceptance criteria AND the verify text to the recipient; the v1
    -- `cortex-handoff` has --verify and --context and S3's packet has neither). bob confirms; both are optional, 8 KiB each.
    verify           text        CHECK (verify IS NULL OR octet_length(verify) <= 8192),
    context          text        CHECK (context IS NULL OR octet_length(context) <= 8192),
    files            jsonb,
    blockers         jsonb,
    decisions        jsonb,
    round            integer     NOT NULL DEFAULT 1 CHECK (round >= 1),
    transition_seq   integer     NOT NULL DEFAULT 0 CHECK (transition_seq >= 0),   -- ADDED: last transition number, taken under this row's lock (S3 §4.9)
    created_at       timestamptz NOT NULL DEFAULT platform.now(),
    updated_at       timestamptz NOT NULL DEFAULT platform.now(),
    closed_at        timestamptz,
    close_reason     text        CHECK (close_reason IS NULL OR octet_length(close_reason) BETWEEN 1 AND 512),   -- required when cancelled (CHECK below)
    superseded_by    uuid,
    CHECK (octet_length(coalesce(files::text, '')) + octet_length(coalesce(blockers::text, '')) + octet_length(coalesce(decisions::text, '')) <= 65536),
    CHECK ((state IN ('accepted', 'cancelled')) = (closed_at IS NOT NULL)),
    CHECK (state <> 'cancelled' OR close_reason IS NOT NULL),
    CHECK (state <> 'pending' OR holder IS NULL),
    CHECK (state <> 'claimed' OR holder IS NOT NULL),
    CHECK (superseded_by IS NULL OR superseded_by <> id),
    CHECK (rework_holder IS NULL OR round > 1)
);
CREATE INDEX handoffs_by_addressee ON handoffs.handoffs (project_id, to_kind, to_ref, state);
CREATE INDEX handoffs_by_holder    ON handoffs.handoffs (project_id, holder) WHERE holder IS NOT NULL;
CREATE INDEX handoffs_by_creator   ON handoffs.handoffs (project_id, created_by, state);

-- The packet is frozen after creation (bob Q-2): what the creator wrote is returned exactly. These columns never change; everything
-- else (state, addressee on reassign, holder, lease, priority, due date, round, rework_holder, closing fields) does. The module role also
-- has no UPDATE grant on them (below); the trigger covers every other role.
CREATE FUNCTION handoffs.refuse_packet_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.id IS DISTINCT FROM OLD.id OR NEW.project_id IS DISTINCT FROM OLD.project_id OR NEW.kind IS DISTINCT FROM OLD.kind
       OR NEW.created_by IS DISTINCT FROM OLD.created_by OR NEW.task_id IS DISTINCT FROM OLD.task_id
       OR NEW.summary IS DISTINCT FROM OLD.summary OR NEW.next_action IS DISTINCT FROM OLD.next_action
       OR NEW.acceptance IS DISTINCT FROM OLD.acceptance OR NEW.verify IS DISTINCT FROM OLD.verify
       OR NEW.context IS DISTINCT FROM OLD.context OR NEW.files IS DISTINCT FROM OLD.files
       OR NEW.blockers IS DISTINCT FROM OLD.blockers OR NEW.decisions IS DISTINCT FROM OLD.decisions
       OR NEW.created_at IS DISTINCT FROM OLD.created_at THEN
        RAISE EXCEPTION 'the handoff packet is frozen after creation' USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER handoffs_packet_frozen BEFORE UPDATE ON handoffs.handoffs
    FOR EACH ROW EXECUTE FUNCTION handoffs.refuse_packet_change();

-- The 200-open-handoff quota (S3 section 4.1, Q12) is CUT from 0.2.001 (cox ruling, via 3a6f3b98): no creator_open table. It returns,
-- with its release, as an additive table.

-- A return per round: UNIQUE (handoff_id, round) is what gives two racing returns exactly one winner (vera promise 2).
CREATE TABLE handoffs.returns (
    id          uuid        PRIMARY KEY,
    handoff_id  uuid        NOT NULL REFERENCES handoffs.handoffs (id) ON DELETE RESTRICT,
    project_id  uuid        NOT NULL,
    round       integer     NOT NULL CHECK (round >= 1),
    author      uuid        NOT NULL,
    incarnation bigint      NOT NULL,
    summary     text        NOT NULL CHECK (octet_length(summary) BETWEEN 1 AND 32768),   -- the cap is the pack's choice (S3 §4.4 sets none)
    risks       text        NOT NULL CHECK (risks <> ''),                                  -- the literal 'none' is allowed, an empty string is not
    tests       jsonb,
    created_at  timestamptz NOT NULL DEFAULT platform.now(),
    UNIQUE (handoff_id, round)
);

CREATE TABLE handoffs.evidence (
    id         uuid    PRIMARY KEY,
    return_id  uuid    NOT NULL REFERENCES handoffs.returns (id) ON DELETE RESTRICT,
    project_id uuid    NOT NULL,
    kind       text    NOT NULL CHECK (kind IN ('test', 'output', 'review', 'artifact', 'link')),
    ref        text,
    inline     bytea   CHECK (inline IS NULL OR octet_length(inline) <= 1048576),      -- at most 1 MiB per entry here; the 1 MiB total per return is the route's check
    sha256     bytea   NOT NULL CHECK (length(sha256) = 32),                           -- computed by the server for inline evidence
    size       integer NOT NULL CHECK (size >= 0),
    CHECK (ref IS NOT NULL OR inline IS NOT NULL)
);
CREATE INDEX evidence_by_return ON handoffs.evidence (return_id);

-- One verdict per round. A rework verdict needs notes (S3 §4.3).
-- self_review (kai ruling 7e45d549, 5 Nov 20:37Z, relayed by cox 140b8030): true only when the return's author is ALSO the handoff's
-- creator and the project lead, so no independent reviewer exists. bob's code enforces that rule and sets the flag; the column only
-- records it (the author, creator and lead live in other tables, so no CHECK can). `show` prints "self-reviewed (no independent reviewer)".
CREATE TABLE handoffs.reviews (
    id          uuid        PRIMARY KEY,
    handoff_id  uuid        NOT NULL REFERENCES handoffs.handoffs (id) ON DELETE RESTRICT,
    project_id  uuid        NOT NULL,
    round       integer     NOT NULL CHECK (round >= 1),
    reviewer    uuid        NOT NULL,
    verdict     text        NOT NULL CHECK (verdict IN ('accept', 'rework')),
    notes       text,
    self_review boolean     NOT NULL DEFAULT false,
    at          timestamptz NOT NULL DEFAULT platform.now(),
    UNIQUE (handoff_id, round),
    CHECK (verdict = 'accept' OR (notes IS NOT NULL AND notes <> ''))
);

-- The transition log, the ordinary audit row for every transition (S3 §4.9): append-only, deterministic order by
-- (handoff_id, seq), seq taken from handoffs.handoffs.transition_seq under the row lock. from_state is NULL for creation.
CREATE TABLE handoffs.transitions (
    id              uuid        PRIMARY KEY,
    handoff_id      uuid        NOT NULL REFERENCES handoffs.handoffs (id) ON DELETE RESTRICT,
    project_id      uuid        NOT NULL,
    seq             integer     NOT NULL CHECK (seq >= 1),
    from_state      text        CHECK (from_state IN ('pending', 'claimed', 'suspended', 'returned', 'accepted', 'cancelled')),
    to_state        text        NOT NULL CHECK (to_state IN ('pending', 'claimed', 'suspended', 'returned', 'accepted', 'cancelled')),
    actor_kind      text        NOT NULL CHECK (actor_kind IN ('worker', 'service', 'system', 'host')),
    actor_id        text        NOT NULL,
    incarnation     bigint      NOT NULL,
    reason          text        CHECK (reason IS NULL OR octet_length(reason) BETWEEN 1 AND 512),   -- system reasons are codes (lease_expired); a lead's or creator's reason is text (S3 §4.3)
    idempotency_key text,
    at              timestamptz NOT NULL DEFAULT platform.now(),
    UNIQUE (handoff_id, seq)
);

CREATE FUNCTION handoffs.refuse_change() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION '% is append-only', TG_TABLE_NAME USING ERRCODE = '42501';
END $$;
CREATE TRIGGER returns_append_only     BEFORE UPDATE OR DELETE ON handoffs.returns     FOR EACH ROW EXECUTE FUNCTION handoffs.refuse_change();
CREATE TRIGGER evidence_append_only    BEFORE UPDATE OR DELETE ON handoffs.evidence    FOR EACH ROW EXECUTE FUNCTION handoffs.refuse_change();
CREATE TRIGGER reviews_append_only     BEFORE UPDATE OR DELETE ON handoffs.reviews     FOR EACH ROW EXECUTE FUNCTION handoffs.refuse_change();
CREATE TRIGGER transitions_append_only BEFORE UPDATE OR DELETE ON handoffs.transitions FOR EACH ROW EXECUTE FUNCTION handoffs.refuse_change();

ALTER TABLE handoffs.handoffs     ENABLE ROW LEVEL SECURITY;  ALTER TABLE handoffs.handoffs     FORCE ROW LEVEL SECURITY;
ALTER TABLE handoffs.returns      ENABLE ROW LEVEL SECURITY;  ALTER TABLE handoffs.returns      FORCE ROW LEVEL SECURITY;
ALTER TABLE handoffs.evidence     ENABLE ROW LEVEL SECURITY;  ALTER TABLE handoffs.evidence     FORCE ROW LEVEL SECURITY;
ALTER TABLE handoffs.reviews      ENABLE ROW LEVEL SECURITY;  ALTER TABLE handoffs.reviews      FORCE ROW LEVEL SECURITY;
ALTER TABLE handoffs.transitions  ENABLE ROW LEVEL SECURITY;  ALTER TABLE handoffs.transitions  FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON handoffs.handoffs
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON handoffs.returns
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON handoffs.evidence
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON handoffs.reviews
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON handoffs.transitions
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);

-- Grants: a handoff changes only in its working columns (UPDATE is per column; the frozen packet columns are not in the list);
-- the rest is insert-only history. Nothing is deleted.
GRANT USAGE ON SCHEMA handoffs TO cortex_m_handoffs;
GRANT SELECT, INSERT ON handoffs.handoffs TO cortex_m_handoffs;
GRANT UPDATE (state, to_kind, to_ref, holder, rework_holder, incarnation, lease_ttl_s, lease_renewed_at, priority, due_at, round,
              transition_seq, updated_at, closed_at, close_reason, superseded_by) ON handoffs.handoffs TO cortex_m_handoffs;
GRANT SELECT, INSERT ON handoffs.returns, handoffs.evidence, handoffs.reviews, handoffs.transitions TO cortex_m_handoffs;
