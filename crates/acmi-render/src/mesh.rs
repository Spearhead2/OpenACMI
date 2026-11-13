//! Object models: Wavefront OBJ and binary glTF (`.glb`) readers, and the
//! library that picks a model for an object from its `Shape`, `Name` and
//! type tags.
//!
//! Model files follow Tacview's naming: `Category.Name.obj` (or `.glb`), for
//! example `FixedWing.F-16.obj` or `Vehicle.Tank.T-72.glb`, in metres, nose
//! along −Z and +Y up. Only the triangles are used: objects are drawn in
//! their coalition colour, so materials and textures are ignored, and
//! normals are recomputed on loading.

use std::fs;
use std::path::{Path, PathBuf};

use crate::normals::DEFAULT_CREASE_ANGLE_DEG;
use acmi_core::TagSet;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use rustc_hash::FxHashMap;

/// File extensions [`load_model`] understands, lower case.
pub const MODEL_EXTENSIONS: [&str; 2] = ["obj", "glb"];

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("cannot read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid glTF: {0}")]
    Gltf(#[from] gltf::Error),
    #[error("unsupported model format")]
    UnsupportedFormat,
    #[error("the model contains no triangles")]
    Empty,
}

/// Reads a model file, choosing the parser by extension.
///
/// The normals stored in the file are not used: every model is given
/// freshly computed ones (see [`crate::normals`]), so that all models are
/// shaded alike whatever exported them.
pub fn load_model(path: &Path) -> Result<CpuMesh, ModelError> {
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let mut mesh = match extension.as_str() {
        "obj" => parse_obj(&fs::read_to_string(path)?),
        "glb" => parse_glb(&fs::read(path)?)?,
        _ => return Err(ModelError::UnsupportedFormat),
    };
    if mesh.vertices.is_empty() {
        return Err(ModelError::Empty);
    }
    mesh.regenerate_normals(DEFAULT_CREASE_ANGLE_DEG);
    Ok(mesh)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// Triangle list ready for upload.
#[derive(Debug, Clone, Default)]
pub struct CpuMesh {
    pub vertices: Vec<MeshVertex>,
    /// Radius of the bounding sphere around the model origin.
    pub radius: f32,
}

/// Parses the geometry of an OBJ document. Materials, texture coordinates
/// and groups are ignored; polygons are fan-triangulated and faces without
/// normals are flat-shaded.
pub fn parse_obj(text: &str) -> CpuMesh {
    let mut positions: Vec<Vec3> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut vertices: Vec<MeshVertex> = Vec::new();
    let mut corners: Vec<(usize, Option<usize>)> = Vec::new();

    for line in text.lines() {
        let mut fields = line.split_ascii_whitespace();
        match fields.next() {
            Some("v") => positions.push(parse_vec3(fields)),
            Some("vn") => normals.push(parse_vec3(fields)),
            Some("f") => {
                corners.clear();
                corners.extend(fields.filter_map(|corner| parse_corner(corner, positions.len(), normals.len())));
                for i in 1..corners.len().saturating_sub(1) {
                    let triangle = [corners[0], corners[i], corners[i + 1]];
                    let [a, b, c] = triangle.map(|(p, _)| positions[p]);
                    let flat = (b - a).cross(c - a).normalize_or_zero();
                    for (p, n) in triangle {
                        let normal = n.map_or(flat, |n| normals[n]);
                        vertices.push(MeshVertex { position: positions[p].to_array(), normal: normal.to_array() });
                    }
                }
            }
            _ => {}
        }
    }

    let radius = bounding_radius(&vertices);
    CpuMesh { vertices, radius }
}

/// Parses the geometry of a binary glTF document.
///
/// Every triangle primitive of the default scene is flattened into one
/// triangle list with node transforms applied. Data must be embedded in the
/// file (the GLB binary chunk); external buffers are skipped.
pub fn parse_glb(bytes: &[u8]) -> Result<CpuMesh, gltf::Error> {
    let document = gltf::Gltf::from_slice(bytes)?;
    let blob = document.blob.as_deref();
    let mut mesh = CpuMesh::default();

    let scenes: Vec<gltf::Scene<'_>> = match document.default_scene() {
        Some(scene) => vec![scene],
        None => document.scenes().collect(),
    };
    let mut stack: Vec<(gltf::Node<'_>, Mat4)> =
        scenes.iter().flat_map(|scene| scene.nodes()).map(|node| (node, Mat4::IDENTITY)).collect();
    while let Some((node, parent)) = stack.pop() {
        let transform = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        stack.extend(node.children().map(|child| (child, transform)));
        let Some(gltf_mesh) = node.mesh() else { continue };
        for primitive in gltf_mesh.primitives().filter(|p| p.mode() == gltf::mesh::Mode::Triangles) {
            append_gltf_primitive(&mut mesh.vertices, &primitive, blob, transform);
        }
    }

    mesh.radius = bounding_radius(&mesh.vertices);
    Ok(mesh)
}

fn append_gltf_primitive(out: &mut Vec<MeshVertex>, primitive: &gltf::Primitive<'_>, blob: Option<&[u8]>, transform: Mat4) {
    let reader = primitive.reader(|buffer| match buffer.source() {
        gltf::buffer::Source::Bin => blob,
        gltf::buffer::Source::Uri(_) => None,
    });
    let Some(positions) = reader.read_positions() else { return };
    let positions: Vec<Vec3> = positions.map(|p| transform.transform_point3(Vec3::from_array(p))).collect();
    // Normals transform by the inverse transpose, which matters under non-uniform scale.
    let normal_matrix = glam::Mat3::from_mat4(transform).inverse().transpose();
    let normals: Option<Vec<Vec3>> =
        reader.read_normals().map(|n| n.map(|n| (normal_matrix * Vec3::from_array(n)).normalize_or_zero()).collect());
    let indices: Vec<u32> = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };

    for triangle in indices.chunks_exact(3) {
        let corner = |i: usize| positions.get(triangle[i] as usize).copied();
        let (Some(a), Some(b), Some(c)) = (corner(0), corner(1), corner(2)) else { continue };
        let flat = (b - a).cross(c - a).normalize_or_zero();
        for &index in triangle {
            let normal = normals.as_ref().and_then(|n| n.get(index as usize)).copied().unwrap_or(flat);
            out.push(MeshVertex { position: positions[index as usize].to_array(), normal: normal.to_array() });
        }
    }
}

