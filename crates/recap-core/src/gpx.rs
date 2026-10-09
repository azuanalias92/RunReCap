//! Minimal streaming GPX reader. Reads track points (`<trkpt>`) with
//! elevation, time and Garmin/Strava heart-rate extensions, plus the
//! track name and type. Waypoints and routes are ignored.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::{Error, Point, Result};

#[derive(Debug, Default)]
pub struct ParsedGpx {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub points: Vec<Point>,
}

pub fn parse(bytes: &[u8]) -> Result<ParsedGpx> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut out = ParsedGpx::default();
    let mut path: Vec<String> = Vec::new();
    let mut cur: Option<Point> = None;
    let mut buf = Vec::new();

    loop {
        let ev = reader
            .read_event_into(&mut buf)
            .map_err(|e| Error::Gpx(format!("at byte {}: {e}", reader.buffer_position())))?;
        match ev {
            Event::Start(e) => {
                let name = local(&e);
                if name == "trkpt" {
                    cur = start_point(&e)?;
                }
                path.push(name);
            }
            Event::Empty(e) => {
                if local(&e) == "trkpt" {
                    if let Some(p) = start_point(&e)? {
                        out.points.push(p);
                    }
                }
            }
            Event::End(_) => {
                if path.pop().as_deref() == Some("trkpt") {
                    if let Some(p) = cur.take() {
                        out.points.push(p);
                    }
                }
            }
            Event::Text(t) => {
                let text = t.unescape().map_err(|e| Error::Gpx(e.to_string()))?;
                let text = text.trim();
                let leaf = path.last().map(String::as_str);
                let parent = path.len().checked_sub(2).map(|i| path[i].as_str());
                match (parent, leaf, cur.as_mut()) {
                    (_, Some("ele"), Some(p)) => p.ele = text.parse().ok(),
                    (_, Some("time"), Some(p)) => p.t = parse_time(text),
                    (_, Some("hr"), Some(p)) => p.hr = text.parse().ok(),
                    (Some("trk"), Some("name"), None) if out.name.is_none() => {
                        out.name = Some(text.to_owned())
                    }
                    (Some("metadata"), Some("name"), None) if out.name.is_none() => {
                        out.name = Some(text.to_owned())
                    }
                    (Some("trk"), Some("type"), None) => out.kind = Some(text.to_owned()),
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    if out.points.len() < 2 {
        return Err(Error::NoTrack);
    }
    Ok(out)
}

fn local(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.local_name().as_ref()).into_owned()
}

fn start_point(e: &BytesStart) -> Result<Option<Point>> {
    let mut lat = None;
    let mut lon = None;
    for a in e.attributes().flatten() {
        let v = a.unescape_value().map_err(|e| Error::Gpx(e.to_string()))?;
        match a.key.local_name().as_ref() {
            b"lat" => lat = v.trim().parse::<f64>().ok(),
            b"lon" => lon = v.trim().parse::<f64>().ok(),
            _ => {}
        }
    }
    Ok(match (lat, lon) {
        (Some(lat), Some(lon)) if lat.abs() <= 90.0 && lon.abs() <= 180.0 => Some(Point {
            lat,
            lon,
            ele: None,
            t: None,
            hr: None,
        }),
        _ => None,
    })
}

fn parse_time(s: &str) -> Option<i64> {
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(|t| t.unix_timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx creator="StravaGPX" version="1.1" xmlns="http://www.topografix.com/GPX/1/1"
     xmlns:gpxtpx="http://www.garmin.com/xmlschemas/TrackPointExtension/v1">
 <metadata><time>2026-10-03T06:12:00Z</time></metadata>
 <trk>
  <name>Morning Run</name>
  <type>9</type>
  <trkseg>
   <trkpt lat="3.1400" lon="101.6900"><ele>50.2</ele><time>2026-10-03T06:12:00Z</time>
    <extensions><gpxtpx:TrackPointExtension><gpxtpx:hr>140</gpxtpx:hr></gpxtpx:TrackPointExtension></extensions>
   </trkpt>
   <trkpt lat="3.1410" lon="101.6910"><ele>52.0</ele><time>2026-10-03T06:13:00Z</time></trkpt>
  </trkseg>
 </trk>
</gpx>"#;

    #[test]
    fn parses_strava_style_gpx() {
        let g = parse(SAMPLE.as_bytes()).unwrap();
        assert_eq!(g.name.as_deref(), Some("Morning Run"));
        assert_eq!(g.kind.as_deref(), Some("9"));
        assert_eq!(g.points.len(), 2);
        assert_eq!(g.points[0].hr, Some(140));
        assert_eq!(g.points[1].t.unwrap() - g.points[0].t.unwrap(), 60);
    }

    #[test]
    fn empty_track_is_error() {
        assert!(matches!(parse(b"<gpx></gpx>"), Err(Error::NoTrack)));
    }
}
