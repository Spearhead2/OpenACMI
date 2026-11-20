# Changelog

All notable changes to OpenACMI will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

---

## [v0.3.0] - 2026-11-20

### Added
- A night sky of real stars: the 9,096 stars of the Yale Bright Star Catalogue, placed for the recorded date and time, with their brightness and colour, dimmed by the air and hidden by daylight. Switch it in the View panel; brightness is in Preferences
- Preferences shows how much disk space the downloaded terrain and imagery tiles take, with buttons to clear them and to open the folder

---

## [v0.2.0] - 2026-11-19

### Added
- A head-up display in the cockpit view of an aircraft, drawn on a plane fixed to the airframe
- A simpler display in the cockpit view of a weapon: a reticle around the nose, flight path marker, Mach, airspeed, altitude and the box on the secondary object
- Zoom in the cockpit view by holding the right mouse button and turning the wheel
- `export_map`, a command-line tool that exports a flat map picture of any latitude / longitude box in the satellite look or an elevation colour scheme

### Fixed
- Pale rectangles in deep water: sea tiles that hold the flat water surface now take their depth from coarser tiles
- Around the Dead Sea: the Arava and the plain of Sodom were drawn as sea, the evaporation ponds south of the Dead Sea were missing, and the Yarmouk gorge showed a sliver of false water

---

## [v0.1.1] - 2026-11-18

### Added
- Windows file association: open `.acmi`, `.zip.acmi` and `.txt.acmi` recordings with OpenACMI by double-click, from Preferences or with `--associate` / `--remove-association`
- Object size settings in Preferences: switch the enlargement of distant objects on or off, limit it, keep objects at real size within a chosen distance, set the size of weapons and static objects, and scale all models
- Generic shapes for helicopters, rotors, torpedoes, infantry, artillery, parachutists, drop tanks and rocket pods

### Fixed
- Falcon BMS recordings opened with hours of nothing before the action: the starting state BMS writes before its first time marker is now attached to the first frame
- Land below sea level around the Caspian Sea was drawn as ocean, and the sea itself was coloured in squares

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

[v0.3.0]: https://github.com/Spearhead2/OpenACMI/releases/tag/v0.3.0
[v0.2.0]: https://github.com/Spearhead2/OpenACMI/releases/tag/v0.2.0
[v0.1.1]: https://github.com/Spearhead2/OpenACMI/releases/tag/v0.1.1
[v0.1.0]: https://github.com/Spearhead2/OpenACMI/releases/tag/v0.1.0
