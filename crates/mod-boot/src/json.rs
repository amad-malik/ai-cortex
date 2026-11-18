//! The JSON form (S3 r2.6 `Boot`, `Whoami`, `Orientation`, `SectionPointer`, `BootHandoff`,
//! `BootSection`): the same selection as the Markdown, field for field. `null` is written, never
//! skipped, where the schema allows it.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Boot {
    pub identity: Whoami,
    pub orientation: Option<Orientation>,
    pub rules: Vec<SectionPointer>,
    pub claimed: Vec<BootHandoff>,
    pub pending: Vec<BootHandoff>,
    pub budget: u32,
    pub used: usize,
    pub sections: Vec<BootSection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Whoami {
    pub installation: String,
    pub project: String,
    pub worker: String,
    pub name: String,
    pub kind: &'static str,
    pub role: &'static str,
    pub team: Option<String>,
    pub job_roles: Vec<String>,
    pub token_id: String,
    pub expires_at: String,
    pub renew_due: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Orientation {
    pub record_id: String,
    pub revision: i32,
    pub author: String,
    pub created_at: String,
    pub body: String,
    /// The record's `fields.reading_path`, whole and in order (cox W1-20, S3 r2.6): the floor
    /// form shortens only the Markdown's list, never this one.
    pub reading_path: Vec<String>,
    pub cut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionPointer {
    pub section: String,
    pub record_id: String,
    pub revision: i32,
    pub sha256: String,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootHandoff {
    pub id: String,
    pub kind: &'static str,
    pub state: &'static str,
    pub priority: i32,
    pub created_by: String,
    pub created_at: String,
    pub summary: String,
    pub summary_cut: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootSection {
    pub name: &'static str,
    pub status: &'static str,
    pub used: usize,
    pub omitted: usize,
}
