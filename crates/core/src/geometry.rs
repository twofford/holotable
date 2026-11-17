//! 2D geometry: points, segments, and whether two segments cross.
//!
//! M4 uses this to stop tokens moving through walls; M5 builds line of sight
//! on it. Pure maths with no dependencies, so it compiles to WASM unchanged.

/// A point. Units depend on the caller; scene rules use image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// A straight line between two points: one piece of a wall, or a door.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    pub a: Point,
    pub b: Point,
}

impl Segment {
    pub const fn new(a: Point, b: Point) -> Self {
        Self { a, b }
    }
}

/// Which way you turn going from `a` to `b` to `c`: positive for left
/// (counter-clockwise, with y pointing down that's clockwise on screen),
/// negative for right, zero for straight on. It's the 2D cross product of
/// (b - a) and (c - a).
fn turn(a: Point, b: Point, c: Point) -> f64 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Is `p` within the bounding box of `s`? Only meaningful when `p` is already
/// known to lie on the line through `s`.
fn within_box(s: Segment, p: Point) -> bool {
    p.x >= s.a.x.min(s.b.x) - EPS
        && p.x <= s.a.x.max(s.b.x) + EPS
        && p.y >= s.a.y.min(s.b.y) - EPS
        && p.y <= s.a.y.max(s.b.y) + EPS
}

/// Tolerance for "exactly on the line". Coordinates are pixels, so this is
/// far below anything visible.
const EPS: f64 = 1e-9;

fn sign(v: f64) -> i8 {
    if v > EPS {
        1
    } else if v < -EPS {
        -1
    } else {
        0
    }
}

/// Do segments `p` and `q` touch or cross?
///
/// The classic test: they cross if `q`'s ends are on opposite sides of `p`,
/// and `p`'s ends are on opposite sides of `q`. If an end lies exactly on the
/// other segment (a turn of zero), it counts as touching when it's within that
/// segment's extent. Touching counts, so a move can't slip through the exact
/// point where two walls meet.
pub fn segments_intersect(p: Segment, q: Segment) -> bool {
    let d1 = sign(turn(q.a, q.b, p.a));
    let d2 = sign(turn(q.a, q.b, p.b));
    let d3 = sign(turn(p.a, p.b, q.a));
    let d4 = sign(turn(p.a, p.b, q.b));

    if d1 * d2 < 0 && d3 * d4 < 0 {
        return true; // A proper crossing.
    }
    (d1 == 0 && within_box(q, p.a))
        || (d2 == 0 && within_box(q, p.b))
        || (d3 == 0 && within_box(p, q.a))
        || (d4 == 0 && within_box(p, q.b))
}

/// Does the straight path `path` cross any of `walls`?
pub fn crosses_any(path: Segment, walls: &[Segment]) -> bool {
    walls.iter().any(|w| segments_intersect(path, *w))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(ax: f64, ay: f64, bx: f64, by: f64) -> Segment {
        Segment::new(Point::new(ax, ay), Point::new(bx, by))
    }

    #[test]
    fn crossing_segments() {
        assert!(segments_intersect(
            seg(0., 0., 10., 10.),
            seg(0., 10., 10., 0.)
        ));
    }

    #[test]
    fn parallel_and_apart() {
        assert!(!segments_intersect(
            seg(0., 0., 10., 0.),
            seg(0., 1., 10., 1.)
        ));
    }

    #[test]
    fn would_cross_if_longer() {
        assert!(!segments_intersect(
            seg(0., 0., 4., 4.),
            seg(0., 10., 10., 0.)
        ));
    }

    #[test]
    fn touching_at_an_end_counts() {
        assert!(segments_intersect(
            seg(0., 0., 5., 5.),
            seg(5., 5., 10., 0.)
        ));
        assert!(segments_intersect(
            seg(0., 5., 10., 5.),
            seg(5., 0., 5., 5.)
        ));
    }

    #[test]
    fn collinear_overlap_counts_and_gap_does_not() {
        assert!(segments_intersect(
            seg(0., 0., 10., 0.),
            seg(5., 0., 15., 0.)
        ));
        assert!(!segments_intersect(
            seg(0., 0., 4., 0.),
            seg(5., 0., 15., 0.)
        ));
    }

    #[test]
    fn path_against_walls() {
        let walls = [seg(5., -10., 5., 10.)];
        assert!(crosses_any(seg(0., 0., 10., 0.), &walls));
        assert!(!crosses_any(seg(0., 0., 4., 0.), &walls));
    }
}