fn bounding_radius(vertices: &[MeshVertex]) -> f32 {
    vertices.iter().map(|v| Vec3::from_array(v.position).length_squared()).fold(0.0, f32::max).sqrt()
}

fn parse_vec3<'a>(mut fields: impl Iterator<Item = &'a str>) -> Vec3 {
    let mut next = || fields.next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
    Vec3::new(next(), next(), next())
}

/// Parses `v`, `v/vt`, `v//vn` or `v/vt/vn` into zero-based indices.
/// Negative indices count back from the end, as the format allows.
fn parse_corner(corner: &str, position_count: usize, normal_count: usize) -> Option<(usize, Option<usize>)> {
    let resolve = |field: Option<&str>, count: usize| -> Option<usize> {
        let index: i64 = field?.parse().ok()?;
        let zero_based = if index < 0 { count as i64 + index } else { index - 1 };
        usize::try_from(zero_based).ok().filter(|&i| i < count)
    };
    let mut parts = corner.split('/');
    let position = resolve(parts.next(), position_count)?;
    let _texture = parts.next();
    Some((position, resolve(parts.next(), normal_count)))
}

/// Finds model files and resolves which one represents an object.
///
/// Resolution order:
/// 1. the object's `Shape` property (an explicit file name),
/// 2. its `Name`, tried exactly and then through progressively looser
///    variants (`MiG-29 Fulcrum` → `mig-29`, `F/A-18C` → `f-18`),
/// 3. a generic model for its type tags.
#[derive(Debug, Default)]
pub struct MeshLibrary {
    /// Lower-case file stem (`fixedwing.f-16`) → path.
    index: FxHashMap<String, PathBuf>,
}

