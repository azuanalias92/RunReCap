use serde::{Deserialize, Serialize};

use crate::geo;

/// One track point. Kept small because the library index holds every run.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ele: Option<f32>,
    /// Unix timestamp, seconds (UTC).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t: Option<i64>,
    /// Heart rate, bpm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hr: Option<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityKind {
    Run,
    Walk,
    Ride,
    Other,
}

impl ActivityKind {
    /// Classify from a GPX `<type>` or Strava "Activity Type" value.
    pub fn from_label(label: &str) -> Option<Self> {
        let l = label.trim().to_ascii_lowercase();
        if l.is_empty() {
            return None;
        }
        // Strava GPX files use numeric codes: 9 = run, 1 = ride, 10 = walk, 11 = hike.
        Some(match l.as_str() {
            "9" => Self::Run,
            "1" => Self::Ride,
            "10" | "11" => Self::Walk,
            _ if l.contains("run") || l.contains("jog") => Self::Run,
            _ if l.contains("walk") || l.contains("hike") => Self::Walk,
            _ if l.contains("ride") || l.contains("cycl") || l.contains("bik") => Self::Ride,
            _ => Self::Other,
        })
    }

    /// Fallback when no label is present: guess from average speed.
    pub fn from_speed(avg_mps: f64) -> Self {
        match avg_mps {
            s if s < 1.6 => Self::Walk,
            s if s <= 6.5 => Self::Run,
            _ => Self::Ride,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    /// Content hash of the source file; also the library file name.
    pub id: String,
    pub name: String,
    pub kind: ActivityKind,
    /// Unix timestamp of the first point, if the GPX has times.
    pub start: Option<i64>,
    pub distance_m: f64,
    /// Elapsed time from first to last point.
    pub duration_s: Option<i64>,
    pub elev_gain_m: Option<f64>,
    /// Simplified track (see [`geo::simplify`]).
    pub points: Vec<Point>,
}

impl Activity {
    /// Build an activity from raw points: computes stats, simplifies the track.
    pub fn from_points(
        id: String,
        name: String,
        kind: Option<ActivityKind>,
        raw: &[Point],
    ) -> crate::Result<Self> {
        if raw.len() < 2 {
            return Err(crate::Error::NoTrack);
        }
        let distance_m = geo::path_length_m(raw);
        let start = raw.iter().find_map(|p| p.t);
        let end = raw.iter().rev().find_map(|p| p.t);
        let duration_s = match (start, end) {
            (Some(s), Some(e)) if e > s => Some(e - s),
            _ => None,
        };
        let kind = kind.unwrap_or_else(|| match duration_s {
            Some(d) => ActivityKind::from_speed(distance_m / d as f64),
            None => ActivityKind::Run,
        });
        Ok(Self {
            id,
            name,
            kind,
            start,
            distance_m,
            duration_s,
            elev_gain_m: geo::elevation_gain_m(raw),
            points: geo::simplify(raw, 1.5),
        })
    }

    /// Average pace in seconds per kilometre.
    pub fn pace_s_per_km(&self) -> Option<f64> {
        let d = self.duration_s? as f64;
        (self.distance_m > 10.0).then(|| d / (self.distance_m / 1000.0))
    }

    /// Points with the first and last `radius_m` metres (straight-line from the
    /// start/end point) removed, so a home address isn't visible on screen.
    pub fn trimmed_points(&self, radius_m: f64) -> &[Point] {
        let pts = &self.points;
        if radius_m <= 0.0 || pts.len() < 2 {
            return pts;
        }
        let first = pts[0];
        let last = pts[pts.len() - 1];
        let lo = pts
            .iter()
            .position(|p| geo::haversine_m(&first, p) > radius_m)
            .unwrap_or(pts.len());
        let hi = pts
            .iter()
            .rposition(|p| geo::haversine_m(&last, p) > radius_m)
            .map_or(0, |i| i + 1);
        if hi > lo + 1 {
            &pts[lo..hi]
        } else {
            // Run is smaller than the privacy radius: keep it whole rather than blank.
            pts
        }
    }
}
