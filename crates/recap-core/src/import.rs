//! Manual import: single GPX files, gzipped GPX, folders, and Strava bulk
//! exports (zip or unzipped folder, using `activities.csv` for names/types).

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

use crate::library::AddOutcome;
use crate::{ActivityKind, Error, Library, Result};

#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    pub added: usize,
    /// Added, but not runs (e.g. rides) — hidden by the default filter.
    pub added_other: usize,
    pub duplicates: usize,
    /// `.fit` / `.tcx` files we can't read yet.
    pub unsupported: usize,
    pub failed: Vec<(String, String)>,
}

impl ImportReport {
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("{} runs imported", self.added)];
        if self.added_other > 0 {
            parts.push(format!("{} other activities", self.added_other));
        }
        if self.duplicates > 0 {
            parts.push(format!("{} duplicates skipped", self.duplicates));
        }
        if self.unsupported > 0 {
            parts.push(format!(
                "{} skipped (.fit/.tcx not supported yet)",
                self.unsupported
            ));
        }
        if !self.failed.is_empty() {
            parts.push(format!("{} failed", self.failed.len()));
        }
        parts.join(" · ")
    }

    fn record(&mut self, source: &str, result: Result<(AddOutcome, ActivityKind)>) {
        match result {
            Ok((AddOutcome::Added, ActivityKind::Run)) => self.added += 1,
            Ok((AddOutcome::Added, _)) => self.added_other += 1,
            Ok((AddOutcome::Duplicate, _)) => self.duplicates += 1,
            Err(e) => {
                tracing::warn!(source, error = %e, "import failed");
                self.failed.push((source.to_owned(), e.to_string()));
            }
        }
    }
}

#[derive(Debug, Clone)]
struct CsvMeta {
    name: String,
    kind: Option<ActivityKind>,
}

enum FileKind {
    Gpx,
    GpxGz,
    Zip,
    Unsupported,
    Ignored,
}

fn classify(name: &str) -> FileKind {
    let n = name.to_ascii_lowercase();
    if n.ends_with(".gpx") {
        FileKind::Gpx
    } else if n.ends_with(".gpx.gz") {
        FileKind::GpxGz
    } else if n.ends_with(".zip") {
        FileKind::Zip
    } else if [".fit", ".fit.gz", ".tcx", ".tcx.gz"]
        .iter()
        .any(|e| n.ends_with(e))
    {
        FileKind::Unsupported
    } else {
        FileKind::Ignored
    }
}

/// Import any mix of files and folders. Saves the library index at the end.
pub fn import_paths(lib: &mut Library, paths: &[PathBuf]) -> ImportReport {
    let mut report = ImportReport::default();
    for path in paths {
        import_path(lib, path, &mut report);
    }
    if let Err(e) = lib.save() {
        report.failed.push(("library index".into(), e.to_string()));
    }
    report
}

