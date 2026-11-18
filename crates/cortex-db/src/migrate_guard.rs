//! The migration guard (F1-13; rework 2, vera's F1B-RW1-MIGRATION-CATALOG-COVERAGE). A step runs
//! under it, in its own transaction as `cortex_owner`. The step is refused with `OutsideSchema`, and
//! rolled back, if it created, changed or removed anything outside its module's schema.
//!
//! **What it is.** A check on a trusted release migration's ordinary mistakes. It is not
//! confinement: SQL meant to evade it, and hostile SQL under any role, are outside what 0.2.001
//! contains (C-32).
//!
//! **How coverage is known to be complete.**
//! 1. **Which catalogues changed comes from the server.** PostgreSQL counts every heap insert,
//!    update and delete per relation for the current transaction, child transactions included
//!    (`pg_stat_get_xact_tuples_*`). The guard reads those counts, before and after the step, for
//!    every table in `pg_catalog`, also taken from the server (`relkind = 'r'`), not from a list.
//!    `track_counts` off would hide them, so the guard then refuses to run.
//! 2. **Every catalogue has exactly one rule** (`guard_catalogues`):
//!    - `Rows`: its rows belong to schema objects, so each row is attributed to a schema;
//!    - `Statistics`: planner statistics, not object definitions;
//!    - `Never`: no release migration may change it.
//!
//!    A changed catalogue whose rule is `Never`, or that has no rule at all (a catalogue a newer
//!    server adds), refuses the step. So a catalogue missing from the table fails closed; it never
//!    passes silently.
//! 3. **A test pins the table to the server** (`f1b_rw2_catalogue_rules`). It fails when the
//!    server's catalogue list and the table differ in either direction, and when a `Rows`
//!    catalogue is not readable by the owner.
//!
//! 4. **Relationship rows are checked at every end they change** (rework 3, vera's
//!    F1B-RW2-PARENT-ATTRIBUTION). Each reference column PostgreSQL declares for a `Rows` catalogue
//!    (`pg_get_catalog_foreign_keys()`) has one rule (`guard_references`):
//!    - `Owner`: the row is attributed to that end;
//!    - `Changed`: writing or removing the row changes that end's definition, so that end must be
//!      inside too;
//!    - `Reference`: that end is only referred to.
//!
//!    The changed ends are: a partition's or child's parent (`pg_inherits.inhparent`); the
//!    extension an object joins as a member, or the parent index a partition index joins
//!    (`pg_depend` with `deptype` e or P); an extended-statistics object's table; an operator
//!    class's family. `deptype` x (AUTO_EXTENSION, `ALTER … DEPENDS ON EXTENSION`) is a reference
//!    (rework 4, vera's F1B-RW3-AUTO-EXTENSION-REFERENCE): it changes only the dependent object. The test
//!    `f1b_rw3_reference_rules` pins the table to the server, as for catalogues: a reference the
//!    server adds fails it.
//! 5. **Removal is checked for every object the step's role could remove**, and only the owner
//!    removes an object. That means objects made after `initdb` (oid 16384 or more) and the `initdb`
//!    objects the role owns through membership (`owned_initdb_sql`, computed by the server; with
//!    the fixture's topology, the `public` schema).
//! 6. **The topology is schema-closed at every step boundary** (kai's ruling T-1). A link can
//!    change an object without writing any row of it: attaching a partition changes the DEFAULT
//!    partition's implicit constraint. The rows cannot show that when the DEFAULT is in another
//!    schema, so the guard is defined for schema-closed trees only: no `pg_inherits` link (a
//!    partition, a DEFAULT partition, a partition index, an inheritance child) may join two
//!    schemas. One query checks it before a step runs (rework 5, after vera's
//!    F1B-RW4-TRANSITIVE-DEFAULT-CONSTRAINT: `StartTopologyNotSchemaClosed`) and again after its
//!    statements, before anything is recorded (rework 6, after vera's
//!    F1B-RW5-TEMP-INHERITANCE-TOPOLOGY: `StepLeftTopologyNotSchemaClosed`, rolled back). The
//!    second holds whatever statement made the link: a temporary table inheriting an own table,
//!    for one, counts as inside to items 2 to 5. Those items still refuse most such steps first,
//!    naming the object. A link made outside the guard, by arbitrary SQL as the owner, is the C-32
//!    limit: the next step sees it.
//! 7. **Nothing the step left runs at COMMIT unobserved** (rework 8, after vera's and sam's
//!    F1B-RW6-DEFERRED-FINALIZATION). PostgreSQL 18.4's `CommitTransaction` runs user code only in
//!    its first loop: the pending deferred triggers; the freezing of holdable cursors, which runs
//!    the rest of their query; and the dropping of other open cursors, which may shut down a
//!    started executor (its next comment: "The remaining actions cannot call any user-defined
//!    code"). So right after the step's statements, before the observations that follow them,
//!    the guard runs `SET CONSTRAINTS ALL IMMEDIATE`, which fires the pending deferred triggers
//!    now and makes any queued later immediate; and it refuses an open holdable cursor
//!    (`StepLeftHoldableCursor`) and any other cursor the step left open (`StepLeftOpenCursor`).
//!    The rest of the commit sequence is enumerated in rework 8's packet. A trigger that defers
//!    itself again, to run after the guard, is SQL written to evade it: the C-32 limit.
//! 8. **The guard reads the catalogue through pg_catalog only** (rework 9; kai's T-2 Addendum 3).
//!    Its observation SQL uses unqualified operators and cast types, which resolve through the
//!    session's `search_path`; a step could put its own schema first and shadow one (an
//!    always-false `<>` on oid hides a link). So before its first observation, and again after
//!    the step and the finalisation, the guard resets the session's settings and role and pins
//!    `search_path` to `pg_catalog, pg_temp` for the rest of the transaction (`OBSERVE`). The
//!    step itself runs with the session's default path, and its own settings do not outlive it.
//!
//! **What it does not see.** Non-transactional in-place catalogue updates (VACUUM's and ANALYZE's
//! page and tuple counts, frozen-xid horizons) are not counted. They are statistics and
//! horizons, not definitions. The `Statistics` catalogues (`pg_statistic`, `pg_statistic_ext_data`)
//! are not checked, and the owner cannot read them.
use std::collections::{BTreeMap, BTreeSet};

use sqlx::PgConnection;

use crate::error::DbError;
use crate::migrate::MigrateError;

/// What the guard does with one catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueRule {
    /// Rows of schema objects: each row is attributed to a schema, and a row the step added or
    /// changed, or removed, outside the step's schema refuses it.
    Rows,
    /// Planner statistics: not object definitions, and not readable by the owner. Not checked.
    Statistics,
    /// No release migration may change it: any change refuses the step, whatever the schema.
    Never,
}

