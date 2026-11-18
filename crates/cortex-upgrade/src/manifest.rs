//! The release manifest (design §3). `parse` enforces the rules the client holds on its own; a
//! manifest that breaks one is refused as a whole and never partly used.

use crate::version::{Version, parse_version};
use cortex_types::ModuleId;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;

/// A manifest larger than this is refused before it is parsed (1 MiB).
pub const MANIFEST_MAX_BYTES: usize = 1024 * 1024;

/// `sha256:` and 64 lowercase hex digits, as bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Digest(pub [u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub digest: Digest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationClass {
    Expand,
    Contract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerImage {
    pub image_ref: String,
    pub asset: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CortexInfo {
    pub label: String,
    /// Version of the closed release archive member list.
    pub members: u64,
    pub min_upgrade_from: Option<Version>,
    pub server_image: ServerImage,
    pub db_image: String,
    /// The sha256 of `db-roles.sh` as it must be after the unpack (cox 14:30Z, Q-7 condition 2).
    pub roles_script_digest: Digest,
    pub sidecar_images: Vec<String>,
    pub migrations: Vec<String>,
    pub modules: Vec<ModuleId>,
    pub migration_class: MigrationClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub date: String,
    pub notes_url: String,
    pub assets: Vec<Asset>,
    pub cortex: CortexInfo,
}

/// A release entry whose `version` is not SemVer is skipped with a reason, never guessed (UP-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub index: usize,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Usable(Box<Release>),
    Skipped(Skipped),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub entries: Vec<Entry>,
}

/// Which rule a refusal names. The one-line message always carries `Rule::name`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Size,
    Json,
    Schema,
    Shape,
    Digest,
    Modules,
    MigrationClass,
    RolesScriptDigest,
}

impl Rule {
    pub fn name(self) -> &'static str {
        match self {
            Rule::Size => "size",
            Rule::Json => "json",
            Rule::Schema => "schema",
            Rule::Shape => "shape",
            Rule::Digest => "digest",
            Rule::Modules => "modules",
            Rule::MigrationClass => "migration_class",
            Rule::RolesScriptDigest => "roles_script_digest",
        }
    }
}

/// A refusal. No variant holds text from the manifest body beyond a position: a refusal message is
/// built from the rule and indexes only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest refused (size): {len} bytes is over the {MANIFEST_MAX_BYTES} byte limit")]
    TooLarge { len: usize },
    #[error("manifest refused (json): the body is not JSON")]
    NotJson,
    #[error("manifest refused (schema): `schema` must be the number 1")]
    Schema,
    #[error("manifest refused (shape): {what}")]
    Shape { what: String },
    #[error(
        "manifest refused (digest): release {release}, asset {asset} has no valid `sha256:` digest"
    )]
    Digest { release: usize, asset: usize },
    #[error(
        "manifest refused (modules): release {release}, entry {position} is not a valid module id (or the list is empty)"
    )]
    Modules { release: usize, position: usize },
    #[error(
        "manifest refused (migration_class): release {release} must say `expand` or `contract`"
    )]
    MigrationClass { release: usize },
    #[error(
        "manifest refused (roles_script_digest): release {release} must carry a valid `sha256:` digest of db-roles.sh"
    )]
    RolesScriptDigest { release: usize },
}

impl ManifestError {
    pub fn rule(&self) -> Rule {
        match self {
            ManifestError::TooLarge { .. } => Rule::Size,
            ManifestError::NotJson => Rule::Json,
            ManifestError::Schema => Rule::Schema,
            ManifestError::Shape { .. } => Rule::Shape,
            ManifestError::Digest { .. } => Rule::Digest,
            ManifestError::Modules { .. } => Rule::Modules,
            ManifestError::MigrationClass { .. } => Rule::MigrationClass,
            ManifestError::RolesScriptDigest { .. } => Rule::RolesScriptDigest,
        }
    }
}

// ---- the wire form. Only `cortex` and its `server_image` are strict about unknown keys: they are
// our own contract, and a key this client does not know could be a rule it does not enforce.
// `digest`, `migration_class` and each module entry arrive as plain values so that a wrong one is
// refused under its own rule, not as a generic shape error.

