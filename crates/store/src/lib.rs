//! SQLite persistence: projects and their immutable OpenAPI snapshots.
//!
//! A single connection behind a mutex is enough for SQLite (one writer at a
//! time) and keeps snapshot appends serialisable. Callers on an async runtime
//! should use `spawn_blocking`.

mod migrations;

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::Serialize;
use serde_json::Value;

pub use migrations::latest_schema_version;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("project not found")]
    ProjectNotFound,
    #[error("project has no baseline snapshot")]
    NoBaseline,
    #[error("version already exists for this project")]
    DuplicateVersion,
    #[error("stored data is corrupt: {0}")]
    Corrupt(String),
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("could not create the database directory: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// Row of `GET /api/projects`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: i64,
    pub name: String,
    pub repository: String,
    pub latest_version_id: Option<i64>,
    pub latest_version: Option<String>,
    pub breaking_count: Option<i64>,
    pub added_count: Option<i64>,
    pub updated_at: Option<String>,
}

/// Row of `GET /api/projects/:id/versions`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionSummary {
    pub id: i64,
    pub version: String,
    pub breaking_count: i64,
    pub added_count: i64,
    pub warning_count: i64,
    pub suggested_version: Option<String>,
    pub created_at: String,
    pub change_summary: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSpec {
    pub id: i64,
    pub version: String,
    pub spec_json: String,
}