/// One `Rows` catalogue. SQL expressions over the row alias `x`:
/// - `identity`: the row's complete identity, as text;
/// - `schema`: the oid of the schema the row belongs to, or NULL if none;
/// - `display`: the name an `OutsideSchema` gives;
/// - `scope`: a filter on the rows considered (pg_shdepend: this database's rows only).
struct RowsCatalogue {
    name: &'static str,
    identity: String,
    schema: String,
    display: String,
    scope: Option<String>,
    /// The identity is the row's oid, so `obj()` can resolve a reference to it.
    by_oid: bool,
    /// The oid of the object the row belongs to: the removal check's boundary (`removable_sql`).
    owner: String,
    /// The column naming the row's owning role, for the catalogues whose rows are owned objects.
    owner_role: Option<&'static str>,
    /// The other ends of a relationship row whose definition the row changes (rework 3, vera's
    /// F1B-RW2-PARENT-ATTRIBUTION). A row is outside if its own schema OR any changed end's is.
    changed: Vec<Endpoint>,
}

/// A changed end of a relationship row: the reference columns it covers (as the server names them
/// in `pg_get_catalog_foreign_keys()`), when it applies, its schema and its name.
struct Endpoint {
    covers: &'static [&'static str],
    cond: String,
    schema: String,
    display: String,
}