fn import_path(lib: &mut Library, path: &Path, report: &mut ImportReport) {
    let label = path.display().to_string();
    if path.is_dir() {
        let csv = path.join("activities.csv");
        let meta = if csv.exists() {
            match fs::read(&csv)
                .map_err(Error::from)
                .and_then(|b| read_strava_csv(&b))
            {
                Ok(m) => m,
                Err(e) => {
                    report
                        .failed
                        .push((csv.display().to_string(), e.to_string()));
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };
        let mut files = Vec::new();
        walk(path, &mut files);
        files.sort();
        for f in files {
            let rel = f
                .strip_prefix(path)
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            let m = meta.get(&rel).cloned();
            import_file(lib, &f, m, report);
        }
        return;
    }
    if let FileKind::Zip = classify(&label) {
        if let Err(e) = import_zip(lib, path, report) {
            report.failed.push((label, e.to_string()));
        }
        return;
    }
    import_file(lib, path, None, report);
}

fn import_file(lib: &mut Library, path: &Path, meta: Option<CsvMeta>, report: &mut ImportReport) {
    let label = path.display().to_string();
    let kind = classify(&label);
    let bytes = match kind {
        FileKind::Gpx => fs::read(path).map_err(Error::from),
        FileKind::GpxGz => fs::read(path).map_err(Error::from).and_then(|b| gunzip(&b)),
        FileKind::Zip => {
            if let Err(e) = import_zip(lib, path, report) {
                report.failed.push((label, e.to_string()));
            }
            return;
        }
        FileKind::Unsupported => {
            report.unsupported += 1;
            return;
        }
        FileKind::Ignored => return,
    };
    let result = bytes.and_then(|b| add(lib, &b, meta));
    report.record(&label, result);
}

fn import_zip(lib: &mut Library, path: &Path, report: &mut ImportReport) -> Result<()> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path)?)?;

    // Strava exports may or may not have a top-level folder; find the CSV anywhere.
    let mut meta = HashMap::new();
    let mut prefix = String::new();
    let csv_name = zip
        .file_names()
        .find(|n| n.ends_with("activities.csv"))
        .map(str::to_owned);
    if let Some(csv_name) = csv_name {
        prefix = csv_name.trim_end_matches("activities.csv").to_owned();
        let mut buf = Vec::new();
        zip.by_name(&csv_name)?.read_to_end(&mut buf)?;
        meta = read_strava_csv(&buf)?;
    }

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let label = format!("{}:{name}", path.display());
        let kind = classify(&name);
        if matches!(kind, FileKind::Unsupported) {
            report.unsupported += 1;
            continue;
        }
        if !matches!(kind, FileKind::Gpx | FileKind::GpxGz) {
            continue;
        }
        let mut raw = Vec::new();
        if let Err(e) = entry.read_to_end(&mut raw) {
            report.failed.push((label, e.to_string()));
            continue;
        }
        let bytes = match kind {
            FileKind::GpxGz => gunzip(&raw),
            _ => Ok(raw),
        };
        let rel = name.strip_prefix(&prefix).unwrap_or(&name);
        let result = bytes.and_then(|b| add(lib, &b, meta.get(rel).cloned()));
        report.record(&label, result);
    }
    Ok(())
}

fn add(
    lib: &mut Library,
    bytes: &[u8],
    meta: Option<CsvMeta>,
) -> Result<(AddOutcome, ActivityKind)> {
    let (name, kind) = meta.map_or((None, None), |m| (Some(m.name), m.kind));
    lib.add_gpx(bytes, name, kind)
}

fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Map of export-relative filename (e.g. `activities/123.gpx.gz`) → metadata.
fn read_strava_csv(bytes: &[u8]) -> Result<HashMap<String, CsvMeta>> {
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(bytes);
    let headers = rdr.headers()?.clone();
    let col = |name: &str| headers.iter().position(|h| h.trim() == name);
    let (Some(file_col), Some(name_col), Some(type_col)) =
        (col("Filename"), col("Activity Name"), col("Activity Type"))
    else {
        return Ok(HashMap::new());
    };
    let mut map = HashMap::new();
    for rec in rdr.records() {
        let rec = rec?;
        let file = rec.get(file_col).unwrap_or("").trim();
        if file.is_empty() {
            continue;
        }
        map.insert(
            file.to_owned(),
            CsvMeta {
                name: rec.get(name_col).unwrap_or("").trim().to_owned(),
                kind: rec.get(type_col).and_then(ActivityKind::from_label),
            },
        );
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strava_csv_maps_filenames() {
        let csv = "Activity ID,Activity Date,Activity Name,Activity Type,Filename\n\
                   1,\"Oct 3, 2026, 6:12:00 AM\",Morning Run,Run,activities/1.gpx.gz\n\
                   2,\"Oct 4, 2026, 6:12:00 AM\",Commute,Ride,activities/2.gpx\n\
                   3,\"Oct 5, 2026, 6:12:00 AM\",Treadmill,Run,\n";
        let m = read_strava_csv(csv.as_bytes()).unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m["activities/1.gpx.gz"].name, "Morning Run");
        assert_eq!(m["activities/2.gpx"].kind, Some(ActivityKind::Ride));
    }
}
