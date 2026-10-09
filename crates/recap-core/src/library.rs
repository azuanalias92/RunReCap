//! The on-disk run library: imported GPX copies plus a JSON index of parsed,
//! simplified activities (so GPX files are never re-parsed).
//!
//! Layout:
//! ```text
//! <dir>/index.json
//! <dir>/tracks/<id>.gpx
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::{gpx, Activity, ActivityKind, Result};

pub struct Library {
    dir: PathBuf,
    /// Sorted newest first.
    activities: Vec<Activity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddOutcome {
    Added,
    Duplicate,
}

/// Which activities are eligible to be shown.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub runs_only: bool,
    pub min_distance_m: f64,
    /// Unix seconds, inclusive.
    pub since: Option<i64>,
    pub until: Option<i64>,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            runs_only: true,
            min_distance_m: 0.0,
            since: None,
            until: None,
        }
    }
}

impl Filter {
    pub fn matches(&self, a: &Activity) -> bool {
        if self.runs_only && a.kind != ActivityKind::Run {
            return false;
        }
        if a.distance_m < self.min_distance_m {
            return false;
        }
        match (a.start, self.since, self.until) {
            (Some(t), Some(s), _) if t < s => false,
            (Some(t), _, Some(u)) if t > u => false,
            _ => true,
        }
    }
}

impl Library {
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("tracks"))?;
        let index = dir.join("index.json");
        let activities = if index.exists() {
            serde_json::from_slice(&fs::read(&index)?)?
        } else {
            Vec::new()
        };
        let mut lib = Self { dir, activities };
        lib.sort();
        Ok(lib)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// All activities, newest first.
    pub fn activities(&self) -> &[Activity] {
        &self.activities
    }

    pub fn filtered<'a>(&'a self, filter: &'a Filter) -> impl Iterator<Item = &'a Activity> + 'a {
        self.activities.iter().filter(move |a| filter.matches(a))
    }

    pub fn get(&self, id: &str) -> Option<&Activity> {
        self.activities.iter().find(|a| a.id == id)
    }

    /// Parse and add one GPX document, returning the stored activity kind. `name`/`kind` override what the GPX says
    /// (used for Strava's `activities.csv`).
    pub fn add_gpx(
        &mut self,
        bytes: &[u8],
        name: Option<String>,
        kind: Option<ActivityKind>,
    ) -> Result<(AddOutcome, ActivityKind)> {
        let id = content_id(bytes);
        if let Some(existing) = self.get(&id) {
            return Ok((AddOutcome::Duplicate, existing.kind));
        }
        let parsed = gpx::parse(bytes)?;
        let kind = kind.or_else(|| parsed.kind.as_deref().and_then(ActivityKind::from_label));
        let name = name
            .filter(|n| !n.trim().is_empty())
            .or(parsed.name)
            .unwrap_or_else(|| "Run".to_owned());
        let activity = Activity::from_points(id.clone(), name, kind, &parsed.points)?;
        fs::write(self.dir.join("tracks").join(format!("{id}.gpx")), bytes)?;
        let kind = activity.kind;
        self.activities.push(activity);
        self.sort();
        Ok((AddOutcome::Added, kind))
    }

    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let before = self.activities.len();
        self.activities.retain(|a| a.id != id);
        let track = self.dir.join("tracks").join(format!("{id}.gpx"));
        if track.exists() {
            fs::remove_file(track)?;
        }
        Ok(self.activities.len() != before)
    }

    /// Write the index atomically (temp file + rename).
    pub fn save(&self) -> Result<()> {
        let tmp = self.dir.join("index.json.tmp");
        fs::write(&tmp, serde_json::to_vec(&self.activities)?)?;
        fs::rename(tmp, self.dir.join("index.json"))?;
        Ok(())
    }

    fn sort(&mut self) {
        self.activities
            .sort_by(|a, b| b.start.cmp(&a.start).then_with(|| a.id.cmp(&b.id)));
    }
}

fn content_id(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest[..10].iter().map(|b| format!("{b:02x}")).collect()
}