/// Every catalogue of PostgreSQL 18 and its rule. `f1b_rw2_catalogue_rules` checks this against
/// the server.
pub fn guard_catalogues() -> Vec<(&'static str, CatalogueRule)> {
    let mut all: Vec<(&'static str, CatalogueRule)> = rows_catalogues()
        .iter()
        .map(|c| (c.name, CatalogueRule::Rows))
        .collect();
    all.extend(STATISTICS.iter().map(|n| (*n, CatalogueRule::Statistics)));
    all.extend(NEVER.iter().map(|n| (*n, CatalogueRule::Never)));
    all
}

const STATISTICS: &[&str] = &["pg_statistic", "pg_statistic_ext_data"];

/// Global objects, roles, databases, replication, publications and subscriptions, foreign-data
/// wrappers and servers, casts, transforms, languages, access methods, extensions, event triggers
/// and large objects: a module's release migration changes none of them.
const NEVER: &[&str] = &[
    "pg_am",
    "pg_auth_members",
    "pg_authid",
    "pg_cast",
    "pg_database",
    "pg_db_role_setting",
    "pg_event_trigger",
    "pg_extension",
    "pg_foreign_data_wrapper",
    "pg_foreign_server",
    "pg_language",
    "pg_largeobject",
    "pg_largeobject_metadata",
    "pg_parameter_acl",
    "pg_publication",
    "pg_publication_namespace",
    "pg_publication_rel",
    "pg_replication_origin",
    "pg_shdescription",
    "pg_shseclabel",
    "pg_subscription",
    "pg_subscription_rel",
    "pg_tablespace",
    "pg_transform",
    "pg_user_mapping",
];

fn rel_ns(o: &str) -> String {
    format!("(SELECT c.relnamespace FROM pg_catalog.pg_class c WHERE c.oid = {o})")
}
fn rel_name(o: &str) -> String {
    format!(
        "(SELECT n.nspname || '.' || c.relname FROM pg_catalog.pg_class c \
         JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace WHERE c.oid = {o})"
    )
}
fn typ_ns(o: &str) -> String {
    format!("(SELECT t.typnamespace FROM pg_catalog.pg_type t WHERE t.oid = {o})")
}
fn typ_name(o: &str) -> String {
    format!(
        "(SELECT n.nspname || '.' || t.typname FROM pg_catalog.pg_type t \
         JOIN pg_catalog.pg_namespace n ON n.oid = t.typnamespace WHERE t.oid = {o})"
    )
}
fn proc_ns(o: &str) -> String {
    format!("(SELECT p.pronamespace FROM pg_catalog.pg_proc p WHERE p.oid = {o})")
}
fn proc_name(o: &str) -> String {
    format!(
        "(SELECT n.nspname || '.' || p.proname || '(' || pg_catalog.pg_get_function_identity_arguments(p.oid) || ')' \
         FROM pg_catalog.pg_proc p JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace WHERE p.oid = {o})"
    )
}
fn fam_ns(o: &str) -> String {
    format!("(SELECT f.opfnamespace FROM pg_catalog.pg_opfamily f WHERE f.oid = {o})")
}
fn fam_name(o: &str) -> String {
    format!(
        "(SELECT n.nspname || '.' || f.opfname FROM pg_catalog.pg_opfamily f \
         JOIN pg_catalog.pg_namespace n ON n.oid = f.opfnamespace WHERE f.oid = {o})"
    )
}
fn cfg_ns(o: &str) -> String {
    format!("(SELECT g.cfgnamespace FROM pg_catalog.pg_ts_config g WHERE g.oid = {o})")
}
fn cfg_name(o: &str) -> String {
    format!(
        "(SELECT n.nspname || '.' || g.cfgname FROM pg_catalog.pg_ts_config g \
         JOIN pg_catalog.pg_namespace n ON n.oid = g.cfgnamespace WHERE g.oid = {o})"
    )
}
fn ns_name(ns: &str) -> String {
    format!("(SELECT n.nspname FROM pg_catalog.pg_namespace n WHERE n.oid = {ns})")
}
fn qualified(ns: &str, name: &str) -> String {
    format!("{} || '.' || {name}", ns_name(ns))
}

/// A catalogue whose rows are schema objects with their own oid.
fn by_oid(name: &'static str, schema: String, display: String) -> RowsCatalogue {
    RowsCatalogue {
        name,
        identity: "x.oid::text".into(),
        schema,
        display,
        scope: None,
        by_oid: true,
        owner: "x.oid".into(),
        owner_role: None,
        changed: Vec::new(),
    }
}

/// A catalogue keyed by something else.
fn keyed(name: &'static str, identity: &str, schema: String, display: String) -> RowsCatalogue {
    RowsCatalogue {
        name,
        identity: identity.into(),
        schema,
        display,
        scope: None,
        by_oid: false,
        owner: identity
            .split("::")
            .next()
            .and_then(|f| f.split(" ||").next())
            .unwrap_or(identity)
            .into(),
        owner_role: None,
        changed: Vec::new(),
    }
}

/// The `Rows` catalogues, in reporting order: the first eight as rework 1 had them.
fn rows_catalogues() -> Vec<RowsCatalogue> {
    let mut v = vec![
        by_oid(
            "pg_class",
            "x.relnamespace".into(),
            qualified("x.relnamespace", "x.relname"),
        ),
        keyed(
            "pg_attribute",
            "x.attrelid::text || ':' || x.attnum",
            rel_ns("x.attrelid"),
            format!("{} || '.' || x.attname", rel_name("x.attrelid")),
        ),
        by_oid("pg_proc", "x.pronamespace".into(), proc_name("x.oid")),
        by_oid(
            "pg_type",
            "x.typnamespace".into(),
            qualified("x.typnamespace", "x.typname"),
        ),
        by_oid(
            "pg_trigger",
            rel_ns("x.tgrelid"),
            format!("{} || '.' || x.tgname", rel_name("x.tgrelid")),
        ),
        by_oid(
            "pg_policy",
            rel_ns("x.polrelid"),
            format!("{} || '.' || x.polname", rel_name("x.polrelid")),
        ),
        by_oid(
            "pg_constraint",
            "x.connamespace".into(),
            qualified("x.connamespace", "x.conname"),
        ),
        by_oid("pg_namespace", "x.oid".into(), "x.nspname::text".into()),
        by_oid("pg_enum", typ_ns("x.enumtypid"), typ_name("x.enumtypid")),
        by_oid(
            "pg_rewrite",
            rel_ns("x.ev_class"),
            format!("{} || '.' || x.rulename", rel_name("x.ev_class")),
        ),
        by_oid(
            "pg_attrdef",
            rel_ns("x.adrelid"),
            format!(
                "{} || '.' || (SELECT a.attname FROM pg_catalog.pg_attribute a WHERE a.attrelid = x.adrelid AND a.attnum = x.adnum)",
                rel_name("x.adrelid")
            ),
        ),
        keyed(
            "pg_index",
            "x.indexrelid::text",
            rel_ns("x.indexrelid"),
            rel_name("x.indexrelid"),
        ),
        keyed(
            "pg_inherits",
            "x.inhrelid::text || ':' || x.inhparent",
            rel_ns("x.inhrelid"),
            rel_name("x.inhrelid"),
        ),
        keyed(
            "pg_sequence",
            "x.seqrelid::text",
            rel_ns("x.seqrelid"),
            rel_name("x.seqrelid"),
        ),
        keyed(
            "pg_partitioned_table",
            "x.partrelid::text",
            rel_ns("x.partrelid"),
            rel_name("x.partrelid"),
        ),
        keyed(
            "pg_foreign_table",
            "x.ftrelid::text",
            rel_ns("x.ftrelid"),
            rel_name("x.ftrelid"),
        ),
        keyed(
            "pg_range",
            "x.rngtypid::text",
            typ_ns("x.rngtypid"),
            typ_name("x.rngtypid"),
        ),
        keyed(
            "pg_aggregate",
            "x.aggfnoid::oid::text",
            proc_ns("x.aggfnoid::oid"),
            proc_name("x.aggfnoid::oid"),
        ),
        by_oid(
            "pg_operator",
            "x.oprnamespace".into(),
            qualified("x.oprnamespace", "x.oprname"),
        ),
        by_oid(
            "pg_opclass",
            "x.opcnamespace".into(),
            qualified("x.opcnamespace", "x.opcname"),
        ),
        by_oid(
            "pg_opfamily",
            "x.opfnamespace".into(),
            qualified("x.opfnamespace", "x.opfname"),
        ),
        by_oid("pg_amop", fam_ns("x.amopfamily"), fam_name("x.amopfamily")),
        by_oid(
            "pg_amproc",
            fam_ns("x.amprocfamily"),
            fam_name("x.amprocfamily"),
        ),
        by_oid(
            "pg_collation",
            "x.collnamespace".into(),
            qualified("x.collnamespace", "x.collname"),
        ),
        by_oid(
            "pg_conversion",
            "x.connamespace".into(),
            qualified("x.connamespace", "x.conname"),
        ),
        by_oid(
            "pg_statistic_ext",
            "x.stxnamespace".into(),
            qualified("x.stxnamespace", "x.stxname"),
        ),
        by_oid(
            "pg_ts_config",
            "x.cfgnamespace".into(),
            qualified("x.cfgnamespace", "x.cfgname"),
        ),
        keyed(
            "pg_ts_config_map",
            "x.mapcfg::text || ':' || x.maptokentype || ':' || x.mapseqno",
            cfg_ns("x.mapcfg"),
            cfg_name("x.mapcfg"),
        ),
        by_oid(
            "pg_ts_dict",
            "x.dictnamespace".into(),
            qualified("x.dictnamespace", "x.dictname"),
        ),
        by_oid(
            "pg_ts_parser",
            "x.prsnamespace".into(),
            qualified("x.prsnamespace", "x.prsname"),
        ),
        by_oid(
            "pg_ts_template",
            "x.tmplnamespace".into(),
            qualified("x.tmplnamespace", "x.tmplname"),
        ),
        by_oid(
            "pg_default_acl",
            "NULLIF(x.defaclnamespace, 0)".into(),
            format!(
                "'default privileges in ' || coalesce({}, 'every schema')",
                ns_name("x.defaclnamespace")
            ),
        ),
    ];
    // The object-reference catalogues: a row belongs to the object it is about, resolved through
    // the oid catalogues above (`obj`); a reference to anything else has no schema.
    let classes: Vec<(&'static str, String)> = v
        .iter()
        .filter(|c| c.by_oid)
        .map(|c| (c.name, c.schema.replace("x.", "y.")))
        .collect();
    let obj = |class: &str, oid: &str| -> String {
        let mut s = format!("(CASE {class}");
        for (name, schema) in &classes {
            s.push_str(&format!(
                " WHEN 'pg_catalog.{name}'::pg_catalog.regclass THEN (SELECT {schema} FROM pg_catalog.{name} y WHERE y.oid = {oid})"
            ));
        }
        s.push_str(" END)");
        s
    };
    let known = classes
        .iter()
        .map(|(n, _)| format!("'pg_catalog.{n}'::pg_catalog.regclass"))
        .collect::<Vec<_>>()
        .join(", ");
    let identify = |class: &str, oid: &str, sub: &str| -> String {
        format!(
            "(CASE WHEN {class} IN ({known}) \
             THEN (pg_catalog.pg_identify_object({class}, {oid}, {sub})).identity \
             ELSE {class}::pg_catalog.regclass::text || ' ' || {oid} END)"
        )
    };
    let identify_typed = |class: &str, oid: &str, sub: &str| -> String {
        format!(
            "(CASE WHEN {class} IN ({known}, 'pg_catalog.pg_extension'::pg_catalog.regclass) \
             THEN (pg_catalog.pg_identify_object({class}, {oid}, {sub})).type || ' ' || \
                  (pg_catalog.pg_identify_object({class}, {oid}, {sub})).identity \
             ELSE {class}::pg_catalog.regclass::text || ' ' || {oid} END)"
        )
    };
    let mut depend = keyed(
        "pg_depend",
        "x.classid::text || ':' || x.objid || ':' || x.objsubid || ':' || x.refclassid || ':' || x.refobjid || ':' || x.refobjsubid || ':' || x.deptype::text",
        obj("x.classid", "x.objid"),
        identify("x.classid", "x.objid", "x.objsubid"),
    );
    depend.owner = "x.objid".into();
    // Extension membership (e) and partition attachment (P) change the referenced object: the
    // extension's members, the parent index's partitions. Any other dependency is a reference,
    // AUTO_EXTENSION (x) included: `ALTER … DEPENDS ON EXTENSION` needs only the dependent object's
    // ownership, writes only this row, and leaves the extension and its members as they are.
    depend.changed.push(Endpoint {
        covers: &["refclassid"],
        cond: "x.deptype IN ('e', 'P')".into(),
        schema: obj("x.refclassid", "x.refobjid"),
        display: identify_typed("x.refclassid", "x.refobjid", "x.refobjsubid"),
    });
    v.push(depend);
    let mut shdepend = keyed(
        "pg_shdepend",
        "x.dbid::text || ':' || x.classid || ':' || x.objid || ':' || x.objsubid || ':' || x.refclassid || ':' || x.refobjid || ':' || x.deptype::text",
        obj("x.classid", "x.objid"),
        identify("x.classid", "x.objid", "x.objsubid"),
    );
    shdepend.owner = "x.objid".into();
    shdepend.scope = Some(
        "x.dbid = (SELECT d.oid FROM pg_catalog.pg_database d WHERE d.datname = pg_catalog.current_database())"
            .into(),
    );
    v.push(shdepend);
    v.push(keyed(
        "pg_description",
        "x.objoid::text || ':' || x.classoid || ':' || x.objsubid",
        obj("x.classoid", "x.objoid"),
        identify("x.classoid", "x.objoid", "x.objsubid"),
    ));
    v.push(keyed(
        "pg_seclabel",
        "x.objoid::text || ':' || x.classoid || ':' || x.objsubid || ':' || x.provider",
        obj("x.classoid", "x.objoid"),
        identify("x.classoid", "x.objoid", "x.objsubid"),
    ));
    v.push(keyed(
        "pg_init_privs",
        "x.objoid::text || ':' || x.classoid || ':' || x.objsubid",
        obj("x.classoid", "x.objoid"),
        identify("x.classoid", "x.objoid", "x.objsubid"),
    ));
    for c in &mut v {
        match c.name {
            "pg_aggregate" => c.owner = "x.aggfnoid::oid".into(),
            // An inheritance or partition row changes its parent's children.
            "pg_inherits" => c.changed.push(Endpoint {
                covers: &["inhparent"],
                cond: "true".into(),
                schema: rel_ns("x.inhparent"),
                display: rel_name("x.inhparent"),
            }),
            // An extended-statistics object changes its table's statistics.
            "pg_statistic_ext" => c.changed.push(Endpoint {
                covers: &["stxrelid", "stxrelid,stxkeys"],
                cond: "true".into(),
                schema: rel_ns("x.stxrelid"),
                display: rel_name("x.stxrelid"),
            }),
            // An operator class joins its family (superuser-only DDL; listed for completeness).
            "pg_opclass" => c.changed.push(Endpoint {
                covers: &["opcfamily"],
                cond: "true".into(),
                schema: fam_ns("x.opcfamily"),
                display: fam_name("x.opcfamily"),
            }),
            _ => {}
        }
        c.owner_role = match c.name {
            "pg_class" => Some("relowner"),
            "pg_type" => Some("typowner"),
            "pg_proc" => Some("proowner"),
            "pg_namespace" => Some("nspowner"),
            "pg_operator" => Some("oprowner"),
            "pg_opclass" => Some("opcowner"),
            "pg_opfamily" => Some("opfowner"),
            "pg_collation" => Some("collowner"),
            "pg_conversion" => Some("conowner"),
            "pg_statistic_ext" => Some("stxowner"),
            "pg_ts_config" => Some("cfgowner"),
            "pg_ts_dict" => Some("dictowner"),
            "pg_default_acl" => Some("defaclrole"),
            _ => None,
        };
    }
    v
}

/// How a catalogue reference is treated (rework 3; `guard_references`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceRule {
    /// The row belongs to the referenced object: the guard attributes the row to it.
    Owner,
    /// Writing or removing the row changes the referenced object's definition: the guard checks
    /// that end too.
    Changed,
    /// The row only refers to the referenced object, which it does not change.
    Reference,
}

/// Every reference column of the `Rows` catalogues that PostgreSQL 18.4 declares
/// (`pg_get_catalog_foreign_keys()`), with its rule. `f1b_rw3_reference_rules` checks this against
/// the server, and checks that every `Changed` entry has an endpoint the guard checks.
pub fn guard_references() -> &'static [(&'static str, &'static str, ReferenceRule)] {
    REFERENCES
}

