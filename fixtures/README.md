# UVTT test fixtures

Generated with the uvtt-map-generator tool from the YAML in `src/`. The YAML is the source of truth: edit it and recompile, don't edit the `.uvtt` files.

| File | Map | Size | Pixels | What it exercises |
| --- | --- | --- | --- | --- |
| `small-bodega.uvtt` | Corner bodega | 12 × 10 | 840 × 700 | Smallest case: 3 rooms, a glass storefront (open portals), open ground, 1 sight-blocking object, 1 light |
| `small-bodega.dd2vtt` | (same file) | | | The `.dd2vtt` extension: same JSON, different name |
| `typical-safehouse.uvtt` | Apartment safehouse | 32 × 22 | 2240 × 1540 | A normal session map: 8 rooms, closed and open doors, glass wall, openings, 2 secret doors, 6 objects, 5 coloured lights |
| `large-arcology.uvtt` | Undercity plaza | 80 × 60 | 5600 × 4200 | Size limits: 23.5 megapixels is over Safari's 16.7-megapixel canvas limit and over 4096 px on a side, so it must be tiled (AST-005, NFR-013). Also curved walls (irregular, circle, oval rooms), a hole in a floor, a diagonal corridor and wide organic passages |

All three are UVTT `format` 0.3 with `pixels_per_grid` 70 and `map_origin` 0,0.

Note: the large map's PNG is only about 0.5 MB because the art is flat colour. Painted maps of the same pixel size are often 10–30 MB, but the iPad memory limit depends on pixel size (5600 × 4200 × 4 bytes ≈ 94 MB decoded), not on file size, so this file still tests the right thing.

To regenerate: `python3 <uvtt-map-generator>/scripts/uvtt_from_yaml.py src/<name>.yml --output <name>.uvtt`.
