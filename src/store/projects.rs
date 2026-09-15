use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError, StoreResult};
use crate::core::Project;

const PALETTE_SIZE: u8 = 10;

fn row_to_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        color_index: r.get::<_, i64>(2)? as u8,
        archived: r.get::<_, i64>(3)? != 0,
    })
}

impl Store {
    pub fn list_projects(&self, include_archived: bool) -> StoreResult<Vec<Project>> {
        let sql = if include_archived {
            "SELECT id, name, color_index, archived FROM projects ORDER BY name"
        } else {
            "SELECT id, name, color_index, archived FROM projects WHERE archived = 0 ORDER BY name"
        };
        let mut st = self.conn().prepare(sql)?;
        let rows = st.query_map([], row_to_project)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn project_by_name(&self, name: &str) -> StoreResult<Option<Project>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, color_index, archived FROM projects WHERE name = ?1",
                [name],
                row_to_project,
            )
            .optional()?)
    }

    pub fn add_project(&self, name: &str) -> StoreResult<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Constraint(
                "project name must not be empty".into(),
            ));
        }
        if self.project_by_name(name)?.is_some() {
            return Err(StoreError::Constraint(format!(
                "project '{name}' already exists"
            )));
        }
        let count: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?;
        let color = (count as u8) % PALETTE_SIZE;
        self.conn().execute(
            "INSERT INTO projects (name, color_index, archived) VALUES (?1, ?2, 0)",
            params![name, color as i64],
        )?;
        Ok(self.project_by_name(name)?.expect("just inserted"))
    }

    pub fn get_or_create_project(&self, name: &str) -> StoreResult<Project> {
        match self.project_by_name(name.trim())? {
            Some(p) => Ok(p),
            None => self.add_project(name),
        }
    }

    pub fn archive_project(&self, name: &str, archived: bool) -> StoreResult<()> {
        let n = self.conn().execute(
            "UPDATE projects SET archived = ?2 WHERE name = ?1",
            params![name, archived as i64],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("project '{name}'")));
        }
        Ok(())
    }

    pub fn rename_project(&self, old: &str, new: &str) -> StoreResult<()> {
        if self.project_by_name(new)?.is_some() {
            return Err(StoreError::Constraint(format!(
                "project '{new}' already exists"
            )));
        }
        let n = self.conn().execute(
            "UPDATE projects SET name = ?2 WHERE name = ?1",
            params![old, new.trim()],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("project '{old}'")));
        }
        Ok(())
    }

    pub fn last_used_project(&self) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT p.name FROM entries e JOIN projects p ON p.id = e.project_id
                 ORDER BY e.date DESC, e.start_min DESC, e.id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }
}
