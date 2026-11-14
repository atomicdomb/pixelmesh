//! Minimal binary FBX 7.4 writer.
//!
//! Writes one model per scene object, each with flat normals, one UV set, plus one shared material and (optionally)
//! one diffuse texture, either referenced by file name or embedded.
//! The layout follows what Blender's exporter emits, which both Unity and
//! Blender's importer read happily. Binary rather than ASCII because Blender
//! refuses ASCII FBX.

use crate::mesh::Mesh;
use glam::Vec2;
use std::collections::HashMap;
use std::io;
use std::path::Path;

pub struct FbxTexture<'a> {
    /// File name written into the FBX (relative, next to the .fbx).
    pub file_name: &'a str,
    /// Raw bytes of the image file if it should be embedded.
    pub embed: Option<&'a [u8]>,
}

/// One scene object; each becomes its own Model + Geometry in the file.
pub struct FbxObject<'a> {
    pub name: &'a str,
    /// Geometry in world space.
    pub mesh: &'a Mesh,
    /// The object's pivot in world space; written as the model's translation.
    pub origin: glam::Vec3,
    /// The object's orientation as XYZ Euler angles in degrees; written as the model's rotation.
    pub rotation: glam::Vec3,
}

impl<'a> FbxObject<'a> {
    /// An object whose pivot is the world origin, with no rotation.
    pub fn baked(name: &'a str, mesh: &'a Mesh) -> Self {
        FbxObject { name, mesh, origin: glam::Vec3::ZERO, rotation: glam::Vec3::ZERO }
    }
}

pub struct FbxOptions<'a> {
    /// Name of the single material shared by all objects.
    pub material_name: &'a str,
    pub texture: Option<FbxTexture<'a>>,
    /// Multiplies all positions on export.
    pub scale: f32,
}

// ------------------------------------------------------------------ node tree

enum Prop {
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Str(Vec<u8>),
    Raw(Vec<u8>),
    F64s(Vec<f64>),
    I32s(Vec<i32>),
}

struct Node {
    name: &'static str,
    props: Vec<Prop>,
    children: Vec<Node>,
}

fn s(v: &str) -> Prop {
    Prop::Str(v.as_bytes().to_vec())
}

/// Object names are stored as "Name\0\x01Class" in binary FBX.
fn name_class(name: &str, class: &str) -> Prop {
    let mut b = name.as_bytes().to_vec();
    b.extend_from_slice(b"\x00\x01");
    b.extend_from_slice(class.as_bytes());
    Prop::Str(b)
}

impl Node {
    fn new(name: &'static str) -> Self {
        Node { name, props: Vec::new(), children: Vec::new() }
    }
    fn p(mut self, prop: Prop) -> Self {
        self.props.push(prop);
        self
    }
    fn c(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }
    fn leaf(name: &'static str, prop: Prop) -> Self {
        Node::new(name).p(prop)
    }

    fn write(&self, out: &mut Vec<u8>, is_last: bool) {
        let start = out.len();
        out.extend_from_slice(&[0; 12]); // end offset, prop count, prop bytes: patched below
        out.push(self.name.len() as u8);
        out.extend_from_slice(self.name.as_bytes());
        let props_start = out.len();
        for p in &self.props {
            p.write(out);
        }
        let props_len = out.len() - props_start;
        write_children(&self.children, self.props.is_empty(), is_last, out);
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(self.props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&(props_len as u32).to_le_bytes());
    }
}

const SENTINEL: [u8; 13] = [0; 13];

fn write_children(children: &[Node], no_props: bool, is_last: bool, out: &mut Vec<u8>) {
    if !children.is_empty() {
        for (i, ch) in children.iter().enumerate() {
            ch.write(out, i + 1 == children.len());
        }
        out.extend_from_slice(&SENTINEL);
    } else if no_props && !is_last {
        // Empty scopes also need the terminator unless they are the last sibling.
        out.extend_from_slice(&SENTINEL);
    }
}

