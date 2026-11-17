//! Line of sight (milestone M5): what can be seen from a point.
//!
//! Everything here is in image pixels and has no I/O, so the server (native)
//! and the browser (through WebAssembly, `crates/wasm`) run the very same code
//! and can never disagree about what a token sees (spec VIS-001).
//!
//! The visibility polygon uses the classic ray-casting method: walls only
//! change direction at their end points, so it's enough to cast a ray towards
//! every end point, plus one just to each side of it (to see past a corner),
//! and keep the nearest wall each ray hits. Joining the hits in angle order
//! gives the polygon.

use crate::geometry::{Point, Segment, crosses_any};

/// How far to each side of a corner the extra rays go, in radians. Small
/// enough that the gap is under a pixel on any map we accept.
const CORNER_EPS: f64 = 1e-5;
/// With a vision range, this many extra rays round off the circle's edge.
const RANGE_RAYS: usize = 72;
/// Widens each wall's arc a little, so rounding never drops a ray that
/// touches its very end. `ray_hit` still decides exactly.
const ARC_SLACK: f64 = 1e-7;

/// Where a ray from `o` along `d` first meets `s`, as a multiple of `d`.
///
/// Solves `o + t·d = a + u·(b − a)` with two cross products: `t` is how far
/// along the ray, `u` how far along the segment (0 to 1 means on it).
fn ray_hit(o: Point, d: (f64, f64), s: Segment) -> Option<f64> {
    let (ex, ey) = (s.b.x - s.a.x, s.b.y - s.a.y);
    let den = d.0 * ey - d.1 * ex;
    if den.abs() < 1e-12 {
        return None; // Parallel: never meets (or slides along it).
    }
    let (ax, ay) = (s.a.x - o.x, s.a.y - o.y);
    let t = (ax * ey - ay * ex) / den;
    let u = (ax * d.1 - ay * d.0) / den;
    (t >= 0.0 && (-1e-9..=1.0 + 1e-9).contains(&u)).then_some(t)
}

/// Shortest distance from `p` to segment `s`. Used to skip walls out of range.
fn distance_to_segment(p: Point, s: Segment) -> f64 {
    let (ex, ey) = (s.b.x - s.a.x, s.b.y - s.a.y);
    let len2 = ex * ex + ey * ey;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.x - s.a.x) * ex + (p.y - s.a.y) * ey) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (s.a.x + t * ex, s.a.y + t * ey);
    (p.x - cx).hypot(p.y - cy)
}

