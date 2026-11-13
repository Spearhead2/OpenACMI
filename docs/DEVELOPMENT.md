# OpenACMI developer guide

A community-built alternative to Tacview: replay flights, analyse engagements and
monitor live telemetry from any source that speaks the ACMI 2.x format.

This guide is for people working on the code. For what the program does and how to use it, see the [README](../README.md).

## Build and run

```bash
cargo run --release -p openacmi -- samples/demo.acmi
```

Requires a recent stable Rust toolchain and a GPU with Vulkan, DirectX 12 or Metal.
The release binary expects the `assets` folder next to it (or `OPENACMI_ASSETS` to
point at it); when run through cargo the folder in this repository is found
above the executable.

```bash
cargo test --workspace
```

## Workspace layout

| Crate | Responsibility | Depends on |
|-------|----------------|------------|
| `acmi-core` | Data model, ACMI parser and writer, geodesy. No I/O. | `glam` |
| `acmi-io` | Loading/saving files (plain and zip), real-time telemetry client. | `acmi-core`, `zip` |
| `acmi-terrain` | Streams, caches and decodes elevation tiles; colours them by elevation. No GPU code. | `ureq`, `image` |
| `acmi-render` | wgpu renderer: globe, streamed terrain, grid, borders, trails, models. Knows nothing about recordings or UI. | `wgpu`, `acmi-terrain` |
| `openacmi` | The application: egui UI, camera, timeline, settings. | all of the above, `eframe` |

The split is what keeps the project easy to extend: the parser can be used
headless (tools, tests, servers), and the renderer only consumes plain data
(`Frame`, `ModelInstance`, `LineVertex`).

### Data model in one paragraph

A `Recording` holds objects, events and global metadata. Each `Object` has a
`Track` (structure-of-arrays transform history, bisected and interpolated on
demand) plus one sparse time series per property, so the full state of any object
can be reconstructed at any time. Property names are interned to `PropId`s; the
properties of the ACMI specification are declared in one table in
`acmi-core/src/props.rs`, and anything else found in a file is registered on first
sight rather than dropped.

### Rendering in one paragraph

The world is a real-size Earth in metres, far beyond `f32` precision, so every
position is uploaded as a double split into two floats and subtracted from the
equally split eye position on the GPU ("relative to eye"). Vertex buffers therefore
never depend on the camera, and trails are rebuilt only when the displayed time
changes. Depth is reversed with an infinite far plane. The scene renders to an
off-screen target that the UI shows as an image.

The atmosphere is computed, not modelled: every ground pixel and every sky pixel
integrates Rayleigh and Mie scattering along its view ray (8 to 16 steps, each
with a 4-step ray towards the sun, no textures). One function therefore gives the
sky seen from the ground, the blue rim seen from space, the haze over distant
terrain, and red light near the terminator. Objects, trails, grid and borders are
drawn after it and are not hazed. `earth.atmosphere_strength` (1 = the real
atmosphere) and `earth.atmosphere_exposure` tune it; `shaders/globe.wgsl` holds
the constants.

## Terrain

The globe streams real elevation data and drapes it as 3D terrain that sharpens
as you zoom in, textured with streamed satellite imagery. With imagery off (or
where none is available) the ground colour is synthesised from the elevation.

* **Source**: the public "Terrain Tiles" dataset on AWS Open Data (Mapzen
  Terrarium encoding), which is free and needs no account or key. Elevation data
  © Mapzen and its sources (SRTM, GMTED, NED, ETOPO1 and others). Requests are
  plain tile downloads and carry no information about you or your recordings.
* **Imagery**: "Sentinel-2 cloudless" by EOX IT Services GmbH (contains modified
  Copernicus Sentinel data 2016 & 2017), licensed CC BY 4.0: a cloud-free mosaic
  at about 10 m per pixel, free with attribution and no key. The credit is shown
  in the corner of the view. Relief shading from the elevation is laid over it.
  Any other Web-Mercator "XYZ" tile server can be used instead by changing
  `imagery_url_template`, e.g. OpenStreetMap's street map
  (`https://tile.openstreetmap.org/{z}/{x}/{y}.png`); check the server's usage
  policy first and set `imagery_attribution` to the credit it requires.