/// Category prefixes tried in front of a bare model name, each with the tag
/// an object must carry for that category to make sense: a "Hawk" that is
/// `Ground+AntiAircraft` is a missile battery, not the trainer aircraft.
/// Objects without any tags may match every category.
const NAME_PREFIXES: &[(&str, TagSet)] = &[
    ("fixedwing.", TagSet::AIR),
    ("rotorcraft.", TagSet::AIR),
    ("missile.", TagSet::WEAPON),
    ("vehicle.tank.", TagSet::GROUND),
    ("vehicle.apc.", TagSet::GROUND),
    ("vehicle.truck.", TagSet::GROUND),
    ("vehicle.car.", TagSet::GROUND),
    ("vehicle.artillery.", TagSet::GROUND),
    ("watercraft.", TagSet::SEA),
    ("antiaircraft.sam.", TagSet::GROUND),
    ("antiaircraft.aaa.", TagSet::GROUND),
    ("antiaircraft.flak.", TagSet::GROUND),
    ("building.", TagSet::GROUND),
    ("rocket.", TagSet::WEAPON),
    ("bomb.", TagSet::WEAPON),
    ("other.", TagSet::EMPTY),
    ("spacecraft.", TagSet::EMPTY),
];

/// Objects named after the vehicle they came from ("F-4E_SEAT_PILOT",
/// "PILOT_MIG29") without being one: these never get a model by name.
const NOT_NAMED_AFTER_THEMSELVES: &[TagSet] =
    &[TagSet::SHRAPNEL, TagSet::PARACHUTIST, TagSet::HUMAN, TagSet::FLARE, TagSet::CHAFF, TagSet::EXPLOSION];

/// Generic models by tag, most specific first. `None` means "draw no model"
/// (tiny, numerous objects that read better as a trail alone). A model that
/// is not installed is passed over for the next entry that fits, so an
/// incomplete model set still draws something sensible.
const TAG_FALLBACKS: &[(TagSet, Option<&str>)] = &[
    (TagSet::SHELL, None),
    (TagSet::BULLET, None),
    (TagSet::FLARE, None),
    (TagSet::CHAFF, None),
    (TagSet::SHRAPNEL, None),
    (TagSet::EXPLOSION, None),
    (TagSet::BEAM, None),
    (TagSet::AIRCRAFT_CARRIER, Some("core.aircraftcarrier")),
    // Before AIR: an ejected pilot is tagged `Air` too.
    (TagSet::PARACHUTIST, Some("other.parachutist")),
    (TagSet::PARACHUTIST, Some("core.human")),
    (TagSet::ROTORCRAFT, Some("core.rotorcraft")),
    (TagSet::AIR, Some("core.fixedwing")),
    (TagSet::BOMB, Some("core.bomb")),
    (TagSet::TORPEDO, Some("core.torpedo")),
    (TagSet::TORPEDO, Some("core.missile")),
    (TagSet::MISSILE, Some("core.missile")),
    (TagSet::ROCKET, Some("core.missile")),
    (TagSet::WEAPON, Some("core.missile")),
    (TagSet::PROJECTILE, Some("core.missile")),
    (TagSet::SEA, Some("core.watercraft")),
    (TagSet::INFANTRY, Some("core.infantry")),
    (TagSet::INFANTRY, Some("core.human")),
    (TagSet::HUMAN, Some("core.human")),
    (TagSet::CONTAINER, Some("core.container")),
    (TagSet::WAYPOINT, Some("core.waypoint")),
    (TagSet::BULLSEYE, Some("core.waypoint")),
    (TagSet::NAVAID, Some("core.waypoint")),
    (TagSet::BUILDING, Some("core.cube")),
    (TagSet::AERODROME, Some("core.cube")),
    (TagSet::STATIC, Some("core.cube")),
    (TagSet::GROUND, Some("core.vehicle")),
    (TagSet::MISC, Some("core.cube")),
];