/// The area visible from `origin`, as a polygon (points in angle order).
///
/// `walls` block sight. The map's edges always do. `range` (pixels) limits
/// how far the eye reaches; `None` is unlimited. An origin off the map sees
/// nothing.
pub fn visibility_polygon(
    origin: Point,
    walls: &[Segment],
    width: f64,
    height: f64,
    range: Option<f64>,
) -> Vec<Point> {
    if !(0.0..=width).contains(&origin.x) || !(0.0..=height).contains(&origin.y) {
        return Vec::new();
    }
    let corners = [
        Point::new(0.0, 0.0),
        Point::new(width, 0.0),
        Point::new(width, height),
        Point::new(0.0, height),
    ];
    // Only walls that some ray could reach matter.
    let mut segs: Vec<Segment> = walls
        .iter()
        .copied()
        .filter(|s| range.is_none_or(|r| distance_to_segment(origin, *s) <= r))
        .collect();
    for i in 0..4 {
        segs.push(Segment::new(corners[i], corners[(i + 1) % 4]));
    }

    let mut angles: Vec<f64> = Vec::with_capacity(segs.len() * 6 + RANGE_RAYS);
    for s in &segs {
        for p in [s.a, s.b] {
            let a = (p.y - origin.y).atan2(p.x - origin.x);
            angles.extend([a - CORNER_EPS, a, a + CORNER_EPS]);
        }
    }
    if range.is_some() {
        let step = std::f64::consts::TAU / RANGE_RAYS as f64;
        angles.extend((0..RANGE_RAYS).map(|i| -std::f64::consts::PI + i as f64 * step));
    }
    // `total_cmp` sorts floats without the NaN worry that `<` has.
    angles.sort_by(f64::total_cmp);
    angles.dedup_by(|a, b| (*a - *b).abs() < 1e-12);

    // For each ray, the distance to the nearest wall it hits. Rather than
    // test every ray against every wall, each wall is tested only against the
    // rays inside the angle it covers as seen from the origin: a short or
    // distant wall covers a sliver of the circle and meets only a few rays.
    let dirs: Vec<(f64, f64)> = angles.iter().map(|a| (a.cos(), a.sin())).collect();
    let mut nearest = vec![range.unwrap_or(f64::INFINITY); angles.len()];
    let pi = std::f64::consts::PI;
    for s in &segs {
        let a1 = (s.a.y - origin.y).atan2(s.a.x - origin.x);
        let a2 = (s.b.y - origin.y).atan2(s.b.x - origin.x);
        // The arc between the two ends, going the short way round.
        let mut d = a2 - a1;
        if d > pi {
            d -= std::f64::consts::TAU;
        } else if d < -pi {
            d += std::f64::consts::TAU;
        }
        let lo = a1.min(a1 + d) - ARC_SLACK;
        let hi = a1.max(a1 + d) + ARC_SLACK;
        let mut test = |from: f64, to: f64| {
            // `partition_point` is a binary search on the sorted angles.
            let first = angles.partition_point(|&x| x < from);
            let last = angles.partition_point(|&x| x <= to);
            for i in first..last {
                if let Some(t) = ray_hit(origin, dirs[i], *s)
                    && t < nearest[i]
                {
                    nearest[i] = t;
                }
            }
        };
        // An arc that crosses the ±π line is tested as two pieces.
        test(lo, hi.min(pi));
        if hi > pi {
            test(-pi, hi - std::f64::consts::TAU);
        }
        if lo < -pi {
            test(lo + std::f64::consts::TAU, pi);
        }
    }

    dirs.iter()
        .zip(nearest)
        .map(|(d, t)| {
            let t = if t.is_finite() { t } else { 0.0 }; // Can't happen inside the map's edges.
            Point::new(origin.x + t * d.0, origin.y + t * d.1)
        })
        .collect()
}

/// Can `from` see the point `to`? Nothing in the way, and within range.
pub fn point_visible(from: Point, to: Point, walls: &[Segment], range: Option<f64>) -> bool {
    if range.is_some_and(|r| (to.x - from.x).hypot(to.y - from.y) > r) {
        return false;
    }
    !crosses_any(Segment::new(from, to), walls)
}

/// Can an eye at `from` see any part of a square token centred at `center`
/// with half-width `half`? Checks the centre and four points near the corners,
/// so a token peeking round a corner counts as seen.
pub fn square_visible(
    from: Point,
    center: Point,
    half: f64,
    walls: &[Segment],
    range: Option<f64>,
) -> bool {
    let k = half * 0.7;
    [
        center,
        Point::new(center.x - k, center.y - k),
        Point::new(center.x + k, center.y - k),
        Point::new(center.x - k, center.y + k),
        Point::new(center.x + k, center.y + k),
    ]
    .into_iter()
    .any(|p| point_visible(from, p, walls, range))
}

/// The grid of cells that "explored" fog is kept in (VIS-005): at most 512
/// cells along the longer side, whatever the picture's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExploredLayout {
    /// Side of one cell, in image pixels.
    pub cell: f64,
    pub cols: u32,
    pub rows: u32,
}

pub const EXPLORED_MAX_CELLS: u32 = 512;

