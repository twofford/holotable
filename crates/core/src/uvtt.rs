//! Universal VTT (`.uvtt` / `.dd2vtt`) map files.
//!
//! A UVTT file is one JSON object: the map's size, its walls, doors and lights,
//! and the map picture as a base64 string. All positions are in **grid squares**,
//! not pixels: `{"x": 3.5, "y": 2}` is halfway across the fourth column, on the
//! line between the second and third rows.
//!
//! [`UvttMap::parse`] reads the JSON into typed structs and checks the values that
//! serde cannot check on its own (for example, that a wall has at least two
//! points). It does not decode the picture, which can be tens of megabytes; call
//! [`UvttMap::image_bytes`] when the bytes are needed.
//!
//! Field names match the UVTT format exactly, so serde needs no renaming.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Where `cargo test` writes the generated TypeScript (see `protocol.rs`).
const EXPORT_DIR: &str = "../../../client/src/protocol/generated/";

/// A position in grid squares. `x` grows to the east and `y` to the south.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct GridPoint {
    pub x: f64,
    pub y: f64,
}

/// The map's size and grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct Resolution {
    /// Offset of the map's top-left corner, in squares. Almost always `0, 0`.
    pub map_origin: GridPoint,
    /// Width and height in squares. Can be fractional.
    pub map_size: GridPoint,
    /// How many image pixels make one grid square.
    pub pixels_per_grid: u32,
}

/// A door or window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct Portal {
    /// The middle of the doorway.
    pub position: GridPoint,
    /// The two ends of the doorway. A fixed-size array, so serde itself rejects
    /// any other number of points.
    pub bounds: [GridPoint; 2],
    /// Angle of the doorway in radians.
    pub rotation: f64,
    /// `true` for a closed door, which blocks sight. Open portals are often windows.
    pub closed: bool,
    /// `true` if the door stands on its own rather than in a wall.
    pub freestanding: bool,
}

/// A light source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct Light {
    pub position: GridPoint,
    /// Radius in squares.
    pub range: f64,
    /// Brightness; `1` is normal.
    pub intensity: f64,
    /// Hex colour without `#`: `RRGGBB` or `RRGGBBAA`.
    pub color: String,
    /// Whether walls cast shadows from this light.
    pub shadows: bool,
}

/// Global lighting settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct Environment {
    /// `true` if lighting is already painted into the picture.
    pub baked_lighting: bool,
    /// Hex colour of the light everywhere on the map, `RRGGBB` or `RRGGBBAA`.
    pub ambient_light: String,
}

/// A whole UVTT map file.
///
/// `#[serde(default)]` on a field means "use the type's default (an empty `Vec`,
/// or `None`) if the field is missing". Older files and some exporters leave
/// these fields out. Fields that are not listed here are ignored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = EXPORT_DIR)]
pub struct UvttMap {
    /// Format version, such as `0.2` or `0.3`.
    pub format: f64,
    pub resolution: Resolution,
    /// Walls. Each wall is a polyline: a list of at least two points.
    #[serde(default)]
    pub line_of_sight: Vec<Vec<GridPoint>>,
    /// Outlines of objects that block sight (furniture, pillars), as polylines.
    #[serde(default)]
    pub objects_line_of_sight: Vec<Vec<GridPoint>>,
    #[serde(default)]
    pub portals: Vec<Portal>,
    #[serde(default)]
    pub lights: Vec<Light>,
    #[serde(default)]
    pub environment: Option<Environment>,
    /// The map picture, base64-encoded. Usually PNG, sometimes WebP or JPEG.
    pub image: String,
}

/// The format of the embedded picture, read from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    WebP,
    Jpeg,
}