impl MeshLibrary {
    /// Adds every model file in `dir`. Later directories override earlier
    /// ones, so user models can replace the bundled set; within a directory
    /// `.glb` wins over `.obj` because it loads faster. Returns the number found.
    pub fn scan_dir(&mut self, dir: &Path) -> usize {
        let Ok(entries) = fs::read_dir(dir) else { return 0 };
        let mut count = 0;
        for path in entries.flatten().map(|entry| entry.path()) {
            let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
            let (true, Some(stem)) = (MODEL_EXTENSIONS.contains(&extension.as_str()), path.file_stem().and_then(|s| s.to_str()))
            else {
                continue;
            };
            let stem = stem.to_lowercase();
            let keeps_glb = self.index.get(&stem).is_some_and(|existing| {
                existing.parent() == path.parent() && existing.extension().is_some_and(|e| e.eq_ignore_ascii_case("glb"))
            });
            if !keeps_glb {
                self.index.insert(stem, path);
            }
            count += 1;
        }
        count
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// File names of every model, sorted, for choosing one by hand. Any of
    /// them works as an object's `Shape`.
    pub fn file_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.index.values().filter_map(|path| path.file_name()?.to_str()).collect();
        names.sort_unstable_by_key(|name| name.to_lowercase());
        names
    }

    /// Adds one model file from anywhere and returns the name to use as an
    /// object's `Shape`. `None` if it is not a model file.
    pub fn add_file(&mut self, path: &Path) -> Option<String> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        if !MODEL_EXTENSIONS.contains(&extension.as_str()) || !path.is_file() {
            return None;
        }
        self.index.insert(path.file_stem()?.to_str()?.to_lowercase(), path.to_owned());
        Some(path.file_name()?.to_str()?.to_owned())
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Path of the model for an object, or `None` if it should have no model.
    pub fn resolve(&self, shape: &str, name: &str, tags: TagSet) -> Option<&Path> {
        if !shape.is_empty() {
            let file = shape.rsplit(['/', '\\']).next().unwrap_or(shape).to_lowercase();
            let stem = MODEL_EXTENSIONS.iter().find_map(|ext| file.strip_suffix(&format!(".{ext}"))).unwrap_or(&file);
            if let Some(path) = self.index.get(stem) {
                return Some(path);
            }
        }
        let named_after_itself = !NOT_NAMED_AFTER_THEMSELVES.iter().any(|tag| tags.contains(*tag));
        if let Some(path) = self.resolve_by_name(name, tags).filter(|_| named_after_itself) {
            return Some(path);
        }
        self.resolve_by_tags(tags)
    }

    fn resolve_by_name(&self, name: &str, tags: TagSet) -> Option<&Path> {
        let normalised = normalise(name);
        if normalised.is_empty() {
            return None;
        }
        std::iter::once(normalised.clone()).chain(name_variants(&normalised)).find_map(|stem| self.lookup_stem(&stem, tags))
    }

    fn lookup_stem(&self, stem: &str, tags: TagSet) -> Option<&Path> {
        if let Some(path) = self.index.get(stem) {
            return Some(path);
        }
        NAME_PREFIXES
            .iter()
            .filter(|(_, required)| tags == TagSet::EMPTY || tags.contains(*required))
            .find_map(|(prefix, _)| self.index.get(&format!("{prefix}{stem}")))
            .map(PathBuf::as_path)
    }

    fn resolve_by_tags(&self, tags: TagSet) -> Option<&Path> {
        for (_, stem) in TAG_FALLBACKS.iter().filter(|(tag, _)| tags.contains(*tag)) {
            // The first fitting entry decides whether there is a model at all.
            let stem = (*stem)?;
            if let Some(path) = self.index.get(stem) {
                return Some(path);
            }
        }
        None
    }
}

fn normalise(name: &str) -> String {
    name.trim().to_lowercase().replace([' ', '_', '/', '.'], "-")
}

fn strip_trailing_letters(token: &str) -> &str {
    token.trim_end_matches(|c: char| c.is_ascii_lowercase())
}