impl ExploredLayout {
    pub fn for_map(width: f64, height: f64) -> Self {
        let cell = (width.max(height) / f64::from(EXPLORED_MAX_CELLS)).max(1.0);
        // `as u32` on these small, positive values is exact.
        let cols = (width / cell).ceil().max(1.0) as u32;
        let rows = (height / cell).ceil().max(1.0) as u32;
        Self { cell, cols, rows }
    }

    /// Bytes needed for one bit per cell.
    pub fn byte_len(&self) -> usize {
        (self.cols as usize * self.rows as usize).div_ceil(8)
    }
}

/// Sets the bit of every cell whose centre is inside `polygon`.
///
/// A scanline fill: for each row of cells, find where the polygon's edges cross
/// the row's middle line, sort those crossings, and fill between each pair.
/// Cell `i` (row-major) is bit `i % 8` of byte `i / 8`. Returns whether any
/// bit was newly set.
pub fn rasterize(polygon: &[Point], layout: ExploredLayout, bits: &mut [u8]) -> bool {
    let n = polygon.len();
    if n < 3 || bits.len() < layout.byte_len() {
        return false;
    }
    let mut changed = false;
    let mut xs: Vec<f64> = Vec::new();
    for row in 0..layout.rows {
        let y = (f64::from(row) + 0.5) * layout.cell;
        xs.clear();
        for i in 0..n {
            let (p, q) = (polygon[i], polygon[(i + 1) % n]);
            if (p.y > y) != (q.y > y) {
                xs.push(p.x + (y - p.y) * (q.x - p.x) / (q.y - p.y));
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.chunks_exact(2) {
            // Cells whose centre ((c + 0.5) · cell) lies in [x0, x1).
            let first = (pair[0] / layout.cell - 0.5).ceil().max(0.0);
            let last = (pair[1] / layout.cell - 0.5)
                .ceil()
                .min(f64::from(layout.cols));
            let (first, last) = (first as u32, last as u32);
            for col in first..last {
                let i = row as usize * layout.cols as usize + col as usize;
                let mask = 1u8 << (i % 8);
                if bits[i / 8] & mask == 0 {
                    bits[i / 8] |= mask;
                    changed = true;
                }
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(ax: f64, ay: f64, bx: f64, by: f64) -> Segment {
        Segment::new(Point::new(ax, ay), Point::new(bx, by))
    }

    /// Twice the signed area (shoelace formula); its absolute value halved is the area.
    fn area(poly: &[Point]) -> f64 {
        let n = poly.len();
        (0..n)
            .map(|i| poly[i].x * poly[(i + 1) % n].y - poly[(i + 1) % n].x * poly[i].y)
            .sum::<f64>()
            .abs()
            / 2.0
    }

    #[test]
    fn an_empty_room_is_all_visible() {
        let poly = visibility_polygon(Point::new(30.0, 40.0), &[], 100.0, 80.0, None);
        assert!((area(&poly) - 8000.0).abs() < 1.0, "area {}", area(&poly));
    }

    #[test]
    fn a_wall_hides_what_is_behind_it() {
        // A wall across the whole map at x = 50: the eye at x = 25 sees half.
        let walls = [seg(50.0, 0.0, 50.0, 100.0)];
        let poly = visibility_polygon(Point::new(25.0, 50.0), &walls, 100.0, 100.0, None);
        assert!((area(&poly) - 5000.0).abs() < 1.0, "area {}", area(&poly));
    }

    #[test]
    fn a_short_wall_casts_a_shadow() {
        // A 20 px wall straight ahead casts a shadow that widens behind it.
        let walls = [seg(50.0, 40.0, 50.0, 60.0)];
        let poly = visibility_polygon(Point::new(25.0, 50.0), &walls, 100.0, 100.0, None);
        // Shadow: trapezoid from x=50 (height 20) to x=100 (height 60): 50·(20+60)/2.
        assert!(
            (area(&poly) - (10_000.0 - 2000.0)).abs() < 1.0,
            "area {}",
            area(&poly)
        );
    }

    #[test]
    fn range_limits_sight_to_a_circle() {
        let poly = visibility_polygon(Point::new(500.0, 500.0), &[], 1000.0, 1000.0, Some(100.0));
        let circle = std::f64::consts::PI * 100.0 * 100.0;
        assert!(
            (area(&poly) - circle).abs() / circle < 0.01,
            "area {}",
            area(&poly)
        );
    }

    #[test]
    fn off_the_map_sees_nothing() {
        assert!(visibility_polygon(Point::new(-1.0, 5.0), &[], 10.0, 10.0, None).is_empty());
    }

    #[test]
    fn points_and_tokens_behind_walls() {
        let walls = [seg(50.0, 0.0, 50.0, 60.0)];
        let eye = Point::new(25.0, 25.0);
        assert!(!point_visible(eye, Point::new(75.0, 25.0), &walls, None));
        assert!(point_visible(eye, Point::new(25.0, 90.0), &walls, None));
        assert!(!point_visible(
            eye,
            Point::new(25.0, 90.0),
            &walls,
            Some(50.0)
        ));
        // A token whose centre is hidden but whose lower part pokes out below the wall's end.
        assert!(square_visible(
            eye,
            Point::new(75.0, 55.0),
            25.0,
            &walls,
            None
        ));
        assert!(!square_visible(
            eye,
            Point::new(75.0, 20.0),
            10.0,
            &walls,
            None
        ));
    }

    #[test]
    fn rasterize_fills_cells_inside() {
        let layout = ExploredLayout::for_map(1000.0, 500.0);
        assert_eq!((layout.cols, layout.rows), (512, 256));
        let small = ExploredLayout {
            cell: 10.0,
            cols: 10,
            rows: 5,
        };
        let mut bits = vec![0u8; small.byte_len()];
        // The left half of the map: columns 0–4 of every row.
        let half = [
            Point::new(0.0, 0.0),
            Point::new(50.0, 0.0),
            Point::new(50.0, 50.0),
            Point::new(0.0, 50.0),
        ];
        assert!(rasterize(&half, small, &mut bits));
        let set = |i: usize| bits[i / 8] & (1 << (i % 8)) != 0;
        let count = (0..50).filter(|&i| set(i)).count();
        assert_eq!(count, 25);
        assert!(set(0) && set(4) && !set(5) && set(14) && !set(15));
        // Filling the same area again changes nothing.
        assert!(!rasterize(&half, small, &mut bits));
    }

    /// The pruned search must find exactly what testing every wall finds.
    #[test]
    fn matches_testing_every_wall() {
        let mut seed: u64 = 7;
        let mut rnd = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let walls: Vec<Segment> = (0..200)
            .map(|_| {
                let (x, y) = (rnd() * 1000.0, rnd() * 800.0);
                seg(x, y, x + rnd() * 200.0 - 100.0, y + rnd() * 200.0 - 100.0)
            })
            .collect();
        for _ in 0..20 {
            let o = Point::new(rnd() * 1000.0, rnd() * 800.0);
            let poly = visibility_polygon(o, &walls, 1000.0, 800.0, None);
            for p in &poly {
                let (dx, dy) = (p.x - o.x, p.y - o.y);
                let len = dx.hypot(dy);
                if len < 1e-9 {
                    continue;
                }
                let d = (dx / len, dy / len);
                let edges = [
                    seg(0.0, 0.0, 1000.0, 0.0),
                    seg(1000.0, 0.0, 1000.0, 800.0),
                    seg(1000.0, 800.0, 0.0, 800.0),
                    seg(0.0, 800.0, 0.0, 0.0),
                ];
                let brute = walls
                    .iter()
                    .chain(&edges)
                    .filter_map(|s| ray_hit(o, d, *s))
                    .fold(f64::INFINITY, f64::min);
                assert!(
                    (brute - len).abs() < 1e-6,
                    "ray hit {len}, every wall says {brute}"
                );
            }
        }
    }
}