impl Prop {
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Prop::Bool(v) => {
                out.push(b'C');
                out.push(*v as u8);
            }
            Prop::I32(v) => {
                out.push(b'I');
                out.extend_from_slice(&v.to_le_bytes());
            }
            Prop::I64(v) => {
                out.push(b'L');
                out.extend_from_slice(&v.to_le_bytes());
            }
            Prop::F64(v) => {
                out.push(b'D');
                out.extend_from_slice(&v.to_le_bytes());
            }
            Prop::Str(v) | Prop::Raw(v) => {
                out.push(if matches!(self, Prop::Str(_)) { b'S' } else { b'R' });
                out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                out.extend_from_slice(v);
            }
            Prop::F64s(v) => {
                out.push(b'd');
                array_header(out, v.len(), 8);
                v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
            }
            Prop::I32s(v) => {
                out.push(b'i');
                array_header(out, v.len(), 4);
                v.iter().for_each(|x| out.extend_from_slice(&x.to_le_bytes()));
            }
        }
    }
}

fn array_header(out: &mut Vec<u8>, len: usize, elem: usize) {
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // encoding 0 = uncompressed
    out.extend_from_slice(&((len * elem) as u32).to_le_bytes());
}

/// A `P:` entry inside Properties70.
fn p70(name: &str, ty: &str, ty2: &str, flags: &str, values: Vec<Prop>) -> Node {
    let mut n = Node::new("P").p(s(name)).p(s(ty)).p(s(ty2)).p(s(flags));
    n.props.extend(values);
    n
}

fn p70_int(name: &str, v: i32) -> Node {
    p70(name, "int", "Integer", "", vec![Prop::I32(v)])
}

fn p70_double(name: &str, v: f64) -> Node {
    p70(name, "double", "Number", "", vec![Prop::F64(v)])
}

fn p70_vec(name: &str, ty: &str, ty2: &str, flags: &str, v: [f64; 3]) -> Node {
    p70(name, ty, ty2, flags, v.iter().map(|x| Prop::F64(*x)).collect())
}

// ---------------------------------------------------------------- the document

/// (geometry id, model id) for the object at `index`.
fn ids(index: usize) -> (i64, i64) {
    (2_000_000 + index as i64, 3_000_000 + index as i64)
}
const MATERIAL_ID: i64 = 1_000_003;
const TEXTURE_ID: i64 = 1_000_004;
const VIDEO_ID: i64 = 1_000_005;