/// Looser spellings of a normalised name, most specific first.
fn name_variants(normalised: &str) -> Vec<String> {
    let parts: Vec<&str> = normalised.split('-').collect();
    let mut variants: Vec<String> = Vec::new();
    let mut add = |variant: String| {
        if !variant.is_empty() && variant != normalised && !variants.contains(&variant) {
            variants.push(variant);
        }
    };

    // Drop trailing words: "mig-29-fulcrum" → "mig-29" → "mig".
    for end in (1..=parts.len()).rev() {
        add(parts[..end].join("-"));
    }
    // Drop a variant letter suffix: "mig-23mld" → "mig-23".
    for end in (1..=parts.len()).rev() {
        let stripped = strip_trailing_letters(parts[end - 1]);
        if !stripped.is_empty() && stripped != parts[end - 1] {
            let mut tokens = parts[..end - 1].to_vec();
            tokens.push(stripped);
            add(tokens.join("-"));
        }
    }
    // Recombine a short designator with a number: "f-a-18c" → "f-18".
    let numbers: Vec<&str> = parts
        .iter()
        .filter(|p| p.starts_with(|c: char| c.is_ascii_digit()))
        .map(|p| match strip_trailing_letters(p) {
            "" => *p,
            stripped => stripped,
        })
        .collect();
    for letters in parts.iter().filter(|p| !p.is_empty() && p.len() <= 4 && !p.starts_with(|c: char| c.is_ascii_digit())) {
        for number in &numbers {
            add(format!("{letters}-{number}"));
        }
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(stems: &[&str]) -> MeshLibrary {
        let mut library = MeshLibrary::default();
        for stem in stems {
            library.index.insert(stem.to_lowercase(), PathBuf::from(format!("{stem}.obj")));
        }
        library
    }

    #[test]
    fn obj_triangulates_and_shades() {
        let mesh = parse_obj(
            "v 0 0 0\nv 2 0 0\nv 2 0 -2\nv 0 0 -2\nvn 0 1 0\n# quad with normals, then a bare triangle\nf 1//1 2//1 3//1 4//1\nf -4 -3 -2\n",
        );
        assert_eq!(mesh.vertices.len(), 9);
        assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
        // Flat normal of (0,0,0)-(2,0,0)-(2,0,-2) points up.
        assert_eq!(mesh.vertices[8].normal, [0.0, 1.0, 0.0]);
        assert!((mesh.radius - 8f32.sqrt()).abs() < 1e-6);
    }

    #[test]
    fn obj_ignores_bad_indices() {
        let mesh = parse_obj("v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\nf 1 2 3\n");
        assert_eq!(mesh.vertices.len(), 3);
    }

    #[test]
    fn name_variants_loosen() {
        assert!(name_variants("mig-29-fulcrum").contains(&"mig-29".to_owned()));
        assert!(name_variants("mig-23mld").contains(&"mig-23".to_owned()));
        assert!(name_variants(&normalise("F/A-18C")).contains(&"f-18".to_owned()));
    }

    #[test]
    fn resolves_by_shape_name_then_tags() {
        let lib = library(&["FixedWing.F-16", "FixedWing.MiG-29", "Core.FixedWing", "Core.Missile", "Core.Cube", "Other.Custom"]);
        let air = TagSet::parse("Air+FixedWing");
        let file = |p: Option<&Path>| p.map(|p| p.to_str().unwrap().to_owned());

        assert_eq!(file(lib.resolve("Other.Custom.obj", "F-16C", air)), Some("Other.Custom.obj".into()));
        assert_eq!(file(lib.resolve("", "F-16C_50", air)), Some("FixedWing.F-16.obj".into()));
        assert_eq!(file(lib.resolve("", "MiG-29 Fulcrum", air)), Some("FixedWing.MiG-29.obj".into()));
        assert_eq!(file(lib.resolve("", "Su-27", air)), Some("Core.FixedWing.obj".into()));
        assert_eq!(file(lib.resolve("", "AIM-120C", TagSet::parse("Weapon+Missile"))), Some("Core.Missile.obj".into()));
        assert_eq!(file(lib.resolve("", "", TagSet::parse("Projectile+Shell"))), None);
        assert_eq!(file(lib.resolve("", "", TagSet::EMPTY)), None);
    }

    /// Assembles a GLB file from a JSON document and a binary chunk.
    fn glb(json: &str, binary: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        json.resize(json.len().next_multiple_of(4), b' ');
        let mut binary = binary.to_vec();
        binary.resize(binary.len().next_multiple_of(4), 0);

        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + json.len() + 8 + binary.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(binary.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&binary);
        out
    }

    #[test]
    fn glb_applies_node_transforms() {
        // One indexed triangle on a node translated 10 m along -Z.
        let json = r#"{
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"mesh": 0, "translation": [0, 0, -10]}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0}, "indices": 1}]}],
            "buffers": [{"byteLength": 44}],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 6}
            ],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0]},
                {"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ]
        }"#;
        let mut binary = Vec::new();
        for value in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0u16, 1, 2] {
            binary.extend_from_slice(&index.to_le_bytes());
        }

        let mesh = parse_glb(&glb(json, &binary)).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.vertices[1].position, [1.0, 0.0, -10.0]);
        // No normals in the file: the face normal is used.
        assert_eq!(mesh.vertices[0].normal, [0.0, 0.0, 1.0]);
        assert!((mesh.radius - 101f32.sqrt()).abs() < 1e-5);
        assert!(parse_glb(b"not a glb").is_err());
    }

    #[test]
    fn glb_wins_over_obj_in_the_same_folder() {
        let dir = std::env::temp_dir().join(format!("openacmi-mesh-scan-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["FixedWing.F-16.obj", "FixedWing.F-16.glb", "Core.Cube.obj", "notes.txt"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let mut library = MeshLibrary::default();
        assert_eq!(library.scan_dir(&dir), 3);
        // Whatever order the directory lists them in, twice over.
        library.scan_dir(&dir);
        let extension = |stem: &str| library.index[stem].extension().unwrap().to_str().unwrap().to_owned();
        assert_eq!(extension("fixedwing.f-16"), "glb");
        assert_eq!(extension("core.cube"), "obj");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_generic_models_fall_back_to_the_next_that_fits() {
        let library = library(&["Core.FixedWing", "Core.Human", "Core.Missile", "Core.Vehicle"]);
        let stem = |tags: &str| {
            library.resolve("", "", TagSet::parse(tags)).map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
        };
        // No helicopter, parachutist or torpedo model is installed.
        assert_eq!(stem("Air+Rotorcraft").as_deref(), Some("Core.FixedWing"));
        assert_eq!(stem("Ground+Light+Human+Air+Parachutist").as_deref(), Some("Core.Human"));
        assert_eq!(stem("Weapon+Torpedo").as_deref(), Some("Core.Missile"));
        // Objects that are deliberately not drawn stay that way.
        assert_eq!(stem("Projectile+Shell"), None);
    }

    #[test]
    fn names_only_match_models_of_their_own_kind() {
        let library = library(&[
            "FixedWing.Hawk",
            "FixedWing.F-4",
            "FixedWing.MiG-29",
            "Core.FixedWing",
            "Core.Vehicle",
            "Other.Parachutist",
        ]);
        let stem = |name: &str, tags: &str| {
            library.resolve("", name, TagSet::parse(tags)).map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
        };
        assert_eq!(stem("MiG-29 Fulcrum", "Air+FixedWing").as_deref(), Some("FixedWing.MiG-29"));
        assert_eq!(stem("F-4E-45MC", "Air+FixedWing").as_deref(), Some("FixedWing.F-4"));
        // A Hawk missile launcher is not the Hawk trainer.
        assert_eq!(stem("Hawk ln", "Ground+AntiAircraft").as_deref(), Some("Core.Vehicle"));
        assert_eq!(stem("Hawk", "Air+FixedWing").as_deref(), Some("FixedWing.Hawk"));
        // Without tags the name is all there is to go by.
        assert_eq!(stem("Hawk", "").as_deref(), Some("FixedWing.Hawk"));
        // Debris and crew are named after the aircraft they left.
        assert_eq!(stem("F-4E_SEAT_PILOT", "Misc+Shrapnel"), None);
        assert_eq!(stem("F-4E_PILOT", "Ground+Light+Human+Air+Parachutist").as_deref(), Some("Other.Parachutist"));
        assert_eq!(stem("Pilot", "Ground+Light+Human+Air+Parachutist").as_deref(), Some("Other.Parachutist"));
    }
}
