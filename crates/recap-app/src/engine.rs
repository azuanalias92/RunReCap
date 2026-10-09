//! Platform-independent app logic shared by the CLI and the menu-bar app:
//! which run is current, rotation, and rendering a wallpaper for a canvas.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use recap_core::{import, Activity, ImportReport, Library};
use recap_render::{Canvas, Fonts, Layout, Pixmap, RenderNotes, Scene, Theme};

use crate::config::{Config, Order, State};

pub struct Engine {
    pub dir: PathBuf,
    pub library: Library,
    pub config: Config,
    pub state: State,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn eligible<'a>(lib: &'a Library, cfg: &Config) -> Vec<&'a Activity> {
    let filter = cfg.filter();
    lib.activities()
        .iter()
        .filter(|a| filter.matches(a))
        .collect()
}

impl Engine {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
            library: Library::open(dir.join("library"))?,
            config: Config::load(dir)?,
            state: State::load(dir),
        })
    }

    pub fn reload_config(&mut self) -> Result<()> {
        self.config = Config::load(&self.dir)?;
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        self.config.save(&self.dir)?;
        self.state.save(&self.dir)
    }

    pub fn eligible(&self) -> Vec<&Activity> {
        eligible(&self.library, &self.config)
    }

    pub fn current(&self) -> Option<&Activity> {
        let runs = self.eligible();
        let id = self.state.current_id.as_deref();
        runs.iter()
            .find(|a| Some(a.id.as_str()) == id)
            .or(runs.first())
            .copied()
    }

    fn set_current(&mut self, id: Option<String>) {
        self.state.current_id = id;
        self.state.last_change = now();
    }

    /// Move through runs in newest-first order; +1 = older ("Next").
    pub fn step(&mut self, delta: isize) {
        let ids: Vec<String> = self.eligible().iter().map(|a| a.id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let cur = self.current().map(|a| a.id.clone());
        let i = cur
            .and_then(|c| ids.iter().position(|x| *x == c))
            .unwrap_or(0) as isize;
        let n = ids.len() as isize;
        let next = (i + delta).rem_euclid(n) as usize;
        self.set_current(Some(ids[next].clone()));
    }

    pub fn shuffle(&mut self) {
        let ids: Vec<String> = self.eligible().iter().map(|a| a.id.clone()).collect();
        if ids.len() < 2 {
            return self.step(1);
        }
        let cur = self.current().map(|a| a.id.clone());
        let mut seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64)
            | 1;
        loop {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let pick = &ids[(seed % ids.len() as u64) as usize];
            if Some(pick) != cur.as_ref() {
                return self.set_current(Some(pick.clone()));
            }
        }
    }

    /// Advance if the rotation interval has passed. Returns true if it changed.
    pub fn rotate_if_due(&mut self) -> bool {
        let every = self.config.rotation.every_minutes as i64 * 60;
        if every == 0 || self.state.pinned || now() - self.state.last_change < every {
            return false;
        }
        match self.config.rotation.order {
            Order::Newest => self.step(1),
            Order::Shuffle => self.shuffle(),
        }
        true
    }

    pub fn import(&mut self, paths: &[PathBuf]) -> ImportReport {
        let before = self.library.activities().first().map(|a| a.id.clone());
        let report = import::import_paths(&mut self.library, paths);
        // Show the newest run after an import that added something newer.
        let after = self.library.activities().first().map(|a| a.id.clone());
        if report.added > 0 && after != before {
            self.set_current(after);
        }
        report
    }

    /// Snapshot everything needed to render the current run, so rendering can
    /// happen on another thread without holding the engine. None = no runs.
    pub fn render_job(&self, system_dark: bool, utc_offset_s: i32) -> Option<RenderJob> {
        let current = self.current()?.clone();
        let all = match self.config.layout {
            Layout::Single => Vec::new(),
            _ => self.eligible().into_iter().cloned().collect(),
        };
        Some(RenderJob {
            theme: self.config.active_theme(system_dark),
            layout: self.config.layout,
            privacy_trim_m: self.config.privacy_trim_m,
            utc_offset_s,
            current,
            all,
            dir: self.dir.clone(),
        })
    }
}

/// A self-contained render request (see [`Engine::render_job`]).
#[derive(Clone)]
pub struct RenderJob {
    pub theme: Theme,
    pub layout: Layout,
    pub privacy_trim_m: f64,
    pub utc_offset_s: i32,
    pub current: Activity,
    /// Every eligible run; empty for the single layout.
    pub all: Vec<Activity>,
    pub dir: PathBuf,
}

impl RenderJob {
    pub fn render(
        &self,
        canvas: Canvas,
        fonts: &mut Fonts,
        show_caption: bool,
    ) -> Result<(Pixmap, RenderNotes)> {
        let mut all: Vec<&Activity> = self.all.iter().collect();
        if all.is_empty() {
            all.push(&self.current);
        }
        let scene = Scene {
            layout: self.layout,
            current: &self.current,
            all: &all,
            privacy_trim_m: self.privacy_trim_m,
            utc_offset_s: self.utc_offset_s,
            show_caption,
        };
        Ok(recap_render::render(
            &scene,
            &self.theme,
            canvas,
            fonts,
            &self.dir,
        )?)
    }
}