/// Geometry + Model nodes for one object.
fn object_nodes(index: usize, obj: &FbxObject, scale: f32) -> (Node, Node) {
    let (geometry_id, model_id) = ids(index);
    let name = obj.name;
    // The file stores geometry relative to the model's pivot and orientation.
    let to_local = crate::xform::euler_quat(obj.rotation).inverse();
    let local = Mesh {
        positions: obj.mesh.positions.iter().map(|&p| to_local * (p - obj.origin)).collect(),
        faces: obj.mesh.faces.clone(),
    };
    let mesh = &local;
    let origin = obj.origin * scale;

    // ---- geometry arrays
    let vertices: Vec<f64> = mesh.positions.iter().flat_map(|p| {
        let p = *p * scale;
        [p.x as f64, p.y as f64, p.z as f64]
    }).collect();

    let mut poly_index = Vec::new();
    let mut normals = Vec::new();
    let mut uv_values: Vec<f64> = Vec::new();
    let mut uv_index = Vec::new();
    let mut uv_lookup: HashMap<[u32; 2], i32> = HashMap::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        let n = mesh.face_normal(fi);
        for (k, &v) in face.verts.iter().enumerate() {
            // The last corner of every polygon is stored bit-inverted.
            let last = k + 1 == face.verts.len();
            poly_index.push(if last { !(v as i32) } else { v as i32 });
            normals.extend([n.x as f64, n.y as f64, n.z as f64]);
            let uv: Vec2 = face.uvs[k];
            let key = [uv.x.to_bits(), uv.y.to_bits()];
            let next = (uv_values.len() / 2) as i32;
            let idx = *uv_lookup.entry(key).or_insert_with(|| {
                uv_values.extend([uv.x as f64, uv.y as f64]);
                next
            });
            uv_index.push(idx);
        }
    }

    let layer_element = |ty: &str| {
        Node::new("LayerElement").c(Node::leaf("Type", s(ty))).c(Node::leaf("TypedIndex", Prop::I32(0)))
    };

    let geometry = Node::new("Geometry")
        .p(Prop::I64(geometry_id))
        .p(name_class(name, "Geometry"))
        .p(s("Mesh"))
        .c(Node::leaf("GeometryVersion", Prop::I32(124)))
        .c(Node::leaf("Vertices", Prop::F64s(vertices)))
        .c(Node::leaf("PolygonVertexIndex", Prop::I32s(poly_index)))
        .c(Node::new("LayerElementNormal")
            .p(Prop::I32(0))
            .c(Node::leaf("Version", Prop::I32(101)))
            .c(Node::leaf("Name", s("")))
            .c(Node::leaf("MappingInformationType", s("ByPolygonVertex")))
            .c(Node::leaf("ReferenceInformationType", s("Direct")))
            .c(Node::leaf("Normals", Prop::F64s(normals))))
        .c(Node::new("LayerElementUV")
            .p(Prop::I32(0))
            .c(Node::leaf("Version", Prop::I32(101)))
            .c(Node::leaf("Name", s("UVMap")))
            .c(Node::leaf("MappingInformationType", s("ByPolygonVertex")))
            .c(Node::leaf("ReferenceInformationType", s("IndexToDirect")))
            .c(Node::leaf("UV", Prop::F64s(uv_values)))
            .c(Node::leaf("UVIndex", Prop::I32s(uv_index))))
        .c(Node::new("LayerElementMaterial")
            .p(Prop::I32(0))
            .c(Node::leaf("Version", Prop::I32(101)))
            .c(Node::leaf("Name", s("")))
            .c(Node::leaf("MappingInformationType", s("AllSame")))
            .c(Node::leaf("ReferenceInformationType", s("IndexToDirect")))
            .c(Node::leaf("Materials", Prop::I32s(vec![0]))))
        .c(Node::new("Layer")
            .p(Prop::I32(0))
            .c(Node::leaf("Version", Prop::I32(100)))
            .c(layer_element("LayerElementNormal"))
            .c(layer_element("LayerElementUV"))
            .c(layer_element("LayerElementMaterial")));

    let model = Node::new("Model")
        .p(Prop::I64(model_id))
        .p(name_class(name, "Model"))
        .p(s("Mesh"))
        .c(Node::leaf("Version", Prop::I32(232)))
        .c(Node::new("Properties70")
            .c(p70_vec("Lcl Translation", "Lcl Translation", "", "A", [origin.x as f64, origin.y as f64, origin.z as f64]))
            .c(p70_vec("Lcl Rotation", "Lcl Rotation", "", "A", [obj.rotation.x as f64, obj.rotation.y as f64, obj.rotation.z as f64]))
            .c(p70_vec("Lcl Scaling", "Lcl Scaling", "", "A", [1.0; 3]))
            .c(p70_int("DefaultAttributeIndex", 0))
            .c(p70("InheritType", "enum", "", "", vec![Prop::I32(1)])))
        .c(Node::leaf("MultiLayer", Prop::I32(0)))
        .c(Node::leaf("MultiTake", Prop::I32(0)))
        .c(Node::leaf("Shading", Prop::Bool(true)))
        .c(Node::leaf("Culling", s("CullingOff")));

    (geometry, model)
}