/// The `Changed` references the guard implements, as `(catalogue, columns)`.
pub fn guard_changed_endpoints() -> Vec<(&'static str, &'static str)> {
    rows_catalogues()
        .iter()
        .flat_map(|c| {
            c.changed
                .iter()
                .flat_map(move |e| e.covers.iter().map(move |cols| (c.name, *cols)))
        })
        .collect()
}

/// The objects made by `initdb` that the current role can remove: those owned by a role it is a
/// member of (rework 3, vera's F1B-RW2-INITDB-REMOVAL). Every other `initdb` object is owned by the
/// bootstrap superuser, and only its owner can drop, alter or comment on it. Computed by the server
/// from every owned `Rows` catalogue, not assumed: with the fixture's topology it is the `public`
/// schema (oid 2200, owned by `pg_database_owner`, of which the database owner is a member).
/// `initdb` oids are unique across catalogues, so the oid alone names the object.
pub fn owned_initdb_sql() -> String {
    rows_catalogues()
        .iter()
        .filter_map(|c| {
            c.owner_role.map(|role| {
                format!(
                    "SELECT x.oid AS o FROM pg_catalog.{name} x \
                     WHERE x.oid < 16384 AND pg_catalog.pg_has_role(x.{role}, 'MEMBER')",
                    name = c.name
                )
            })
        })
        .collect::<Vec<_>>()
        .join("\nUNION ALL\n")
}