* **Cache**: every downloaded tile is kept on disk
  (`%LOCALAPPDATA%\OpenACMI\cache\terrain` and `...\cache\imagery` on Windows),
  laid out `z/x/y`, so an area you have viewed works offline afterwards. The
  cache is never pruned; delete the folders to reclaim space.
* **Ground selector**: the "Ground" list in the View panel picks what the
  globe is drawn with: satellite imagery, elevation colours in one of five
  schemes, or the bundled flat world map without terrain.
* **Elevation colours**: water is blue, darkening with depth, and land is
  coloured by height with relief shading baked in. The schemes are `natural`
  (green lowlands through tan and rock to snow), `atlas` (printed-atlas tints),
  `arid` (sand and rock), `greyscale`, and `dark` (dim, so objects stand out).
  The snow line drops towards the poles in all of them.
* **Settings** (`[terrain]` in `settings.toml`): `enabled`, `online` (set false
  to use only the cache), `url_template` (any Terrarium-encoded tile server),
  `cache_dir`, `max_zoom` (12 ≈ 30 m per sample), `pixels_per_sample`,
  `vertical_exaggeration`, `download_threads`, `elevation_colors` (a scheme name),
  and for imagery `imagery`,
  `imagery_url_template`, `imagery_max_zoom`, `imagery_relief` (0 to 1),
  `imagery_brightness`, `imagery_saturation`, `imagery_gamma`, `imagery_attribution`. Terrain and
  imagery can also be toggled in the View panel; without terrain the original
  flat world map is shown.

With terrain on, the telemetry panel shows height above ground from the DEM when
the recording has no `AGL`, clicking the globe reports the elevation, and the
camera stays above the ground.

Land below sea level is handled by a table of enclosed basins in
`acmi-terrain/src/surface.rs` (Jordan Rift Valley, Death Valley, Salton Trough,
Danakil, Qattara, Caspian, Turpan and others). Inside a basin the ground keeps
its true negative height and is drawn as sand, and lakes such as the Dead Sea
sit at their own water level. Everywhere else, at or below sea level means ocean.

Known limits of colouring from elevation alone: deserts above sea level come out
green, lakes above sea level are not blue, and low land directly on a coast (the
Dutch polders, New Orleans) is still drawn as sea. Poles beyond 85° are plain caps. The grid and borders are drawn at
sea level, so they shift slightly against high terrain at oblique angles.

## Controls

| Input | Action |
|-------|--------|
| Left / right drag | Orbit (Global, Tracking), look around (Cockpit, Free) |
| Wheel, `+` / `-` | Zoom; in Free flight, change speed |
| `W A S D`, arrows | Orbit / fly; `Q` `E` down / up in Free flight; `Shift` = faster |
| Click an object | Select as primary (`Ctrl`/`Shift`+click: secondary) |
| Click the globe | Show the coordinates under the pointer |
| Middle drag | Scrub through time |
| `Shift` + right drag | Move the sun |
| `Space` | Play / pause |
| `,` `.` | Previous / next sample of the primary object |
| `Home` `End` | Start / end of the recording |
| `C` | Cycle camera mode |
| `Ctrl+O`, `Ctrl+S` | Open, save as |

Files can also be dropped onto the window.

## Settings and user content

## Panels and graphs

Every panel (Telemetry 1 and 2, Graph 1 and 2, Compare, Session, Event Log,
View) can be docked to the left, right, top or bottom of the 3D view, where
panels sharing a side become tabs, or undocked into a floating window:

* the `…` button at the end of a dock's tab row moves or closes the tab in front;
* a floating window has Left / Right / Top / Bottom buttons, and docks when it
  is dragged to an edge of the 3D view;
* **View** in the menu bar lists every panel with its place, and resets the layout.

Graph 1 plots the primary object, Graph 2 the secondary one, and Compare draws
both on the same axes (primary blue, secondary orange). **Plot…** chooses the
quantities (altitude, speeds, Mach, vertical speed, heading, pitch, roll, AOA,
G, turn rate), one strip each; the list next to it sets the time span. Click or
drag in a strip to move playback there. In a top or bottom dock the strips sit
side by side. Compare also plots what relates the two objects: distance,
closure rate (positive while closing), bearing to target and altitude delta
(secondary relative to primary).

