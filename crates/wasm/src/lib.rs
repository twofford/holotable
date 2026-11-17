//! The browser's view of `holotable-core`'s line of sight (spec VIS-001).
//!
//! `#[wasm_bindgen]` makes a Rust type or function callable from JavaScript.
//! The `wasm-bindgen` tool then writes the JavaScript glue and TypeScript
//! types (`client/src/wasm/holotable_wasm.{js,d.ts}`). Numbers cross the
//! boundary as JavaScript numbers, `&[f64]` arrives from a `Float64Array`,
//! `Vec<f64>` goes back as one, and `&mut [u8]` writes straight into a
//! `Uint8Array` the page owns.
//!
//! The rules themselves aren't here: this crate only converts arguments and
//! calls `holotable_core`, so the browser runs exactly the server's code.

use holotable_core::{
    geometry::Point,
    scene::{DoorState, SceneRules},
    uvtt::UvttMap,
    vision::{self, ExploredLayout},
};
use wasm_bindgen::prelude::*;

/// One scene's walls and doors, ready to answer "what can be seen from here?".
#[wasm_bindgen]
pub struct SceneSight {
    rules: SceneRules,
}

#[wasm_bindgen]
impl SceneSight {
    /// Builds the scene's walls from its UVTT map (as JSON) and grid, the same
    /// way the server does. Throws (a JavaScript exception) if the JSON is bad.
    #[wasm_bindgen(constructor)]
    pub fn new(
        map_json: &str,
        width: u32,
        height: u32,
        pixels_per_grid: f64,
        offset_x: f64,
        offset_y: f64,
    ) -> Result<SceneSight, JsError> {
        let map: UvttMap = serde_json::from_str(map_json)?;
        Ok(SceneSight {
            rules: SceneRules::new(&map, width, height, pixels_per_grid, offset_x, offset_y),
        })
    }

    #[wasm_bindgen(js_name = setDoor)]
    pub fn set_door(&mut self, door: u32, open: bool, locked: bool) {
        self.rules.set_door(door, DoorState { open, locked });
    }

    /// Door lines in image pixels, four numbers each: x1, y1, x2, y2.
    #[wasm_bindgen(js_name = doorLines)]
    pub fn door_lines(&self) -> Vec<f64> {
        self.rules
            .door_lines()
            .iter()
            .flat_map(|s| [s.a.x, s.a.y, s.b.x, s.b.y])
            .collect()
    }

    /// The area visible from (x, y), as x, y pairs. `range` is in pixels;
    /// 0 or less means unlimited.
    pub fn polygon(&self, x: f64, y: f64, range: f64) -> Vec<f64> {
        let range = (range > 0.0).then_some(range);
        vision::visibility_polygon(
            Point::new(x, y),
            self.rules.sight(),
            self.rules.width,
            self.rules.height,
            range,
        )
        .into_iter()
        .flat_map(|p| [p.x, p.y])
        .collect()
    }

    /// Can an eye at (x, y) see any part of a square token centred at
    /// (cx, cy) with half-width `half`? Same test as the server's.
    #[wasm_bindgen(js_name = seesSquare)]
    pub fn sees_square(&self, x: f64, y: f64, range: f64, cx: f64, cy: f64, half: f64) -> bool {
        vision::square_visible(
            Point::new(x, y),
            Point::new(cx, cy),
            half,
            self.rules.sight(),
            (range > 0.0).then_some(range),
        )
    }
}

/// Marks the cells inside `polygon` (x, y pairs) in `bits`, the explored fog
/// laid out as `vision::ExploredLayout`. Returns whether anything changed.
#[wasm_bindgen]
pub fn rasterize(polygon: &[f64], cell: f64, cols: u32, rows: u32, bits: &mut [u8]) -> bool {
    let points: Vec<Point> = polygon
        .chunks_exact(2)
        .map(|p| Point::new(p[0], p[1]))
        .collect();
    vision::rasterize(&points, ExploredLayout { cell, cols, rows }, bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sight_matches_the_core() {
        let json = include_str!("../../../fixtures/small-bodega.uvtt");
        let sight = SceneSight::new(json, 840, 700, 70.0, 0.0, 0.0).unwrap();
        let poly = sight.polygon(245.0, 315.0, 0.0);
        assert!(poly.len() >= 6 && poly.len().is_multiple_of(2));
        assert_eq!(sight.door_lines().len(), 4 * 4);
        // The back room (x > 560) is behind the wall; the shop isn't.
        assert!(!sight.sees_square(245.0, 315.0, 0.0, 665.0, 175.0, 35.0));
        assert!(sight.sees_square(245.0, 315.0, 0.0, 385.0, 315.0, 35.0));
    }
}