pub fn export_bytes(objects: &[FbxObject], opt: &FbxOptions) -> Vec<u8> {
    let version = 7400;
    let has_tex = opt.texture.is_some();
    let count = objects.len() as i32;

    let material = Node::new("Material")
        .p(Prop::I64(MATERIAL_ID))
        .p(name_class(opt.material_name, "Material"))
        .p(s(""))
        .c(Node::leaf("Version", Prop::I32(102)))
        .c(Node::leaf("ShadingModel", s("lambert")))
        .c(Node::leaf("MultiLayer", Prop::I32(0)))
        .c(Node::new("Properties70")
            .c(p70_vec("DiffuseColor", "Color", "", "A", [1.0; 3]))
            .c(p70("DiffuseFactor", "Number", "", "A", vec![Prop::F64(1.0)]))
            .c(p70_vec("AmbientColor", "Color", "", "A", [0.0; 3]))
            .c(p70_vec("EmissiveColor", "Color", "", "A", [0.0; 3])));

    let mut objects_node = Node::new("Objects");
    let mut connections = Node::new("Connections");
    for (i, obj) in objects.iter().enumerate() {
        let (geometry_id, model_id) = ids(i);
        let (geometry, model) = object_nodes(i, obj, opt.scale);
        objects_node = objects_node.c(geometry).c(model);
        connections = connections
            .c(Node::new("C").p(s("OO")).p(Prop::I64(model_id)).p(Prop::I64(0)))
            .c(Node::new("C").p(s("OO")).p(Prop::I64(geometry_id)).p(Prop::I64(model_id)))
            .c(Node::new("C").p(s("OO")).p(Prop::I64(MATERIAL_ID)).p(Prop::I64(model_id)));
    }
    objects_node = objects_node.c(material);

    if let Some(tex) = &opt.texture {
        let tex_name = Path::new(tex.file_name).file_stem().and_then(|x| x.to_str()).unwrap_or("texture");
        let mut video = Node::new("Video")
            .p(Prop::I64(VIDEO_ID))
            .p(name_class(tex_name, "Video"))
            .p(s("Clip"))
            .c(Node::leaf("Type", s("Clip")))
            .c(Node::new("Properties70").c(p70("Path", "KString", "XRefUrl", "", vec![s(tex.file_name)])))
            .c(Node::leaf("UseMipMap", Prop::I32(0)))
            .c(Node::leaf("Filename", s(tex.file_name)))
            .c(Node::leaf("RelativeFilename", s(tex.file_name)));
        if let Some(bytes) = tex.embed {
            video = video.c(Node::leaf("Content", Prop::Raw(bytes.to_vec())));
        }
        let texture = Node::new("Texture")
            .p(Prop::I64(TEXTURE_ID))
            .p(name_class(tex_name, "Texture"))
            .p(s(""))
            .c(Node::leaf("Type", s("TextureVideoClip")))
            .c(Node::leaf("Version", Prop::I32(202)))
            .c(Node::leaf("TextureName", name_class(tex_name, "Texture")))
            .c(Node::new("Properties70")
                .c(p70("UVSet", "KString", "", "", vec![s("UVMap")]))
                .c(p70("UseMaterial", "bool", "", "", vec![Prop::I32(1)])))
            .c(Node::leaf("Media", name_class(tex_name, "Video")))
            .c(Node::leaf("FileName", s(tex.file_name)))
            .c(Node::leaf("RelativeFilename", s(tex.file_name)))
            .c(Node::new("ModelUVTranslation").p(Prop::F64(0.0)).p(Prop::F64(0.0)))
            .c(Node::new("ModelUVScaling").p(Prop::F64(1.0)).p(Prop::F64(1.0)))
            .c(Node::leaf("Texture_Alpha_Source", s("None")))
            .c(Node::new("Cropping").p(Prop::I32(0)).p(Prop::I32(0)).p(Prop::I32(0)).p(Prop::I32(0)));
        objects_node = objects_node.c(texture).c(video);
        connections = connections
            .c(Node::new("C").p(s("OP")).p(Prop::I64(TEXTURE_ID)).p(Prop::I64(MATERIAL_ID)).p(s("DiffuseColor")))
            .c(Node::new("C").p(s("OO")).p(Prop::I64(VIDEO_ID)).p(Prop::I64(TEXTURE_ID)));
    }

    let object_type = |ty: &str, count: i32| Node::new("ObjectType").p(s(ty)).c(Node::leaf("Count", Prop::I32(count)));
    let mut definitions = Node::new("Definitions")
        .c(Node::leaf("Version", Prop::I32(100)))
        .c(Node::leaf("Count", Prop::I32(2 + 2 * count + if has_tex { 2 } else { 0 })))
        .c(object_type("GlobalSettings", 1))
        .c(object_type("Geometry", count))
        .c(object_type("Model", count))
        .c(object_type("Material", 1));
    if has_tex {
        definitions = definitions.c(object_type("Texture", 1)).c(object_type("Video", 1));
    }

    let creator = "pixelmesh 0.1";
    let top = vec![
        Node::new("FBXHeaderExtension")
            .c(Node::leaf("FBXHeaderVersion", Prop::I32(1003)))
            .c(Node::leaf("FBXVersion", Prop::I32(version)))
            .c(Node::leaf("EncryptionType", Prop::I32(0)))
            .c(Node::new("CreationTimeStamp")
                .c(Node::leaf("Version", Prop::I32(1000)))
                .c(Node::leaf("Year", Prop::I32(2026)))
                .c(Node::leaf("Month", Prop::I32(1)))
                .c(Node::leaf("Day", Prop::I32(1)))
                .c(Node::leaf("Hour", Prop::I32(0)))
                .c(Node::leaf("Minute", Prop::I32(0)))
                .c(Node::leaf("Second", Prop::I32(0)))
                .c(Node::leaf("Millisecond", Prop::I32(0))))
            .c(Node::leaf("Creator", s(creator))),
        // FileId / CreationTime / footer form a checksum triple the FBX SDK verifies;
        // this is the well-known constant set that Blender also writes.
        Node::leaf("FileId", Prop::Raw(vec![0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1])),
        Node::leaf("CreationTime", s("1970-01-01 10:00:00:000")),
        Node::leaf("Creator", s(creator)),
        Node::new("GlobalSettings")
            .c(Node::leaf("Version", Prop::I32(1000)))
            .c(Node::new("Properties70")
                // Y up, right-handed, 1 unit = 1 metre.
                .c(p70_int("UpAxis", 1))
                .c(p70_int("UpAxisSign", 1))
                .c(p70_int("FrontAxis", 2))
                .c(p70_int("FrontAxisSign", 1))
                .c(p70_int("CoordAxis", 0))
                .c(p70_int("CoordAxisSign", 1))
                .c(p70_int("OriginalUpAxis", -1))
                .c(p70_int("OriginalUpAxisSign", 1))
                .c(p70_double("UnitScaleFactor", 100.0))
                .c(p70_double("OriginalUnitScaleFactor", 100.0))),
        Node::new("Documents")
            .c(Node::leaf("Count", Prop::I32(1)))
            .c(Node::new("Document")
                .p(Prop::I64(1_000_000))
                .p(s("Scene"))
                .p(s("Scene"))
                .c(Node::leaf("RootNode", Prop::I64(0)))),
        Node::new("References"),
        definitions,
        objects_node,
        connections,
        Node::new("Takes").c(Node::leaf("Current", s(""))),
    ];

    let mut out = Vec::new();
    out.extend_from_slice(b"Kaydara FBX Binary  \x00\x1a\x00");
    out.extend_from_slice(&(version as u32).to_le_bytes());
    write_children(&top, true, false, &mut out);

    // Footer.
    out.extend_from_slice(&[0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e]);
    out.extend_from_slice(&[0; 4]);
    let pad = match (16 - out.len() % 16) % 16 {
        0 => 16,
        n => n,
    };
    out.extend(std::iter::repeat(0u8).take(pad));
    out.extend_from_slice(&(version as u32).to_le_bytes());
    out.extend_from_slice(&[0; 120]);
    out.extend_from_slice(&[0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b]);
    out
}

pub fn export_file(path: &Path, objects: &[FbxObject], opt: &FbxOptions) -> io::Result<()> {
    std::fs::write(path, export_bytes(objects, opt))
}
