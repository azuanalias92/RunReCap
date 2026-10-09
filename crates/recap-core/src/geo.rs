//! Small geometry helpers: distances, elevation gain, simplification, projection.

use crate::Point;

const EARTH_RADIUS_M: f64 = 6_371_008.8;

pub fn haversine_m(a: &Point, b: &Point) -> f64 {
    let (la1, la2) = (a.lat.to_radians(), b.lat.to_radians());
    let dlat = la2 - la1;
    let dlon = (b.lon - a.lon).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

pub fn path_length_m(points: &[Point]) -> f64 {
    points.windows(2).map(|w| haversine_m(&w[0], &w[1])).sum()
}

/// Total climb, ignoring changes under 3 m to filter GPS altitude noise.
pub fn elevation_gain_m(points: &[Point]) -> Option<f64> {
    let mut eles = points.iter().filter_map(|p| p.ele.map(f64::from));
    let mut anchor = eles.next()?;
    let mut gain = 0.0;
    for e in eles {
        if e - anchor >= 3.0 {
            gain += e - anchor;
            anchor = e;
        } else if anchor - e >= 3.0 {
            anchor = e;
        }
    }
    Some(gain)
}

/// Web Mercator projection to unitless (x, y) with y pointing down.
pub fn mercator(p: &Point) -> (f64, f64) {
    let x = p.lon.to_radians();
    let lat = p.lat.clamp(-85.0511, 85.0511).to_radians();
    let y = (std::f64::consts::FRAC_PI_4 + lat / 2.0).tan().ln();
    (x, -y)
}

/// Ramer–Douglas–Peucker simplification with a tolerance in metres.
pub fn simplify(points: &[Point], tolerance_m: f64) -> Vec<Point> {
    if points.len() < 3 {
        return points.to_vec();
    }
    // Local equirectangular frame in metres around the first point.
    let lat0 = points[0].lat.to_radians();
    let xy: Vec<(f64, f64)> = points
        .iter()
        .map(|p| {
            (
                p.lon.to_radians() * lat0.cos() * EARTH_RADIUS_M,
                p.lat.to_radians() * EARTH_RADIUS_M,
            )
        })
        .collect();
    let mut keep = vec![false; points.len()];
    keep[0] = true;
    keep[points.len() - 1] = true;
    let mut stack = vec![(0usize, points.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (mut idx, mut max) = (a, 0.0);
        for i in a + 1..b {
            let d = seg_dist(xy[i], xy[a], xy[b]);
            if d > max {
                max = d;
                idx = i;
            }
        }
        if max > tolerance_m {
            keep[idx] = true;
            stack.push((a, idx));
            stack.push((idx, b));
        }
    }
    points
        .iter()
        .zip(keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

fn seg_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    ((p.0 - a.0 - t * dx).powi(2) + (p.1 - a.1 - t * dy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(lat: f64, lon: f64) -> Point {
        Point {
            lat,
            lon,
            ele: None,
            t: None,
            hr: None,
        }
    }

    #[test]
    fn haversine_one_degree_lat() {
        let d = haversine_m(&pt(0.0, 0.0), &pt(1.0, 0.0));
        assert!((d - 111_195.0).abs() < 50.0, "{d}");
    }

    #[test]
    fn simplify_drops_collinear_points() {
        let pts: Vec<_> = (0..100).map(|i| pt(3.0 + i as f64 * 1e-5, 101.0)).collect();
        assert_eq!(simplify(&pts, 1.0).len(), 2);
    }

    #[test]
    fn elevation_gain_ignores_noise() {
        let mut pts = vec![];
        for (i, e) in [10.0, 11.0, 10.0, 15.0, 14.0, 20.0].iter().enumerate() {
            let mut p = pt(0.0, i as f64);
            p.ele = Some(*e);
            pts.push(p);
        }
        assert_eq!(elevation_gain_m(&pts), Some(10.0));
    }
}