/// Why a UVTT file could not be used.
///
/// `thiserror` writes the `Display` and `Error` impls from the `#[error]` text.
#[derive(Debug, thiserror::Error)]
pub enum UvttError {
    /// Not valid JSON, or a field is missing or has the wrong type.
    #[error("not a valid UVTT file: {0}")]
    Json(#[from] serde_json::Error),
    /// Valid JSON with a value that makes no sense. `field` names where it is.
    #[error("invalid UVTT file: {field}: {reason}")]
    Invalid { field: String, reason: String },
    /// The `image` field is not valid base64.
    #[error("invalid UVTT image: not valid base64: {0}")]
    ImageBase64(#[from] base64::DecodeError),
    /// The picture is not PNG, WebP or JPEG.
    #[error("invalid UVTT image: not a PNG, WebP or JPEG")]
    ImageFormat,
}

impl UvttMap {
    /// Parses and checks a UVTT file.
    ///
    /// `?` returns early with the error if `from_str` fails. The `#[from]` on
    /// `UvttError::Json` lets `?` turn the `serde_json::Error` into a `UvttError`.
    pub fn parse(json: &str) -> Result<Self, UvttError> {
        let map: UvttMap = serde_json::from_str(json)?;
        map.validate()?;
        Ok(map)
    }

    /// The picture's expected size in pixels: map size × pixels per square, rounded.
    pub fn image_size_px(&self) -> (u32, u32) {
        let ppg = f64::from(self.resolution.pixels_per_grid);
        let size = self.resolution.map_size;
        // Validation guarantees positive, finite sizes, so these casts cannot wrap.
        ((size.x * ppg).round() as u32, (size.y * ppg).round() as u32)
    }

    /// Decodes the base64 picture into raw file bytes (still PNG, WebP or JPEG).
    pub fn image_bytes(&self) -> Result<Vec<u8>, UvttError> {
        // Some exporters wrap long base64 strings in newlines; strip whitespace first.
        let compact: String = self.image.chars().filter(|c| !c.is_whitespace()).collect();
        Ok(base64::engine::general_purpose::STANDARD.decode(compact)?)
    }

    /// Checks the values that the types alone cannot rule out.
    ///
    /// Takes `&self`: it only reads the map, so it borrows it instead of taking it.
    fn validate(&self) -> Result<(), UvttError> {
        let res = &self.resolution;
        if res.pixels_per_grid == 0 {
            return invalid("resolution.pixels_per_grid", "must be greater than 0");
        }
        check_point("resolution.map_origin", res.map_origin)?;
        check_point("resolution.map_size", res.map_size)?;
        if res.map_size.x <= 0.0 || res.map_size.y <= 0.0 {
            return invalid(
                "resolution.map_size",
                "width and height must be greater than 0",
            );
        }

        // `enumerate()` pairs each item with its index, for the error message.
        for (i, wall) in self.line_of_sight.iter().enumerate() {
            check_polyline(&format!("line_of_sight[{i}]"), wall)?;
        }
        for (i, outline) in self.objects_line_of_sight.iter().enumerate() {
            check_polyline(&format!("objects_line_of_sight[{i}]"), outline)?;
        }
        for (i, portal) in self.portals.iter().enumerate() {
            let field = format!("portals[{i}]");
            check_point(&field, portal.position)?;
            check_point(&field, portal.bounds[0])?;
            check_point(&field, portal.bounds[1])?;
            if !portal.rotation.is_finite() {
                return invalid(&field, "rotation must be a finite number");
            }
        }
        for (i, light) in self.lights.iter().enumerate() {
            let field = format!("lights[{i}]");
            check_point(&field, light.position)?;
            if !(light.range.is_finite() && light.range >= 0.0) {
                return invalid(&field, "range must be 0 or more");
            }
            if !(light.intensity.is_finite() && light.intensity >= 0.0) {
                return invalid(&field, "intensity must be 0 or more");
            }
            check_color(&format!("{field}.color"), &light.color)?;
        }
        if let Some(env) = &self.environment {
            check_color("environment.ambient_light", &env.ambient_light)?;
        }
        if self.image.trim().is_empty() {
            return invalid("image", "is empty");
        }
        Ok(())
    }
}

/// Reads the picture format and, for PNG, its pixel size, from the decoded bytes.
///
/// Returns `None` for the size of WebP and JPEG pictures: reading their headers
/// is more involved and not needed yet.
pub fn image_info(bytes: &[u8]) -> Result<(ImageFormat, Option<(u32, u32)>), UvttError> {
    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    if bytes.starts_with(PNG_SIGNATURE) {
        // A PNG starts with the signature (8 bytes), then the IHDR chunk: length
        // (4), type "IHDR" (4), width (4, big-endian), height (4, big-endian).
        // `get` returns `None` instead of panicking if the file is too short.
        let size = match (bytes.get(16..20), bytes.get(20..24)) {
            (Some(w), Some(h)) if bytes.get(12..16) == Some(b"IHDR") => Some((
                u32::from_be_bytes(w.try_into().expect("slice is 4 bytes")),
                u32::from_be_bytes(h.try_into().expect("slice is 4 bytes")),
            )),
            _ => None,
        };
        return Ok((ImageFormat::Png, size));
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok((ImageFormat::WebP, None));
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok((ImageFormat::Jpeg, None));
    }
    Err(UvttError::ImageFormat)
}

// --- Small helpers. Each returns `Err` on the first problem it finds. ---

/// Builds an `Invalid` error. Generic over `T` so callers can use it as their
/// return value whatever their `Ok` type is.
fn invalid<T>(field: &str, reason: &str) -> Result<T, UvttError> {
    Err(UvttError::Invalid {
        field: field.to_owned(),
        reason: reason.to_owned(),
    })
}

fn check_point(field: &str, p: GridPoint) -> Result<(), UvttError> {
    if p.x.is_finite() && p.y.is_finite() {
        Ok(())
    } else {
        invalid(field, "coordinates must be finite numbers")
    }
}

/// `&[GridPoint]` is a slice: a borrowed view of the points, whatever owns them.
fn check_polyline(field: &str, points: &[GridPoint]) -> Result<(), UvttError> {
    if points.len() < 2 {
        return invalid(field, "a line needs at least 2 points");
    }
    points.iter().try_for_each(|p| check_point(field, *p))
}

fn check_color(field: &str, color: &str) -> Result<(), UvttError> {
    let ok = matches!(color.len(), 6 | 8) && color.chars().all(|c| c.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        invalid(field, "must be RRGGBB or RRGGBBAA hex")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a fixture at compile time. `CARGO_MANIFEST_DIR` is this crate's
    /// folder, so the path works wherever `cargo test` is run from.
    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/",
                $name
            ))
        };
    }

    /// The numbers each fixture should produce, from the generator's own report.
    struct Expected {
        squares: (f64, f64),
        pixels: (u32, u32),
        walls: usize,
        objects: usize,
        portals: usize,
        closed_portals: usize,
        lights: usize,
    }

    fn check_fixture(json: &str, e: Expected) {
        let map = UvttMap::parse(json).expect("fixture parses");
        assert_eq!(map.format, 0.3);
        assert_eq!(map.resolution.pixels_per_grid, 70);
        assert_eq!(
            (map.resolution.map_size.x, map.resolution.map_size.y),
            e.squares
        );
        assert_eq!(map.image_size_px(), e.pixels);
        assert_eq!(map.line_of_sight.len(), e.walls);
        assert_eq!(map.objects_line_of_sight.len(), e.objects);
        assert_eq!(map.portals.len(), e.portals);
        assert_eq!(
            map.portals.iter().filter(|p| p.closed).count(),
            e.closed_portals
        );
        assert_eq!(map.lights.len(), e.lights);
        assert!(map.environment.is_some());

        // The embedded picture is a PNG of exactly the size the grid implies.
        let bytes = map.image_bytes().expect("image is valid base64");
        let (format, size) = image_info(&bytes).expect("image format is known");
        assert_eq!(format, ImageFormat::Png);
        assert_eq!(size, Some(e.pixels));
    }

    #[test]
    fn small_bodega() {
        check_fixture(
            fixture!("small-bodega.uvtt"),
            Expected {
                squares: (12.0, 10.0),
                pixels: (840, 700),
                walls: 28,
                objects: 1,
                portals: 4,
                closed_portals: 2,
                lights: 1,
            },
        );
    }

    #[test]
    fn dd2vtt_is_the_same_format() {
        let a = UvttMap::parse(fixture!("small-bodega.uvtt")).unwrap();
        let b = UvttMap::parse(fixture!("small-bodega.dd2vtt")).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn typical_safehouse() {
        check_fixture(
            fixture!("typical-safehouse.uvtt"),
            Expected {
                squares: (32.0, 22.0),
                pixels: (2240, 1540),
                walls: 169,
                objects: 6,
                portals: 10,
                closed_portals: 8,
                lights: 5,
            },
        );
    }

    #[test]
    fn large_arcology() {
        check_fixture(
            fixture!("large-arcology.uvtt"),
            Expected {
                squares: (80.0, 60.0),
                pixels: (5600, 4200),
                walls: 13,
                objects: 13,
                portals: 12,
                closed_portals: 12,
                lights: 9,
            },
        );
    }

    /// A minimal valid file. Tests below break one thing at a time.
    fn minimal() -> serde_json::Value {
        serde_json::json!({
            "format": 0.3,
            "resolution": {
                "map_origin": {"x": 0, "y": 0},
                "map_size": {"x": 2, "y": 1},
                "pixels_per_grid": 50
            },
            "image": "iVBORw0KGgo="
        })
    }

    fn parse_value(v: serde_json::Value) -> Result<UvttMap, UvttError> {
        UvttMap::parse(&v.to_string())
    }

    #[test]
    fn optional_fields_default_to_empty() {
        let map = parse_value(minimal()).unwrap();
        assert!(map.line_of_sight.is_empty());
        assert!(map.portals.is_empty());
        assert!(map.lights.is_empty());
        assert!(map.environment.is_none());
        assert_eq!(map.image_size_px(), (100, 50));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let mut v = minimal();
        v["software"] = "Dungeondraft".into();
        assert!(parse_value(v).is_ok());
    }

    #[test]
    fn missing_resolution_is_a_json_error() {
        let mut v = minimal();
        v.as_object_mut().unwrap().remove("resolution");
        assert!(matches!(parse_value(v), Err(UvttError::Json(_))));
    }

    #[test]
    fn zero_pixels_per_grid_is_invalid() {
        let mut v = minimal();
        v["resolution"]["pixels_per_grid"] = 0.into();
        let err = parse_value(v).unwrap_err();
        assert!(
            matches!(&err, UvttError::Invalid { field, .. } if field == "resolution.pixels_per_grid")
        );
    }

    #[test]
    fn negative_map_size_is_invalid() {
        let mut v = minimal();
        v["resolution"]["map_size"]["x"] = (-1).into();
        assert!(matches!(parse_value(v), Err(UvttError::Invalid { .. })));
    }

    #[test]
    fn one_point_wall_is_invalid() {
        let mut v = minimal();
        v["line_of_sight"] = serde_json::json!([[{"x": 0, "y": 0}]]);
        let err = parse_value(v).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid UVTT file: line_of_sight[0]: a line needs at least 2 points"
        );
    }

    #[test]
    fn portal_with_three_bounds_is_rejected() {
        let mut v = minimal();
        v["portals"] = serde_json::json!([{
            "position": {"x": 1, "y": 0},
            "bounds": [{"x": 0, "y": 0}, {"x": 1, "y": 0}, {"x": 2, "y": 0}],
            "rotation": 0, "closed": true, "freestanding": false
        }]);
        assert!(matches!(parse_value(v), Err(UvttError::Json(_))));
    }

    #[test]
    fn bad_light_color_is_invalid() {
        let mut v = minimal();
        v["lights"] = serde_json::json!([{
            "position": {"x": 1, "y": 1}, "range": 3, "intensity": 1,
            "color": "#ff0000", "shadows": true
        }]);
        assert!(
            matches!(parse_value(v), Err(UvttError::Invalid { field, .. }) if field == "lights[0].color")
        );
    }

    #[test]
    fn empty_image_is_invalid() {
        let mut v = minimal();
        v["image"] = "".into();
        assert!(
            matches!(parse_value(v), Err(UvttError::Invalid { field, .. }) if field == "image")
        );
    }

    #[test]
    fn bad_base64_fails_on_decode_not_parse() {
        let mut v = minimal();
        v["image"] = "not base64!!".into();
        let map = parse_value(v).expect("parse does not decode the image");
        assert!(matches!(map.image_bytes(), Err(UvttError::ImageBase64(_))));
    }

    #[test]
    fn image_info_recognises_formats() {
        assert_eq!(
            image_info(b"RIFF\0\0\0\0WEBPVP8 ").unwrap(),
            (ImageFormat::WebP, None)
        );
        assert_eq!(
            image_info(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap(),
            (ImageFormat::Jpeg, None)
        );
        assert!(matches!(image_info(b"GIF89a"), Err(UvttError::ImageFormat)));
        // A truncated PNG is still a PNG, but its size is unknown.
        assert_eq!(
            image_info(b"\x89PNG\r\n\x1a\n").unwrap(),
            (ImageFormat::Png, None)
        );
    }
}
