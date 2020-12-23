use std::io::{self, Read, Cursor, Write, Seek};

use byteorder::{LittleEndian as Le, ReadBytesExt, WriteBytesExt};
use bstr::{BStr, BString, ByteSlice};
use crate::error::Diagnostic;

use crate::CompileError;
use crate::pos::{Spanned, FileId};
use crate::ast::{self, Expr};
use crate::ident::Ident;
use crate::signature::Functions;
use crate::meta::{ToMeta, FromMeta, Meta, FromMetaError};

// =============================================================================

/// Game-independent representation of a STD file.
#[derive(Debug, Clone, PartialEq)]
pub struct StdFile {
    pub unknown: u32,
    pub entries: Vec<Entry>,
    pub instances: Vec<Instance>,
    pub script: Vec<Instr>,
    pub extra: StdExtra,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StdExtra {
    Th06 {
        bgm_names: [BString; 5],
        bgm_paths: [BString; 5],
    },
    Th10 {
        anm_path: BString,
    },
}

impl StdFile {
    pub fn decompile(&self, functions: &Functions) -> ast::Script {
        _decompile_std(self, functions)
    }

    pub fn compile(file_id: FileId, script: &ast::Script, functions: &Functions) -> Result<Self, CompileError> {
        _compile_std(file_id, script, functions)
    }
}

impl FromMeta for StdFile {
    fn from_meta(meta: &Meta) -> Result<Self, FromMetaError<'_>> {
        Ok(StdFile {
            unknown: meta.expect_field("unknown")?,
            entries: meta.expect_field("objects")?,
            instances: meta.expect_field("instances")?,
            script: vec![],
            extra: StdExtra::Th10 {
                anm_path: meta.expect_field("anm_file")?,
            },
        })
    }
}

impl ToMeta for StdFile {
    fn to_meta(&self) -> Meta {
        let anm_path = match &self.extra {
            StdExtra::Th10 { anm_path } => anm_path,
            StdExtra::Th06 { .. } => unimplemented!(),
        };
        Meta::make_object()
            .field("unknown", &self.unknown)
            .field("anm_file", &anm_path)
            .field("objects", &self.entries)
            .field("instances", &self.instances)
            .build()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: u16,
    pub unknown: u16,
    pub pos: [f32; 3],
    pub size: [f32; 3],
    pub quads: Vec<Quad>,
}

impl FromMeta for Entry {
    fn from_meta(meta: &Meta) -> Result<Self, FromMetaError<'_>> {
        Ok(Entry {
            id: meta.expect_field::<i32>("id")? as u16,
            unknown: meta.expect_field::<i32>("unknown")? as u16,
            pos: meta.expect_field("pos")?,
            size: meta.expect_field("size")?,
            quads: meta.expect_field("quads")?,
        })
    }
}

impl ToMeta for Entry {
    fn to_meta(&self) -> Meta {
        Meta::make_object()
            .field("id", &(self.id as i32))
            .field("unknown", &(self.unknown as i32))
            .field("pos", &self.pos)
            .field("size", &self.size)
            .field("quads", &self.quads)
            .build()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub unknown: u16,
    pub anm_script: u16,
    pub pos: [f32; 3],
    pub size: [f32; 2],
}

impl FromMeta for Quad {
    fn from_meta(meta: &Meta) -> Result<Self, FromMetaError<'_>> {
        Ok(Quad {
            unknown: meta.get_field::<i32>("unknown")?.unwrap_or(0) as u16,
            anm_script: meta.expect_field::<i32>("anm_script")? as u16,
            pos: meta.expect_field("pos")?,
            size: meta.expect_field("size")?,
        })
    }
}

impl ToMeta for Quad {
    fn to_meta(&self) -> Meta {
        Meta::make_object()
            .field_default("unknown", &(self.unknown as i32), &0)
            .field("anm_script", &(self.anm_script as i32))
            .field("pos", &self.pos)
            .field("size", &self.size)
            .build()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub object_id: u16,
    pub unknown: u16,
    pub pos: [f32; 3],
}

impl FromMeta for Instance {
    fn from_meta(meta: &Meta) -> Result<Self, FromMetaError<'_>> {
        Ok(Instance {
            object_id: meta.expect_field::<i32>("object")? as u16,
            unknown: meta.get_field::<i32>("unknown")?.unwrap_or(256) as u16,
            pos: meta.expect_field("pos")?,
        })
    }
}