Where a recording lacks IAS, CAS, TAS, Mach, G or AOA, they are estimated and
marked **DERIVED** in amber, with dashed lines in graphs; hovering a value says
how. The estimates (`crates/acmi-core/src/derive.rs`) assume the standard
atmosphere and no wind, fit the flight path over a few seconds, and need
recorded attitude for AOA. Recorded values are always used when present.

A new quantity is a variant in `crates/openacmi/src/quantity.rs`;
a new panel is a variant of `PanelId` in `config.rs` plus its contents in
`ui/panels.rs`.

Right-clicking an object in the 3D view opens a menu with **Edit Object…**:
its name, call sign, pilot, group, coalition, country, type and colour can be
changed, and its 3D model chosen from the library or from any `.glb` / `.obj`
file. Edits apply to the whole recording at once and are included in
**File › Save As**; the model is stored in the object's `Shape` property.

**File › Preferences…** holds the settings that are changed less often,
grouped by what they affect; hover any of them for what it does. Changes apply
at once, except those marked "restart". The window's place, size and maximised
state, the sizes of docks and the positions of floating panels are remembered
between runs (in `data/app.ron` beside the settings).

Settings live in `settings.toml` in the platform configuration directory
(`%APPDATA%\OpenACMI\config` on Windows) and are written on exit. Every option is
documented in `crates/openacmi/src/config.rs`.

User content is read from the folder chosen with **File › 3D Model Folder…**
(`general.user_data_dir` in the settings), else the `OPENACMI_DATA_DIR`
environment variable, else a `data` folder next to the executable. Changing the
folder takes effect at once. While no models of your own are found, the 3D view
says so and the telemetry panel marks the stand-in as `(generic)`.

* `meshesobj/` and `meshes/`: object models as `.obj` or binary glTF (`.glb`),
  named like Tacview's, e.g. `FixedWing.F-16.glb`. They override the bundled
  generic shapes; where both formats exist the `.glb` is used. Only geometry is
  read: objects are drawn in their coalition colour.
* `acmi_colors.json`: colour schemes keyed by the ACMI `Color` property; entries
  override or extend `assets/acmi_colors.json`.

## Extending

* **A new ACMI property**: one line in the `known_props!` table (`acmi-core/src/props.rs`).
* **A new object tag**: one entry in `tags!` (`acmi-core/src/tags.rs`); a generic
  model for it is one entry in `TAG_FALLBACKS` (`acmi-render/src/mesh.rs`).
* **A new setting**: a field with a default in `config.rs`.
* **A new panel**: a module under `crates/openacmi/src/ui/` drawing from `App`.
* **A new visual**: a pipeline in `acmi-render/src/renderer.rs` fed by a field of `Frame`.
* **A new model format**: a parser returning `CpuMesh` plus its extension in
  `MODEL_EXTENSIONS` (`acmi-render/src/mesh.rs`).
* **Another elevation colour scheme**: a variant of `ElevationPalette` with its
  colour stops in `acmi-terrain/src/shading.rs`; it appears in the Ground list by itself.
* **Another basin below sea level**: one row in `BASINS` (`acmi-terrain/src/surface.rs`);
  the `probe` example in that crate measures a region's heights.
* **A new data source**: anything that can produce ACMI lines can feed
  `Parser::feed_line`, exactly as the telemetry client does.

## Differences from the Godot version

Added: lossless loading by default (frame decimation is now opt-in via
`loading.keep_every_nth_frame`), every property kept as a time series, a writer
that round-trips all of them, Save As and CSV export, playback speed control,
object picking in the 3D view, derived speed / climb / turn rate, real sun position
from the recording date, line continuations and recycled object ids in the parser,
the telemetry password hash, streamed 3D terrain with satellite imagery or
elevation-based colouring, and `.glb` models.

Not ported: the HUD/MFD "Tapes" placeholders,
the bundled height-mapped and normal-mapped Earth material (replaced by streamed
terrain), and the menu entries that were placeholders in the original (merge, ADS-B, online debriefing,
controller sessions).

## Assets

`assets/borders.bin`, `assets/textures/world_map.png` and `assets/acmi_colors.json`
come unchanged from the Godot project, as do the generic shapes in `assets/meshes`.
Only shapes made for OpenACMI are in the repository; models from other libraries
are the user's to install (see the README).