/// Latest snapshot of a project, used as the comparison baseline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Baseline {
    pub id: i64,
    pub version: String,
    pub spec_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSnapshot {
    pub version: String,
    pub spec_json: String,
    pub breaking_count: i64,
    pub added_count: i64,
    pub warning_count: i64,
    pub suggested_version: Option<String>,
    pub change_summary_json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreatedProject {
    pub id: i64,
    pub version_id: i64,
}

#[derive(Debug)]
pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Opens (creating if needed) the database file and applies pending migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    /// Private in-memory database (tests).
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrations::apply(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> MutexGuard<'_, Connection> {
        // A panic mid-statement leaves no open transaction (dropped = rolled back).
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// Cheap liveness probe for health checks.
    pub fn ping(&self) -> Result<()> {
        self.conn().query_row("SELECT 1", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn count_projects(&self) -> Result<i64> {
        Ok(self
            .conn()
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectSummary>> {
        let conn = self.conn();
        let mut stmt = conn.prepare_cached(
            "SELECT p.id, p.name, p.repository, v.id, v.version, v.breaking_count, v.added_count, v.created_at
             FROM projects p
             LEFT JOIN versions v ON v.id = (SELECT id FROM versions WHERE project_id = p.id ORDER BY id DESC LIMIT 1)
             ORDER BY p.id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ProjectSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                repository: r.get(2)?,
                latest_version_id: r.get(3)?,
                latest_version: r.get(4)?,
                breaking_count: r.get(5)?,
                added_count: r.get(6)?,
                updated_at: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn project_exists(&self, project_id: i64) -> Result<bool> {
        project_exists(&self.conn(), project_id)
    }

    /// Creates a project with its baseline snapshot atomically.
    pub fn create_project(
        &self,
        name: &str,
        repository: &str,
        baseline: &NewSnapshot,
    ) -> Result<CreatedProject> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO projects (name, repository) VALUES (?1, ?2)",
            params![name, repository],
        )?;
        let id = tx.last_insert_rowid();
        let version_id = insert_snapshot(&tx, id, baseline)?;
        tx.commit()?;
        Ok(CreatedProject { id, version_id })
    }

    /// Snapshots of a project, newest first; `None` when the project does not exist.
    pub fn list_versions(&self, project_id: i64) -> Result<Option<Vec<VersionSummary>>> {
        let conn = self.conn();
        if !project_exists(&conn, project_id)? {
            return Ok(None);
        }
        let mut stmt = conn.prepare_cached(
            "SELECT id, version, breaking_count, added_count, warning_count, suggested_version, created_at, change_summary
             FROM versions WHERE project_id = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([project_id], |r| {
            Ok((
                VersionSummary {
                    id: r.get(0)?,
                    version: r.get(1)?,
                    breaking_count: r.get(2)?,
                    added_count: r.get(3)?,
                    warning_count: r.get(4)?,
                    suggested_version: r.get(5)?,
                    created_at: r.get(6)?,
                    change_summary: Value::Null,
                },
                r.get::<_, String>(7)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (mut summary, raw) = row?;
            summary.change_summary = serde_json::from_str(&raw).map_err(|e| {
                StoreError::Corrupt(format!("change summary of version {}: {e}", summary.id))
            })?;
            out.push(summary);
        }
        Ok(Some(out))
    }

    pub fn get_spec(&self, project_id: i64, version_id: i64) -> Result<Option<StoredSpec>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, version, spec_json FROM versions WHERE id = ?1 AND project_id = ?2",
                [version_id, project_id],
                |r| {
                    Ok(StoredSpec {
                        id: r.get(0)?,
                        version: r.get(1)?,
                        spec_json: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Reads the latest snapshot and appends a new one inside a single write
    /// transaction, so concurrent uploads always diff against the true latest.
    ///
    /// `build` receives the baseline and returns the snapshot to store plus any
    /// value the caller wants back.
    pub fn append_snapshot<T, E>(
        &self,
        project_id: i64,
        build: impl FnOnce(&Baseline) -> Result<(NewSnapshot, T), E>,
    ) -> Result<(i64, T), E>
    where
        E: From<StoreError>,
    {
        let mut conn = self.conn();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(StoreError::from)?;
        if !project_exists(&tx, project_id)? {
            return Err(StoreError::ProjectNotFound.into());
        }
        let baseline = tx
            .query_row(
                "SELECT id, version, spec_json FROM versions WHERE project_id = ?1 ORDER BY id DESC LIMIT 1",
                [project_id],
                |r| {
                    Ok(Baseline {
                        id: r.get(0)?,
                        version: r.get(1)?,
                        spec_json: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(StoreError::from)?
            .ok_or(StoreError::NoBaseline)?;
        let (snapshot, extra) = build(&baseline)?;
        let id = insert_snapshot(&tx, project_id, &snapshot)?;
        tx.commit().map_err(StoreError::from)?;
        Ok((id, extra))
    }
}

fn project_exists(conn: &Connection, project_id: i64) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM projects WHERE id = ?1", [project_id], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

fn insert_snapshot(conn: &Connection, project_id: i64, s: &NewSnapshot) -> Result<i64> {
    let result = conn.execute(
        "INSERT INTO versions (project_id, version, spec_json, breaking_count, added_count, warning_count, suggested_version, change_summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            project_id,
            s.version,
            s.spec_json,
            s.breaking_count,
            s.added_count,
            s.warning_count,
            s.suggested_version,
            s.change_summary_json
        ],
    );
    match result {
        Ok(_) => Ok(conn.last_insert_rowid()),
        Err(e) if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) => {
            Err(StoreError::DuplicateVersion)
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(version: &str) -> NewSnapshot {
        NewSnapshot {
            version: version.to_owned(),
            spec_json: format!(r#"{{"v":"{version}"}}"#),
            breaking_count: 1,
            added_count: 2,
            warning_count: 3,
            suggested_version: Some("2.0.0".to_owned()),
            change_summary_json: r#"[{"severity":"breaking"}]"#.to_owned(),
        }
    }

    #[test]
    fn migrations_reach_latest_version() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(
            store.schema_version().expect("version"),
            latest_schema_version()
        );
    }

    #[test]
    fn project_lifecycle() {
        let store = Store::open_in_memory().expect("open");
        assert_eq!(store.count_projects().expect("count"), 0);
        let created = store
            .create_project("Billing", "acme/billing", &snapshot("1.0.0"))
            .expect("create");
        assert!(store.project_exists(created.id).expect("exists"));

        let (id, extra) = store
            .append_snapshot::<_, StoreError>(created.id, |base| {
                assert_eq!(base.version, "1.0.0");
                Ok((snapshot("2.0.0"), "extra"))
            })
            .expect("append");
        assert_eq!(extra, "extra");

        let projects = store.list_projects().expect("list");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].latest_version_id, Some(id));
        assert_eq!(projects[0].latest_version.as_deref(), Some("2.0.0"));

        let versions = store
            .list_versions(created.id)
            .expect("versions")
            .expect("some");
        assert_eq!(
            versions
                .iter()
                .map(|v| v.version.as_str())
                .collect::<Vec<_>>(),
            ["2.0.0", "1.0.0"]
        );
        assert_eq!(versions[0].warning_count, 3);
        assert_eq!(versions[0].change_summary[0]["severity"], "breaking");

        let spec = store.get_spec(created.id, id).expect("spec").expect("some");
        assert_eq!(spec.spec_json, r#"{"v":"2.0.0"}"#);
        assert!(store.get_spec(created.id + 1, id).expect("spec").is_none());
    }

    #[test]
    fn duplicate_versions_are_rejected() {
        let store = Store::open_in_memory().expect("open");
        let created = store
            .create_project("Billing", "", &snapshot("1.0.0"))
            .expect("create");
        let err = store
            .append_snapshot::<(), StoreError>(created.id, |_| Ok((snapshot("1.0.0"), ())))
            .expect_err("duplicate");
        assert!(matches!(err, StoreError::DuplicateVersion));
        // The failed append must not leave a partial row behind.
        let versions = store.list_versions(created.id).expect("q").expect("some");
        assert_eq!(versions.len(), 1);
    }

    #[test]
    fn append_to_unknown_project_fails() {
        let store = Store::open_in_memory().expect("open");
        let err = store
            .append_snapshot::<(), StoreError>(42, |_| Ok((snapshot("1.0.0"), ())))
            .expect_err("missing");
        assert!(matches!(err, StoreError::ProjectNotFound));
        assert!(store.list_versions(42).expect("query").is_none());
    }

    #[test]
    fn legacy_node_database_is_upgraded_in_place() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("nested").join("legacy.sqlite");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        {
            let conn = Connection::open(&path).expect("open legacy");
            conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
                .expect("legacy schema");
            conn.execute(
                "INSERT INTO projects (name, repository) VALUES ('Old', 'o/r')",
                [],
            )
            .expect("project");
            conn.execute(
                "INSERT INTO versions (project_id, version, spec_json) VALUES (1, '1.0.0', '{}')",
                [],
            )
            .expect("version");
        }
        let store = Store::open(&path).expect("upgrade");
        assert_eq!(store.schema_version().expect("v"), latest_schema_version());
        let versions = store.list_versions(1).expect("versions").expect("some");
        assert_eq!(versions[0].warning_count, 0);
        assert_eq!(versions[0].suggested_version, None);
        assert_eq!(versions[0].change_summary, serde_json::json!([]));
    }

    #[test]
    fn open_creates_missing_directories() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("a").join("b").join("db.sqlite");
        Store::open(&path).expect("open").ping().expect("ping");
        assert!(path.exists());
    }
}