const REFERENCES: &[(&str, &str, ReferenceRule)] = &[
    ("pg_aggregate", "aggcombinefn", ReferenceRule::Reference), // -> pg_proc
    ("pg_aggregate", "aggdeserialfn", ReferenceRule::Reference), // -> pg_proc
    ("pg_aggregate", "aggfinalfn", ReferenceRule::Reference),   // -> pg_proc
    ("pg_aggregate", "aggfnoid", ReferenceRule::Owner),         // -> pg_proc
    ("pg_aggregate", "aggmfinalfn", ReferenceRule::Reference),  // -> pg_proc
    ("pg_aggregate", "aggminvtransfn", ReferenceRule::Reference), // -> pg_proc
    ("pg_aggregate", "aggmtransfn", ReferenceRule::Reference),  // -> pg_proc
    ("pg_aggregate", "aggmtranstype", ReferenceRule::Reference), // -> pg_type
    ("pg_aggregate", "aggserialfn", ReferenceRule::Reference),  // -> pg_proc
    ("pg_aggregate", "aggsortop", ReferenceRule::Reference),    // -> pg_operator
    ("pg_aggregate", "aggtransfn", ReferenceRule::Reference),   // -> pg_proc
    ("pg_aggregate", "aggtranstype", ReferenceRule::Reference), // -> pg_type
    ("pg_amop", "amopfamily", ReferenceRule::Owner),            // -> pg_opfamily
    ("pg_amop", "amoplefttype", ReferenceRule::Reference),      // -> pg_type
    ("pg_amop", "amopmethod", ReferenceRule::Reference),        // -> pg_am
    ("pg_amop", "amopopr", ReferenceRule::Reference),           // -> pg_operator
    ("pg_amop", "amoprighttype", ReferenceRule::Reference),     // -> pg_type
    ("pg_amop", "amopsortfamily", ReferenceRule::Reference),    // -> pg_opfamily
    ("pg_amproc", "amproc", ReferenceRule::Reference),          // -> pg_proc
    ("pg_amproc", "amprocfamily", ReferenceRule::Owner),        // -> pg_opfamily
    ("pg_amproc", "amproclefttype", ReferenceRule::Reference),  // -> pg_type
    ("pg_amproc", "amprocrighttype", ReferenceRule::Reference), // -> pg_type
    ("pg_attrdef", "adrelid", ReferenceRule::Owner),            // -> pg_class
    ("pg_attrdef", "adrelid,adnum", ReferenceRule::Owner),      // -> pg_attribute
    ("pg_attribute", "attcollation", ReferenceRule::Reference), // -> pg_collation
    ("pg_attribute", "attrelid", ReferenceRule::Owner),         // -> pg_class
    ("pg_attribute", "atttypid", ReferenceRule::Reference),     // -> pg_type
    ("pg_class", "relam", ReferenceRule::Reference),            // -> pg_am
    ("pg_class", "relnamespace", ReferenceRule::Owner),         // -> pg_namespace
    ("pg_class", "reloftype", ReferenceRule::Reference),        // -> pg_type
    ("pg_class", "relowner", ReferenceRule::Reference),         // -> pg_authid
    ("pg_class", "relrewrite", ReferenceRule::Reference),       // -> pg_class
    ("pg_class", "reltablespace", ReferenceRule::Reference),    // -> pg_tablespace
    ("pg_class", "reltoastrelid", ReferenceRule::Reference),    // -> pg_class
    ("pg_class", "reltype", ReferenceRule::Reference),          // -> pg_type
    ("pg_collation", "collnamespace", ReferenceRule::Owner),    // -> pg_namespace
    ("pg_collation", "collowner", ReferenceRule::Reference),    // -> pg_authid
    ("pg_constraint", "conexclop", ReferenceRule::Reference),   // -> pg_operator
    ("pg_constraint", "conffeqop", ReferenceRule::Reference),   // -> pg_operator
    ("pg_constraint", "confrelid", ReferenceRule::Reference),   // -> pg_class
    (
        "pg_constraint",
        "confrelid,confkey",
        ReferenceRule::Reference,
    ), // -> pg_attribute
    ("pg_constraint", "conindid", ReferenceRule::Reference),    // -> pg_class
    ("pg_constraint", "connamespace", ReferenceRule::Owner),    // -> pg_namespace
    ("pg_constraint", "conparentid", ReferenceRule::Reference), // -> pg_constraint
    ("pg_constraint", "conpfeqop", ReferenceRule::Reference),   // -> pg_operator
    ("pg_constraint", "conppeqop", ReferenceRule::Reference),   // -> pg_operator
    ("pg_constraint", "conrelid", ReferenceRule::Reference),    // -> pg_class
    ("pg_constraint", "conrelid,conkey", ReferenceRule::Reference), // -> pg_attribute
    ("pg_constraint", "contypid", ReferenceRule::Reference),    // -> pg_type
    ("pg_conversion", "connamespace", ReferenceRule::Owner),    // -> pg_namespace
    ("pg_conversion", "conowner", ReferenceRule::Reference),    // -> pg_authid
    ("pg_conversion", "conproc", ReferenceRule::Reference),     // -> pg_proc
    ("pg_default_acl", "defaclnamespace", ReferenceRule::Owner), // -> pg_namespace
    ("pg_default_acl", "defaclrole", ReferenceRule::Reference), // -> pg_authid
    ("pg_depend", "classid", ReferenceRule::Owner),             // -> pg_class
    ("pg_depend", "refclassid", ReferenceRule::Changed),        // -> pg_class
    ("pg_description", "classoid", ReferenceRule::Owner),       // -> pg_class
    ("pg_enum", "enumtypid", ReferenceRule::Owner),             // -> pg_type
    ("pg_foreign_table", "ftrelid", ReferenceRule::Owner),      // -> pg_class
    ("pg_foreign_table", "ftserver", ReferenceRule::Reference), // -> pg_foreign_server
    ("pg_index", "indclass", ReferenceRule::Reference),         // -> pg_opclass
    ("pg_index", "indcollation", ReferenceRule::Reference),     // -> pg_collation
    ("pg_index", "indexrelid", ReferenceRule::Owner),           // -> pg_class
    ("pg_index", "indrelid", ReferenceRule::Reference),         // -> pg_class
    ("pg_index", "indrelid,indkey", ReferenceRule::Reference),  // -> pg_attribute
    ("pg_inherits", "inhparent", ReferenceRule::Changed),       // -> pg_class
    ("pg_inherits", "inhrelid", ReferenceRule::Owner),          // -> pg_class
    ("pg_init_privs", "classoid", ReferenceRule::Owner),        // -> pg_class
    ("pg_namespace", "nspowner", ReferenceRule::Reference),     // -> pg_authid
    ("pg_opclass", "opcfamily", ReferenceRule::Changed),        // -> pg_opfamily
    ("pg_opclass", "opcintype", ReferenceRule::Reference),      // -> pg_type
    ("pg_opclass", "opckeytype", ReferenceRule::Reference),     // -> pg_type
    ("pg_opclass", "opcmethod", ReferenceRule::Reference),      // -> pg_am
    ("pg_opclass", "opcnamespace", ReferenceRule::Owner),       // -> pg_namespace
    ("pg_opclass", "opcowner", ReferenceRule::Reference),       // -> pg_authid
    ("pg_operator", "oprcode", ReferenceRule::Reference),       // -> pg_proc
    ("pg_operator", "oprcom", ReferenceRule::Reference),        // -> pg_operator
    ("pg_operator", "oprjoin", ReferenceRule::Reference),       // -> pg_proc
    ("pg_operator", "oprleft", ReferenceRule::Reference),       // -> pg_type
    ("pg_operator", "oprnamespace", ReferenceRule::Owner),      // -> pg_namespace
    ("pg_operator", "oprnegate", ReferenceRule::Reference),     // -> pg_operator
    ("pg_operator", "oprowner", ReferenceRule::Reference),      // -> pg_authid
    ("pg_operator", "oprrest", ReferenceRule::Reference),       // -> pg_proc
    ("pg_operator", "oprresult", ReferenceRule::Reference),     // -> pg_type
    ("pg_operator", "oprright", ReferenceRule::Reference),      // -> pg_type
    ("pg_opfamily", "opfmethod", ReferenceRule::Reference),     // -> pg_am
    ("pg_opfamily", "opfnamespace", ReferenceRule::Owner),      // -> pg_namespace
    ("pg_opfamily", "opfowner", ReferenceRule::Reference),      // -> pg_authid
    (
        "pg_partitioned_table",
        "partclass",
        ReferenceRule::Reference,
    ), // -> pg_opclass
    (
        "pg_partitioned_table",
        "partcollation",
        ReferenceRule::Reference,
    ), // -> pg_collation
    (
        "pg_partitioned_table",
        "partdefid",
        ReferenceRule::Reference,
    ), // -> pg_class
    ("pg_partitioned_table", "partrelid", ReferenceRule::Owner), // -> pg_class
    (
        "pg_partitioned_table",
        "partrelid,partattrs",
        ReferenceRule::Owner,
    ), // -> pg_attribute
    ("pg_policy", "polrelid", ReferenceRule::Owner),            // -> pg_class
    ("pg_policy", "polroles", ReferenceRule::Reference),        // -> pg_authid
    ("pg_proc", "proallargtypes", ReferenceRule::Reference),    // -> pg_type
    ("pg_proc", "proargtypes", ReferenceRule::Reference),       // -> pg_type
    ("pg_proc", "prolang", ReferenceRule::Reference),           // -> pg_language
    ("pg_proc", "pronamespace", ReferenceRule::Owner),          // -> pg_namespace
    ("pg_proc", "proowner", ReferenceRule::Reference),          // -> pg_authid
    ("pg_proc", "prorettype", ReferenceRule::Reference),        // -> pg_type
    ("pg_proc", "prosupport", ReferenceRule::Reference),        // -> pg_proc
    ("pg_proc", "protrftypes", ReferenceRule::Reference),       // -> pg_type
    ("pg_proc", "provariadic", ReferenceRule::Reference),       // -> pg_type
    ("pg_range", "rngcanonical", ReferenceRule::Reference),     // -> pg_proc
    ("pg_range", "rngcollation", ReferenceRule::Reference),     // -> pg_collation
    ("pg_range", "rngmultitypid", ReferenceRule::Reference),    // -> pg_type
    ("pg_range", "rngsubdiff", ReferenceRule::Reference),       // -> pg_proc
    ("pg_range", "rngsubopc", ReferenceRule::Reference),        // -> pg_opclass
    ("pg_range", "rngsubtype", ReferenceRule::Reference),       // -> pg_type
    ("pg_range", "rngtypid", ReferenceRule::Owner),             // -> pg_type
    ("pg_rewrite", "ev_class", ReferenceRule::Owner),           // -> pg_class
    ("pg_seclabel", "classoid", ReferenceRule::Owner),          // -> pg_class
    ("pg_sequence", "seqrelid", ReferenceRule::Owner),          // -> pg_class
    ("pg_sequence", "seqtypid", ReferenceRule::Reference),      // -> pg_type
    ("pg_shdepend", "classid", ReferenceRule::Owner),           // -> pg_class
    ("pg_shdepend", "dbid", ReferenceRule::Reference),          // -> pg_database
    ("pg_shdepend", "refclassid", ReferenceRule::Reference),    // -> pg_class
    ("pg_statistic_ext", "stxnamespace", ReferenceRule::Owner), // -> pg_namespace
    ("pg_statistic_ext", "stxowner", ReferenceRule::Reference), // -> pg_authid
    ("pg_statistic_ext", "stxrelid", ReferenceRule::Changed),   // -> pg_class
    (
        "pg_statistic_ext",
        "stxrelid,stxkeys",
        ReferenceRule::Changed,
    ), // -> pg_attribute
    ("pg_trigger", "tgconstraint", ReferenceRule::Reference),   // -> pg_constraint
    ("pg_trigger", "tgconstrindid", ReferenceRule::Reference),  // -> pg_class
    ("pg_trigger", "tgconstrrelid", ReferenceRule::Reference),  // -> pg_class
    ("pg_trigger", "tgfoid", ReferenceRule::Reference),         // -> pg_proc
    ("pg_trigger", "tgparentid", ReferenceRule::Reference),     // -> pg_trigger
    ("pg_trigger", "tgrelid", ReferenceRule::Owner),            // -> pg_class
    ("pg_trigger", "tgrelid,tgattr", ReferenceRule::Owner),     // -> pg_attribute
    ("pg_ts_config", "cfgnamespace", ReferenceRule::Owner),     // -> pg_namespace
    ("pg_ts_config", "cfgowner", ReferenceRule::Reference),     // -> pg_authid
    ("pg_ts_config", "cfgparser", ReferenceRule::Reference),    // -> pg_ts_parser
    ("pg_ts_config_map", "mapcfg", ReferenceRule::Owner),       // -> pg_ts_config
    ("pg_ts_config_map", "mapdict", ReferenceRule::Reference),  // -> pg_ts_dict
    ("pg_ts_dict", "dictnamespace", ReferenceRule::Owner),      // -> pg_namespace
    ("pg_ts_dict", "dictowner", ReferenceRule::Reference),      // -> pg_authid
    ("pg_ts_dict", "dicttemplate", ReferenceRule::Reference),   // -> pg_ts_template
    ("pg_ts_parser", "prsend", ReferenceRule::Reference),       // -> pg_proc
    ("pg_ts_parser", "prsheadline", ReferenceRule::Reference),  // -> pg_proc
    ("pg_ts_parser", "prslextype", ReferenceRule::Reference),   // -> pg_proc
    ("pg_ts_parser", "prsnamespace", ReferenceRule::Owner),     // -> pg_namespace
    ("pg_ts_parser", "prsstart", ReferenceRule::Reference),     // -> pg_proc
    ("pg_ts_parser", "prstoken", ReferenceRule::Reference),     // -> pg_proc
    ("pg_ts_template", "tmplinit", ReferenceRule::Reference),   // -> pg_proc
    ("pg_ts_template", "tmpllexize", ReferenceRule::Reference), // -> pg_proc
    ("pg_ts_template", "tmplnamespace", ReferenceRule::Owner),  // -> pg_namespace
    ("pg_type", "typanalyze", ReferenceRule::Reference),        // -> pg_proc
    ("pg_type", "typarray", ReferenceRule::Reference),          // -> pg_type
    ("pg_type", "typbasetype", ReferenceRule::Reference),       // -> pg_type
    ("pg_type", "typcollation", ReferenceRule::Reference),      // -> pg_collation
    ("pg_type", "typelem", ReferenceRule::Reference),           // -> pg_type
    ("pg_type", "typinput", ReferenceRule::Reference),          // -> pg_proc
    ("pg_type", "typmodin", ReferenceRule::Reference),          // -> pg_proc
    ("pg_type", "typmodout", ReferenceRule::Reference),         // -> pg_proc
    ("pg_type", "typnamespace", ReferenceRule::Owner),          // -> pg_namespace
    ("pg_type", "typoutput", ReferenceRule::Reference),         // -> pg_proc
    ("pg_type", "typowner", ReferenceRule::Reference),          // -> pg_authid
    ("pg_type", "typreceive", ReferenceRule::Reference),        // -> pg_proc
    ("pg_type", "typrelid", ReferenceRule::Reference),          // -> pg_class
    ("pg_type", "typsend", ReferenceRule::Reference),           // -> pg_proc
    ("pg_type", "typsubscript", ReferenceRule::Reference),      // -> pg_proc
];