impl ToMeta for Instance {
    fn to_meta(&self) -> Meta {
        Meta::make_object()
            .field("object", &(self.object_id as i32))
            .field_default("unknown", &(self.unknown as i32), &256)
            .field("pos", &self.pos)
            .build()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instr {
    pub time: i32,
    pub opcode: u16,
    pub args: Vec<u32>,
}

// =============================================================================

fn _decompile_std(std: &StdFile, functions: &Functions) -> ast::Script {
    use crate::signature::ArgEncoding;

    let code = std.script.iter().map(|instr| {
        let Instr { time, opcode, args } = instr;
        let ins_ident = {
            functions.opcode_names.get(&(*opcode as u32)).cloned()
                .unwrap_or_else(|| Ident::new_ins(*opcode as u32))
        };
        let args = match functions.ins_signature(&ins_ident) {
            Some(siggy) => {
                let encodings = siggy.arg_encodings();
                assert_eq!(encodings.len(), args.len()); // FIXME: return Error
                encodings.iter().zip(args).map(|(enc, &arg)| match enc {
                    ArgEncoding::Dword => <Spanned<Expr>>::new(arg as i32),
                    ArgEncoding::Color => Spanned::new(Expr::LitInt {
                        value: arg as i32,
                        hex: true,
                    }),
                    ArgEncoding::Float => <Spanned<Expr>>::new(f32::from_bits(arg)),
                }).collect()
            },
            None => args.iter().map(|&x| <Spanned<Expr>>::new(x as i32)).collect(),
        };
        Spanned::from(ast::Stmt {
            time: *time,
            labels: vec![],
            body: Spanned::from(ast::StmtBody::Expr(Spanned::from(Expr::Call { func: ins_ident, args }))),
        })
    }).collect();

    ast::Script {
        items: vec! [
            Spanned::from(ast::Item::Meta {
                keyword: ast::MetaKeyword::Meta,
                name: None,
                meta: std.to_meta(),
            }),
            Spanned::from(ast::Item::AnmScript {
                number: None,
                name: "main".parse().unwrap(),
                code: ast::Block(code),
            }),
        ],
    }
}

fn _compile_std(file_id: FileId, script: &ast::Script, functions: &Functions) -> Result<StdFile, CompileError> {
    use ast::Item;

    let (meta, main_sub) = {
        let (mut found_meta, mut found_main_sub) = (None, None);
        for item in script.items.iter() {
            match &item.value {
                Item::Meta { keyword: ast::MetaKeyword::Meta, name: None, meta } => {
                    if let Some(_) = found_meta.replace(meta) {
                        bail_nospan!("multiple 'meta's");
                    }
                    found_meta = Some(meta);
                },
                Item::Meta { keyword: ast::MetaKeyword::Meta, name: Some(name), .. } => bail_nospan!("unexpected named meta '{}' in STD file", name),
                Item::Meta { keyword, .. } => bail_nospan!("unexpected '{}' in STD file", keyword),
                Item::AnmScript { number: Some(_), .. } => bail_nospan!("unexpected numbered script in STD file"),
                Item::AnmScript { number: None, name, code } => {
                    if name != "main" {
                        bail_nospan!("STD entry point must be called 'main'");
                    }
                    if let Some(_) = found_main_sub {
                        bail_nospan!("multiple 'main' scripts");
                    }
                    found_main_sub = Some(code);
                },
                Item::FileList { .. } => bail_nospan!("unexpected file list in STD file"),
                Item::Func { .. } => bail_nospan!("unexpected function def in STD file"),
            }
        }
        match (found_meta, found_main_sub) {
            (Some(meta), Some(main)) => (meta, main),
            (None, _) => bail_nospan!("missing 'main' sub"),
            (Some(_), None) => bail_nospan!("missing 'meta' section"),
        }
    };

    let mut out = StdFile::from_meta(meta).map_err(|e| CompileError(Diagnostic::error().with_message(format!("{}", e))))?;
    out.script = _compile_main(file_id, &main_sub.0, functions)?;

    Ok(out)
}

fn _compile_main(file_id: FileId, code: &[Spanned<ast::Stmt>], functions: &Functions) -> Result<Vec<Instr>, CompileError> {
    use crate::signature::ArgEncoding;

    let mut out = vec![];
    for stmt in code {
        match &stmt.body.value {
            ast::StmtBody::Expr(e) => match &e.value {
                ast::Expr::Call { func, args } => {
                    let siggy = match functions.ins_signature(func) {
                        Some(siggy) => siggy,
                        None => bail_span!(file_id, stmt, "signature of {} is not known", func),
                    };
                    let opcode = match functions.resolve_aliases(func).as_ins() {
                        Some(opcode) => opcode,
                        None => bail_span!(file_id, stmt, "don't know how to compile function {} (not an instruction)", func),
                    };
                    let encodings = siggy.arg_encodings();
                    if encodings.len() != args.len() {
                        bail_span!(file_id, stmt, "wrong number of arguments (expected {}, got {})", encodings.len(), args.len())
                    }

                    out.push(Instr {
                        time: stmt.time,
                        opcode: opcode as _,
                        args: encodings.iter().zip(args).enumerate().map(|(index, (enc, arg))| match enc {
                            ArgEncoding::Dword |
                            ArgEncoding::Color => match **arg {
                                ast::Expr::LitInt { value, .. } => Ok(value as u32),
                                ast::Expr::LitFloat { .. } |
                                ast::Expr::LitString { .. } => bail_span!(file_id, arg, "expected an int for arg {} of {}", index+1, func),
                                _ => bail_span!(file_id, arg, "unsupported expression type in STD file"),
                            },
                            ArgEncoding::Float => match **arg {
                                ast::Expr::LitFloat { value, .. } => Ok(value.to_bits()),
                                ast::Expr::LitInt { .. } |
                                ast::Expr::LitString { .. } => bail_span!(file_id, arg, "expected a float for arg {} of {}", index+1, func),
                                _ => bail_span!(file_id, arg, "unsupported expression type in STD file"),
                            },
                        }).collect::<Result<_, _>>()?,
                    });
                },
                _ => bail_span!(file_id, stmt, "unsupported expression type in STD file"), // FIXME: give location info
            },
            _ => bail_span!(file_id, stmt, "unsupported statement type in STD file"), // FIXME: give location info
        }
    }
    Ok(out)
}

// =============================================================================

// FIXME clean up API, return Result
pub fn read_std_10(bytes: &[u8]) -> StdFile {
    let mut f = Cursor::new(bytes);

    let num_entries = f.read_u16::<Le>().expect("incomplete header") as usize;
    let num_quads = f.read_u16::<Le>().expect("incomplete header") as usize;
    let instances_offset = f.read_u32::<Le>().expect("incomplete header") as usize;
    let script_offset = f.read_u32::<Le>().expect("incomplete header") as usize;
    let unknown = f.read_u32::<Le>().expect("incomplete header");
    let extra = read_extra(&mut f).expect("incomplete header");
    let entry_offsets = (0..num_entries).map(|_| f.read_u32::<Le>().expect("unexpected EOF")).collect::<Vec<_>>();
    let entries = (0..num_entries)
        .map(|i| read_entry(&bytes[entry_offsets[i] as usize..]))
        .collect::<Vec<_>>();
    assert_eq!(num_quads, entries.iter().map(|x| x.quads.len()).sum());
    let instances = {
        let mut f = Cursor::new(&bytes[instances_offset..]);
        let mut vec = vec![];
        while let Some(instance) = read_instance(&mut f) {
            vec.push(instance);
        }
        vec
    };
    let script = {
        let mut f = Cursor::new(&bytes[script_offset..]);
        let mut script = vec![];
        while let Some(instr) = read_instr(&mut f) {
            script.push(instr);
        }
        script
    };
    StdFile { unknown, extra, entries, instances, script }
}

pub fn write_std_10(f: &mut (impl Write + Seek), std: &StdFile) -> io::Result<()> {
    let start_pos = f.seek(io::SeekFrom::Current(0))?;

    f.write_u16::<Le>(std.entries.len() as u16)?;
    f.write_u16::<Le>(std.entries.iter().map(|x| x.quads.len()).sum::<usize>() as u16)?;

    let instances_offset_pos = f.seek(io::SeekFrom::Current(0))?;
    f.write_u32::<Le>(0)?;
    let script_offset_pos = f.seek(io::SeekFrom::Current(0))?;
    f.write_u32::<Le>(0)?;

    f.write_u32::<Le>(std.unknown)?;

    write_extra(f, &std.extra)?;

    let entry_offsets_pos = f.seek(io::SeekFrom::Current(0))?;
    for _ in &std.entries {
        f.write_u32::<Le>(0)?;
    }

    let mut entry_offsets = vec![];
    for entry in &std.entries {
        entry_offsets.push(f.seek(io::SeekFrom::Current(0))? - start_pos);
        write_entry(f, entry)?;
    }

    let instances_offset = f.seek(io::SeekFrom::Current(0))? - start_pos;
    for instance in &std.instances {
        write_instance(f, instance)?;
    }
    write_terminal_instance(f)?;

    let script_offset = f.seek(io::SeekFrom::Current(0))? - start_pos;
    for instr in &std.script {
        write_instr(f, instr)?;
    }
    write_terminal_instr(f)?;

    let end_pos = f.seek(io::SeekFrom::Current(0))?;
    f.seek(io::SeekFrom::Start(instances_offset_pos))?;
    f.write_u32::<Le>(instances_offset as u32)?;
    f.seek(io::SeekFrom::Start(script_offset_pos))?;
    f.write_u32::<Le>(script_offset as u32)?;
    f.seek(io::SeekFrom::Start(entry_offsets_pos))?;
    for offset in entry_offsets {
        f.write_u32::<Le>(offset as u32)?;
    }
    f.seek(io::SeekFrom::Start(end_pos))?;
    Ok(())
}

fn read_extra(f: &mut Cursor<&[u8]>) -> Option<StdExtra> {
    Some(StdExtra::Th10 { anm_path: read_string_128(f) })
}
fn write_extra(f: &mut impl Write, x: &StdExtra) -> io::Result<()> {
    match x {
        StdExtra::Th10 { anm_path } => write_string_128(f, anm_path.as_bstr())?,
        StdExtra::Th06 { .. } => unimplemented!(),
    };
    Ok(())
}

fn read_string_128(f: &mut Cursor<&[u8]>) -> BString {
    let mut out = [0u8; 128];
    f.read_exact(&mut out).expect("unexpected EOF");
    let mut out = out.as_bstr().to_owned();
    while let Some(0) = out.last() {
        out.pop();
    }
    out
}
fn write_string_128(f: &mut impl Write, s: &BStr) -> io::Result<()> {
    let mut buf = [0u8; 128];
    assert!(s.len() < 127, "string too long: {:?}", s);
    buf[..s.len()].copy_from_slice(&s[..]);
    f.write_all(&mut buf)
}
fn read_vec2(f: &mut Cursor<&[u8]>) -> Option<[f32; 2]> {
    Some([f.read_f32::<Le>().ok()?, f.read_f32::<Le>().ok()?])
}
fn read_vec3(f: &mut Cursor<&[u8]>) -> Option<[f32; 3]> {Some([
    f.read_f32::<Le>().ok()?,
    f.read_f32::<Le>().ok()?,
    f.read_f32::<Le>().ok()?,
])}

fn write_vec2(f: &mut impl Write, x: &[f32; 2]) -> io::Result<()> {
    f.write_f32::<Le>(x[0])?;
    f.write_f32::<Le>(x[1])?;
    Ok(())
}
fn write_vec3(f: &mut impl Write, x: &[f32; 3]) -> io::Result<()> {
    f.write_f32::<Le>(x[0])?;
    f.write_f32::<Le>(x[1])?;
    f.write_f32::<Le>(x[2])?;
    Ok(())
}


fn read_entry(bytes: &[u8]) -> Entry {
    let mut f = Cursor::new(bytes);
    let id = f.read_u16::<Le>().expect("unexpected EOF");
    let unknown = f.read_u16::<Le>().expect("unexpected EOF");
    let pos = read_vec3(&mut f).expect("unexpected EOF");
    let size = read_vec3(&mut f).expect("unexpected EOF");
    let mut quads = vec![];
    while let Some(quad) = read_quad(&mut f) {
        quads.push(quad);
    }
    Entry { id, unknown, pos, size, quads }
}

fn write_entry(f: &mut impl Write, x: &Entry) -> io::Result<()> {
    f.write_u16::<Le>(x.id)?;
    f.write_u16::<Le>(x.unknown)?;
    write_vec3(f, &x.pos)?;
    write_vec3(f, &x.size)?;
    for quad in &x.quads {
        write_quad(f, quad)?;
    }
    write_terminal_quad(f)
}

fn read_quad(f: &mut Cursor<&[u8]>) -> Option<Quad> {
    let unknown = f.read_u16::<Le>().expect("unexpected EOF");
    match f.read_u16::<Le>().expect("unexpected EOF") {
        0x1c => {},
        0x4 => { // End of stream
            assert_eq!(unknown, 0xffff);
            return None;
        },
        s => panic!("bad object size: {}", s),
    };

    let anm_script = f.read_u16::<Le>().expect("unexpected EOF");
    match f.read_u16::<Le>().expect("unexpected EOF") {
        0 => {},
        s => panic!("unexpected nonzero padding: {}", s),
    };

    let pos = read_vec3(f).expect("unexpected EOF");
    let size = read_vec2(f).expect("unexpected EOF");
    Some(Quad { unknown, anm_script, pos, size })
}

fn write_quad(f: &mut impl Write, quad: &Quad) -> io::Result<()> {
    f.write_u16::<Le>(quad.unknown)?;
    f.write_u16::<Le>(0x1c)?; // size
    f.write_u16::<Le>(quad.anm_script)?;
    f.write_u16::<Le>(0)?;
    write_vec3(f, &quad.pos)?;
    write_vec2(f, &quad.size)?;
    Ok(())
}
fn write_terminal_quad(f: &mut impl Write) -> io::Result<()> {
    f.write_i16::<Le>(-1)?;
    f.write_u16::<Le>(0x4)?; // size
    Ok(())
}


fn read_instance(f: &mut Cursor<&[u8]>) -> Option<Instance> {
    let object_id = f.read_u16::<Le>().expect("unexpected EOF");
    let unknown = f.read_u16::<Le>().expect("unexpected EOF");
    if object_id == 0xffff {
        return None;
    }
    let pos = read_vec3(f).expect("unexpected EOF");
    Some(Instance { object_id, unknown, pos })
}

fn write_instance(f: &mut impl Write, inst: &Instance) -> io::Result<()> {
    f.write_u16::<Le>(inst.object_id)?;
    f.write_u16::<Le>(inst.unknown)?;
    write_vec3(f, &inst.pos)?;
    Ok(())
}
fn write_terminal_instance(f: &mut impl Write) -> io::Result<()> {
    for _ in 0..4 {
        f.write_i32::<Le>(-1)?;
    }
    Ok(())
}

fn read_instr(f: &mut Cursor<&[u8]>) -> Option<Instr> {
    let time = f.read_i32::<Le>().expect("unexpected EOF");
    let opcode = f.read_i16::<Le>().expect("unexpected EOF");
    let size = f.read_u16::<Le>().expect("unexpected EOF");
    if opcode == -1 {
        return None
    }

    assert_eq!(size % 4, 0);
    let nargs = (size - 8)/4;
    let args = (0..nargs).map(|_| f.read_u32::<Le>().expect("unexpected EOF")).collect::<Vec<_>>();
    Some(Instr { time, opcode: opcode as u16, args })
}
fn write_instr(f: &mut impl Write, instr: &Instr) -> io::Result<()> {
    f.write_i32::<Le>(instr.time)?;
    f.write_u16::<Le>(instr.opcode)?;
    f.write_u16::<Le>(8 + 4 * instr.args.len() as u16)?;
    for &x in &instr.args {
        f.write_u32::<Le>(x)?;
    }
    Ok(())
}
fn write_terminal_instr(f: &mut impl Write) -> io::Result<()> {
    for _ in 0..5 {
        f.write_i32::<Le>(-1)?;
    }
    Ok(())
}