#[derive(Deserialize)]
struct RawRelease {
    // `version` was read and checked before this struct is built.
    date: String,
    notes_url: String,
    assets: Vec<RawAsset>,
    cortex: RawCortex,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    url: String,
    #[serde(default)]
    digest: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCortex {
    label: String,
    members: u64,
    min_upgrade_from: Option<String>,
    server_image: RawServerImage,
    db_image: String,
    #[serde(default)]
    roles_script_digest: Value,
    #[serde(default)]
    sidecar_images: Vec<String>,
    migrations: Vec<String>,
    modules: Vec<Value>,
    #[serde(default)]
    migration_class: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServerImage {
    #[serde(rename = "ref")]
    image_ref: String,
    asset: String,
}

fn shape(what: impl Into<String>) -> ManifestError {
    ManifestError::Shape { what: what.into() }
}

/// `sha256:` and exactly 64 lowercase hex digits.
fn parse_digest(v: &Value) -> Option<Digest> {
    let hex = v.as_str()?.strip_prefix("sha256:")?;
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let nib = |c: u8| match c {
            b'0'..=b'9' => Some(c - b'0'),
            b'a'..=b'f' => Some(c - b'a' + 10),
            _ => None,
        };
        out[i] = nib(pair[0])? << 4 | nib(pair[1])?;
    }
    Some(Digest(out))
}

fn release_from(index: usize, raw: RawRelease, version: Version) -> Result<Release, ManifestError> {
    let mut assets = Vec::with_capacity(raw.assets.len());
    for (asset, a) in raw.assets.into_iter().enumerate() {
        let digest = parse_digest(&a.digest).ok_or(ManifestError::Digest {
            release: index,
            asset,
        })?;
        assets.push(Asset {
            name: a.name,
            url: a.url,
            digest,
        });
    }
    let c = raw.cortex;
    let roles_script_digest = parse_digest(&c.roles_script_digest)
        .ok_or(ManifestError::RolesScriptDigest { release: index })?;
    if c.modules.is_empty() {
        return Err(ManifestError::Modules {
            release: index,
            position: 0,
        });
    }
    let mut modules = Vec::with_capacity(c.modules.len());
    for (position, m) in c.modules.iter().enumerate() {
        let id = m.as_str().and_then(|s| ModuleId::parse(s).ok());
        modules.push(id.ok_or(ManifestError::Modules {
            release: index,
            position,
        })?);
    }
    let migration_class = match c.migration_class.as_str() {
        Some("expand") => MigrationClass::Expand,
        Some("contract") => MigrationClass::Contract,
        _ => return Err(ManifestError::MigrationClass { release: index }),
    };
    let min_upgrade_from = match c.min_upgrade_from {
        None => None,
        Some(s) => Some(
            parse_version(&s)
                .map_err(|_| shape(format!("release {index}: min_upgrade_from is not SemVer")))?,
        ),
    };
    Ok(Release {
        version,
        date: raw.date,
        notes_url: raw.notes_url,
        assets,
        cortex: CortexInfo {
            label: c.label,
            members: c.members,
            min_upgrade_from,
            server_image: ServerImage {
                image_ref: c.server_image.image_ref,
                asset: c.server_image.asset,
            },
            db_image: c.db_image,
            roles_script_digest,
            sidecar_images: c.sidecar_images,
            migrations: c.migrations,
            modules,
            migration_class,
        },
    })
}

/// The rules of design §3 that need no network and no file. The size is checked before anything
/// is parsed; a refusal of any kind refuses the whole manifest (nothing is partly used); an entry
/// whose `version` is not SemVer is the one thing that is skipped instead (UP-14).
pub fn parse(body: &[u8]) -> Result<Manifest, ManifestError> {
    if body.len() > MANIFEST_MAX_BYTES {
        return Err(ManifestError::TooLarge { len: body.len() });
    }
    let top: Value = serde_json::from_slice(body).map_err(|_| ManifestError::NotJson)?;
    let top = top
        .as_object()
        .ok_or_else(|| shape("the top level must be an object"))?;
    if top.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err(ManifestError::Schema);
    }
    let releases = top
        .get("releases")
        .and_then(Value::as_array)
        .ok_or_else(|| shape("`releases` must be a list"))?;
    let mut seen = HashSet::new();
    let mut entries = Vec::with_capacity(releases.len());
    for (index, value) in releases.iter().enumerate() {
        let text = value
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| shape(format!("release {index}: `version` must be a string")))?;
        let version = match parse_version(text) {
            Ok(v) => v,
            Err(e) => {
                entries.push(Entry::Skipped(Skipped {
                    index,
                    reason: format!("release {index} skipped: {e}"),
                }));
                continue;
            }
        };
        let raw = RawRelease::deserialize(value).map_err(|_| {
            shape(format!(
                "release {index}: a field is missing, mistyped or not known"
            ))
        })?;
        if !seen.insert(version.clone()) {
            return Err(shape(format!(
                "release {index}: the same version is listed twice"
            )));
        }
        entries.push(Entry::Usable(Box::new(release_from(index, raw, version)?)));
    }
    Ok(Manifest { entries })
}