/// Inside the step's schema (`$1`): the schema itself, toast, and temporary schemas.
const INSIDE: &str = "n.nspname = $1 OR n.nspname = 'pg_toast' OR n.nspname ~ '^pg_temp_' OR n.nspname ~ '^pg_toast_temp_'";

/// Outside, and removable by a migration: the built-in schemas cannot lose objects to `cortex_owner`.
fn removable_pred(schema: &str) -> String {
    format!(
        "NOT coalesce((SELECT {INSIDE} OR n.nspname IN ('pg_catalog', 'information_schema') \
         FROM pg_catalog.pg_namespace n WHERE n.oid = ({schema})), false)"
    )
}

/// Outside, for a row this step wrote (the built-in schemas included).
fn written_outside(schema: &str) -> String {
    format!(
        "NOT coalesce((SELECT {INSIDE} FROM pg_catalog.pg_namespace n WHERE n.oid = ({schema})), false)"
    )
}

fn display(c: &RowsCatalogue) -> String {
    format!(
        "coalesce(({})::text, '{}:' || ({}))",
        c.display, c.name, c.identity
    )
}

/// The row is outside by `pred` at its own schema or at any changed end.
fn any_end(c: &RowsCatalogue, pred: fn(&str) -> String) -> String {
    let mut s = format!("({})", pred(&c.schema));
    for e in &c.changed {
        s.push_str(&format!(" OR (({}) AND {})", e.cond, pred(&e.schema)));
    }
    s
}

