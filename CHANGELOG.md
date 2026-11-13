# Changelog

All notable changes to OpenACMI will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [v0.1.0] - 2026-11-12

Initial source release. OpenACMI is now written in Rust, on wgpu and egui.

### Added
- ACMI 2.x parser that keeps every property of every object as a time series, with lossless loading, Save As and CSV export of an object's telemetry
- Plain and zip-compressed recordings, detected from the file's contents
- Real-time telemetry client, including the password handshake
- 3D globe with country borders, latitude / longitude grid and a physically based atmosphere
- Streamed 3D terrain from open elevation data, with correct handling of land below sea level
- Streamed satellite imagery, and five elevation colour schemes as alternatives
- Sun position from the recording's date and time
- Object models in `.glb` and `.obj`, matched by name and type, with normals regenerated on load
- Ribbon trails for aircraft and helicopters that show roll; line trails and altitude lines for everything else
- Telemetry panels for a primary and a secondary object
- Graph panels for each, and a comparison graph with distance, closure rate, bearing to target and altitude difference
- Estimates of IAS, CAS, TAS, Mach, G and AOA where a recording lacks them, always marked as derived
- Event log and timeline marks
- Panels that dock to any side of the 3D view or float
- Global orbit, tracking, cockpit and free-flight cameras
- Editing of an object's identity and 3D model from a right-click menu
- Preferences window; window, panel layout and settings remembered between runs
- Windows release build

---

[v0.1.0]: https://github.com/Spearhead2/OpenACMI/releases/tag/v0.1.0
