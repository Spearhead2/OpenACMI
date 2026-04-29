# Changelog

All notable changes to OpenACMI will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---
<!--
## [Unreleased]

### Added

### Changed

### Deprecated

### Removed

### Fixed

### Security
---


## [v0.1.0] — YYYY-MM-DD

## Initial Source Release 🎉

### Added
- Initial ACMI 2.x parser (`ACMIParser`) with full object property support and delta-state carry-forward
- `cBaseObject` / `cDynamicObject` two-tier object system — static objects carry no history overhead
- Structure-of-Arrays packed history (`snap_time`, `snap_lon`, `snap_lat`, etc.) replacing per-frame Dictionary allocation
- `ACMIProps` static class — four typed enums (`SP`, `FP`, `BP`, `IP`) and `PROP_MAP` for zero-boilerplate property dispatch
- `cWorldManager` session data store with `_parsing` flag to suppress signal spam during load
- `EntityRegistry` — lifetime bridge between data layer and 3D scene layer
- `ObjectManager` with per-category scene tree containers (`Aircraft`, `Weapons`, `Ground`, etc.)
- `TrailRenderer` — per-entity trail drawing with configurable `trail_before` / `trail_after` time windows and opacity fade
- `ACMIDecimator` — keep-every-Nth-frame decimation with event/creation/destruction frame protection
- `TimelineController` — scrub, step, and live playback modes
- `LabelManager` autoload — radial repulsion deconfliction for `ScreenLabel3D` instances
- `ScreenLabel3D` — 2D label anchored to a 3D world position with camera-facing visibility culling
- `GeoRaycaster` — ray–sphere intersection for geodetic coordinate picking in the 3D viewport
- `ColorRegistry` autoload — name-to-`ACMIColor` resource lookup for entity display colors
- `ACMIColor` resource — per-coalition/color display color, trail colors, and label colors
- `MeshLib` autoload — mesh resolution by shape name, object name, and type-tag fallback chain
- Country border rendering from binary border file
- ACMI text export (`_build_acmi_text`) reconstructing files from parsed session data
- `DashedLine2D` — Created in-house animated dashed polyline node with per-arc-length width curve and color gradient
- `DashedLine2D Editor Plugin` — point editor matching built-in Line2D UX (add, move, delete, insert, undo)

---

<!-- Link references — update tags as releases are made -->
[Unreleased]: https://github.com/Spearhead2/OpenACMI/compare/v0.1.0...HEAD
[v0.1.0]: https://github.com/Spearhead2/OpenACMI/
<!--releases/tag/v0.1.0-->