/// The name of the first end that is outside by `pred`: the row's own, else a changed end's.
fn end_display(c: &RowsCatalogue, pred: fn(&str) -> String) -> String {
    let mut s = format!("(CASE WHEN {} THEN {}", pred(&c.schema), display(c));
    for e in &c.changed {
        s.push_str(&format!(
            " WHEN ({}) AND {} THEN coalesce(({})::text, '{}:' || ({}))",
            e.cond,
            pred(&e.schema),
            e.display,
            c.name,
            c.identity
        ));
    }
    s.push_str(&format!(" ELSE {} END)", display(c)));
    s
}

fn scope_and(c: &RowsCatalogue) -> String {
    c.scope
        .as_ref()
        .map(|s| format!(" AND {s}"))
        .unwrap_or_default()
}

/// Every row version of the `Rows` catalogues: `(k, ctid, xmin)`. Cheap: no attribution.
fn versions_sql(cats: &[RowsCatalogue]) -> String {
    cats.iter()
        .enumerate()
        .map(|(k, c)| {
            format!(
                "SELECT {k}::int4, x.ctid::text, x.xmin::text::int8 FROM pg_catalog.{name} x WHERE true{scope}",
                name = c.name,
                scope = scope_and(c),
            )
        })
        .collect::<Vec<_>>()
        .join("\nUNION ALL\n")
}

/// Removal, pass 1: the candidate rows of the catalogues `which`, `(k, ctid)`. They are rows of
/// objects the step could remove: an object can be removed only by its owner, so these are objects
/// made after `initdb` (oid 16384 or more), or the `initdb` objects the current role owns through
/// membership (`owned_initdb_sql`). Only this cheap test runs over every row. Attribution runs in
/// pass 2, on the candidates only. Measured in rework 3: a single query mixing this test with
/// attribution spent about 0.1 to 0.6 ms on every catalogue row, and timed the suite out again.
fn candidates_sql(cats: &[RowsCatalogue], which: &BTreeSet<usize>) -> Option<String> {
    let parts: Vec<String> = cats
        .iter()
        .enumerate()
        .filter(|(k, _)| which.contains(k))
        .map(|(k, c)| {
            format!(
                "SELECT {k}::int4, x.ctid::text FROM pg_catalog.{name} x \
                 WHERE (({owner}) >= 16384 OR ({owner}) IN (SELECT o FROM owned)){scope}",
                name = c.name,
                owner = c.owner,
                scope = scope_and(c),
            )
        })
        .collect();
    (!parts.is_empty()).then(|| {
        format!(
            "WITH owned AS ({}) {}",
            owned_initdb_sql(),
            parts.join("\nUNION ALL\n")
        )
    })
}

/// Removal, pass 2, for one catalogue: of the candidate rows (`$2`, ctids), those outside the
/// step's schema (`$1`), or in none, at their own schema or at a changed end: `(identity, name)`.
fn removable_rows_sql(c: &RowsCatalogue) -> String {
    format!(
        "SELECT ({id})::text, {disp} FROM pg_catalog.{name} x \
         WHERE x.ctid = ANY($2::text[]::tid[]) AND ({rem})",
        id = c.identity,
        disp = end_display(c, removable_pred),
        name = c.name,
        rem = any_end(c, removable_pred),
    )
}

/// Of the given row versions of one catalogue (`$2`, ctids), those written by this transaction or
/// a child of it, outside the step's schema: their display names. A child's id is greater than its
/// parent's, so each `xmin` is widened against the top-level id; one more than 2^31 behind it is
/// older and not asked about (the CASE keeps `pg_xact_status` from ever seeing it).
fn written_sql(c: &RowsCatalogue) -> String {
    let ahead =
        "((x.xmin::text::int8 - t.top % 4294967296) % 4294967296 + 4294967296) % 4294967296";
    format!(
        "WITH t AS (SELECT pg_catalog.pg_current_xact_id()::text::int8 AS top) \
         SELECT {disp} FROM pg_catalog.{name} x, t \
         WHERE x.ctid = ANY($2::text[]::tid[]) \
           AND CASE WHEN {ahead} < 2147483648 \
                    THEN pg_catalog.pg_xact_status((t.top + {ahead})::text::xid8) = 'in progress' \
                    ELSE false END \
           AND ({out})",
        disp = end_display(c, written_outside),
        name = c.name,
        out = any_end(c, written_outside),
    )
}

/// This transaction's write counts for every table in `pg_catalog`: `name -> (inserted, updated
/// plus deleted)`.
const COUNTS: &str = "SELECT c.relname::text, \
     pg_catalog.pg_stat_get_xact_tuples_inserted(c.oid), \
     pg_catalog.pg_stat_get_xact_tuples_updated(c.oid) + pg_catalog.pg_stat_get_xact_tuples_deleted(c.oid) \
     FROM pg_catalog.pg_class c \
     WHERE c.relnamespace = 'pg_catalog'::pg_catalog.regnamespace AND c.relkind = 'r'";

/// The guard's settings for its observations (item 8): the session's settings and role reset,
/// and `search_path` pinned to `pg_catalog` (then `pg_temp`) for the rest of the transaction.
const OBSERVE: &str = "RESET ALL; RESET ROLE; SET LOCAL search_path TO pg_catalog, pg_temp";

/// The first holdable cursor open in this session (item 7).
const HOLDABLE_CURSOR: &str =
    "SELECT name FROM pg_catalog.pg_cursors WHERE is_holdable ORDER BY name LIMIT 1";

/// The first other named cursor open in this session (item 7); the guard's own statement runs in
/// the unnamed portal.
const OPEN_CURSOR: &str = "SELECT name FROM pg_catalog.pg_cursors WHERE NOT is_holdable AND name <> '' ORDER BY name LIMIT 1";

/// The first `pg_inherits` link whose two ends are in different schemas, named (item 6).
const CROSS_SCHEMA_LINK: &str = "SELECT pg_catalog.format('%I.%I is %s of %I.%I', cn.nspname, c.relname, \
     CASE WHEN c.relkind IN ('i', 'I') THEN 'a partition index' \
          WHEN c.relispartition AND t.partdefid = c.oid THEN 'the DEFAULT partition' \
          WHEN c.relispartition THEN 'a partition' \
          ELSE 'an inheritance child' END, \
     pn.nspname, p.relname) \
     FROM pg_catalog.pg_inherits i \
     JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid \
     JOIN pg_catalog.pg_namespace cn ON cn.oid = c.relnamespace \
     JOIN pg_catalog.pg_class p ON p.oid = i.inhparent \
     JOIN pg_catalog.pg_namespace pn ON pn.oid = p.relnamespace \
     LEFT JOIN pg_catalog.pg_partitioned_table t ON t.partrelid = p.oid \
     WHERE c.relnamespace <> p.relnamespace \
     ORDER BY cn.nspname, c.relname, pn.nspname, p.relname LIMIT 1";

fn db(e: sqlx::Error) -> MigrateError {
    MigrateError::Db(DbError::from_sqlx(e))
}

type Counts = BTreeMap<String, (i64, i64)>;

async fn counts(conn: &mut PgConnection) -> Result<Counts, MigrateError> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(COUNTS).fetch_all(conn).await.map_err(db)?;
    Ok(rows.into_iter().map(|(n, i, u)| (n, (i, u))).collect())
}

async fn versions(
    conn: &mut PgConnection,
    sql: &str,
) -> Result<BTreeSet<(i32, String, i64)>, MigrateError> {
    let rows: Vec<(i32, String, i64)> = sqlx::query_as(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_all(conn)
        .await
        .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// The removable rows of the catalogues `which` (both passes): `(k, identity)` to the name.
async fn removable(
    conn: &mut PgConnection,
    cats: &[RowsCatalogue],
    which: &BTreeSet<usize>,
    schema: &str,
) -> Result<BTreeMap<(i32, String), String>, MigrateError> {
    let Some(sql) = candidates_sql(cats, which) else {
        return Ok(BTreeMap::new());
    };
    let candidates: Vec<(i32, String)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *conn)
        .await
        .map_err(db)?;
    let mut by: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for (k, ctid) in candidates {
        by.entry(k).or_default().push(ctid);
    }
    let mut out = BTreeMap::new();
    for (k, ctids) in by {
        let rows: Vec<(String, String)> =
            sqlx::query_as(sqlx::AssertSqlSafe(removable_rows_sql(&cats[k as usize])))
                .bind(schema)
                .bind(&ctids)
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?;
        out.extend(rows.into_iter().map(|(id, d)| ((k, id), d)));
    }
    Ok(out)
}

/// The first link across schemas (item 6), if any.
async fn cross_schema_link(conn: &mut PgConnection) -> Result<Option<String>, MigrateError> {
    sqlx::query_scalar(CROSS_SCHEMA_LINK)
        .fetch_optional(conn)
        .await
        .map_err(db)
}

/// Runs `sql` in the open transaction under the guard (the module comment). A starting topology
/// that is not schema-closed refuses the step before it runs. Otherwise the first finding is
/// named: a changed catalogue that no migration may change (by name), else a row written outside
/// the schema, else an outside object removed, each the first by catalogue and name, else a link
/// the step left across schemas.
pub(crate) async fn run_guarded(
    conn: &mut PgConnection,
    schema: &str,
    version: u32,
    sql: &'static str,
) -> Result<(), MigrateError> {
    // Item 8: every observation below reads through pg_catalog, whatever the session set.
    sqlx::raw_sql(OBSERVE)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    let on: bool = sqlx::query_scalar("SELECT pg_catalog.current_setting('track_counts')::bool")
        .fetch_one(&mut *conn)
        .await
        .map_err(db)?;
    if !on {
        return Err(MigrateError::Db(DbError::Other(
            "track_counts is off: the migration guard cannot see catalogue writes".into(),
        )));
    }
    // 0. The starting topology (item 6): the step does not run while any link crosses schemas.
    if let Some(link) = cross_schema_link(conn).await? {
        return Err(MigrateError::StartTopologyNotSchemaClosed { version, link });
    }
    let cats = rows_catalogues();
    let rules: BTreeMap<&str, CatalogueRule> = guard_catalogues().into_iter().collect();
    let all: BTreeSet<usize> = (0..cats.len()).collect();
    let versions_q = versions_sql(&cats);

    let counts_before = counts(conn).await?;
    let old = versions(conn, &versions_q).await?;
    let removable_before = removable(conn, &cats, &all, schema).await?;
    // The step runs with the session's default path, not the guard's pin.
    sqlx::raw_sql("RESET search_path")
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    sqlx::raw_sql(sql).execute(&mut *conn).await.map_err(db)?;
    // Finalisation (item 7), before every observation that follows: the deferred triggers fire
    // now and any queued later is immediate; a holdable cursor refuses the step.
    sqlx::raw_sql("SET CONSTRAINTS ALL IMMEDIATE")
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    // Item 8 again: the step's (and its triggers') settings are reset before the guard looks.
    sqlx::raw_sql(OBSERVE)
        .execute(&mut *conn)
        .await
        .map_err(db)?;
    if let Some(cursor) = sqlx::query_scalar::<_, String>(HOLDABLE_CURSOR)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?
    {
        return Err(MigrateError::StepLeftHoldableCursor { version, cursor });
    }
    if let Some(cursor) = sqlx::query_scalar::<_, String>(OPEN_CURSOR)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?
    {
        return Err(MigrateError::StepLeftOpenCursor { version, cursor });
    }
    let new = versions(conn, &versions_q).await?;
    let counts_after = counts(conn).await?;

    let refuse = |object: String| Err(MigrateError::OutsideSchema { version, object });
    let changed: BTreeMap<String, (i64, i64)> = counts_after
        .iter()
        .filter_map(|(name, (i, u))| {
            let (bi, bu) = counts_before.get(name).copied().unwrap_or((0, 0));
            (i - bi > 0 || u - bu > 0).then(|| (name.clone(), (i - bi, u - bu)))
        })
        .collect();

    // 1. A catalogue no migration may change, or one the table does not know.
    if let Some(name) = changed.keys().find(|name| {
        !matches!(
            rules.get(name.as_str()),
            Some(CatalogueRule::Rows | CatalogueRule::Statistics)
        )
    }) {
        return refuse(format!("pg_catalog.{name}"));
    }

    // 2. A row version this step wrote, outside the schema.
    let mut fresh: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (k, ctid, _) in new.difference(&old) {
        fresh.entry(*k as usize).or_default().push(ctid.clone());
    }
    let mut written: Vec<(usize, String)> = Vec::new();
    for (k, ctids) in fresh {
        let names: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(written_sql(&cats[k])))
            .bind(schema)
            .bind(&ctids)
            .fetch_all(&mut *conn)
            .await
            .map_err(db)?;
        written.extend(names.into_iter().map(|n| (k, n)));
    }
    if let Some((_, object)) = written.into_iter().min() {
        return refuse(object);
    }

    // 3. An outside object removed, or moved into the schema: only in catalogues this transaction
    //    updated or deleted rows of, so a concurrent session's removal elsewhere is not the step's.
    let touched: BTreeSet<usize> = cats
        .iter()
        .enumerate()
        .filter(|(_, c)| changed.get(c.name).is_some_and(|(_, u)| *u > 0))
        .map(|(k, _)| k)
        .collect();
    let removable_after = removable(conn, &cats, &touched, schema).await?;
    let gone = removable_before
        .iter()
        .filter(|((k, _), _)| touched.contains(&(*k as usize)))
        .filter(|(id, _)| !removable_after.contains_key(*id))
        .map(|((k, _), name)| (*k, name.clone()))
        .min();
    if let Some((_, object)) = gone {
        return refuse(object);
    }

    // 4. The topology the step leaves (item 6): the same query, after its statements and before
    //    anything is recorded, whatever statement made the link.
    if let Some(link) = cross_schema_link(conn).await? {
        return Err(MigrateError::StepLeftTopologyNotSchemaClosed { version, link });
    }
    Ok(())
}
