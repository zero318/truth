use std::io;

use bstr::{BStr, BString, ByteSlice};
use indexmap::IndexMap;

use crate::ast;
use crate::binary_io::{bail, BinRead, BinWrite, ReadResult, WriteResult};
use crate::error::{CompileError, SimpleError};
use crate::game::Game;
use crate::ident::Ident;
use crate::llir::{self, Instr, InstrFormat};
use crate::meta::{self, FromMeta, FromMetaError, Meta, ToMeta};
use crate::pos::Sp;
use crate::type_system::TypeSystem;
use crate::passes::DecompileKind;

// =============================================================================

/// Game-independent representation of a STD file.
#[derive(Debug, Clone, PartialEq)]
pub struct StdFile {
    pub unknown: u32,
    pub objects: IndexMap<Sp<Ident>, Object>,
    pub instances: Vec<Instance>,
    pub script: Vec<Instr>,
    pub extra: StdExtra,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StdExtra {
    Th06 {
        stage_name: BString,
        bgm: [Std06Bgm; 4],
    },
    Th10 {
        anm_path: BString,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Std06Bgm {
    pub path: BString,
    pub name: BString,
}

impl FromMeta for Std06Bgm {
    fn from_meta(meta: &Sp<Meta>) -> Result<Self, FromMetaError<'_>> {
        meta.parse_object(|m| Ok(Std06Bgm {
            path: m.expect_field("path")?,
            name: m.expect_field("name")?,
        }))
    }
}

impl ToMeta for Std06Bgm {
    fn to_meta(&self) -> Meta {
        Meta::make_object()
            .field("path", &self.path)
            .field("name", &self.name)
            .build()
    }
}

impl StdFile {
    pub fn decompile_to_ast(&self, game: Game, ty_ctx: &TypeSystem, decompile_kind: DecompileKind) -> Result<ast::Script, SimpleError> {
        decompile_std(&*game_format(game), self, ty_ctx, decompile_kind)
    }

    pub fn compile_from_ast(game: Game, script: &ast::Script, ty_ctx: &mut TypeSystem) -> Result<Self, CompileError> {
        compile_std(&*game_format(game), script, ty_ctx)
    }

    pub fn write_to_stream(&self, mut w: impl io::Write + io::Seek, game: Game) -> WriteResult {
        write_std(&mut w, &*game_format(game), self)
    }

    pub fn read_from_bytes(game: Game, bytes: &[u8]) -> ReadResult<Self> {
        read_std(&*game_format(game), bytes)
    }
}

impl StdFile {
    fn init_from_meta<'m>(file_format: &dyn FileFormat, fields: &'m Sp<meta::Fields>) -> Result<Self, FromMetaError<'m>> {
        let mut m = meta::ParseObject::new(fields);
        let out = StdFile {
            unknown: m.expect_field("unknown")?,
            objects: m.expect_field("objects")?,
            instances: m.expect_field("instances")?,
            script: vec![],
            extra: file_format.extra_from_meta(&mut m)?,
        };
        m.finish()?;
        Ok(out)
    }

    fn make_meta(&self, file_format: &dyn FileFormat) -> meta::Fields {
        Meta::make_object()
            .field("unknown", &self.unknown)
            .with_mut(|b| file_format.extra_to_meta(&self.extra, b))
            .field("objects", &self.objects)
            .field("instances", &self.instances)
            .build_fields()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub unknown: u16,
    pub pos: [f32; 3],
    pub size: [f32; 3],
    pub quads: Vec<Quad>,
}

impl FromMeta for Object {
    fn from_meta(meta: &Sp<Meta>) -> Result<Self, FromMetaError<'_>> {
        meta.parse_object(|m| Ok(Object {
            unknown: m.expect_field::<i32>("unknown")? as u16,
            pos: m.expect_field("pos")?,
            size: m.expect_field("size")?,
            quads: m.expect_field("quads")?,
        }))
    }
}

impl ToMeta for Object {
    fn to_meta(&self) -> Meta {
        Meta::make_object()
            .field("unknown", &(self.unknown as i32))
            .field("pos", &self.pos)
            .field("size", &self.size)
            .field("quads", &self.quads)
            .build()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub anm_script: u16,
    pub extra: QuadExtra,
}

#[derive(Debug, Clone, PartialEq)]
pub enum QuadExtra {
    /// Type 0 quad.
    Rect {
        pos: [f32; 3],
        size: [f32; 2],
    },
    /// Type 1 quad. Only available in IN and PoFV.
    Strip {
        start: [f32; 3],
        end: [f32; 3],
        width: f32,
    }
}

impl FromMeta for Quad {
    fn from_meta(meta: &Sp<Meta>) -> Result<Self, FromMetaError<'_>> {
        meta.parse_variant()?
            .variant("rect", |m| Ok(Quad {
                anm_script: m.expect_field::<i32>("anm_script")? as u16,
                extra: QuadExtra::Rect {
                    pos: m.expect_field("pos")?,
                    size: m.expect_field("size")?,
                },
            }))
            .variant("strip", |m| Ok(Quad {
                anm_script: m.expect_field::<i32>("anm_script")? as u16,
                extra: QuadExtra::Strip {
                    start: m.expect_field("start")?,
                    end: m.expect_field("end")?,
                    width: m.expect_field("width")?,
                },
            }))
            .finish()
    }
}

impl ToMeta for Quad {
    fn to_meta(&self) -> Meta {
        let variant = match self.extra {
            QuadExtra::Rect { .. } => "rect",
            QuadExtra::Strip { .. } => "strip",
        };

        Meta::make_variant(variant)
            .field("anm_script", &(self.anm_script as i32))
            .with_mut(|b| match &self.extra {
                QuadExtra::Rect { pos, size } => {
                    b.field("pos", pos);
                    b.field("size", size);
                },
                QuadExtra::Strip { start, end, width } => {
                    b.field("start", start);
                    b.field("end", end);
                    b.field("width", width);
                },
            })
            .build()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub object: Sp<Ident>,
    pub unknown: u16,
    pub pos: [f32; 3],
}

impl FromMeta for Instance {
    fn from_meta(meta: &Sp<Meta>) -> Result<Self, FromMetaError<'_>> {
        meta.parse_any_variant(|ident, meta| Ok(Instance {
            object: ident.clone(),
            unknown: meta.get_field::<i32>("unknown")?.unwrap_or(256) as u16,
            pos: meta.expect_field("pos")?,
        }))
    }
}

impl ToMeta for Instance {
    fn to_meta(&self) -> Meta {
        Meta::make_variant(&self.object)
            .field_default("unknown", &(self.unknown as i32), &256)
            .field("pos", &self.pos)
            .build()
    }
}

// =============================================================================

fn decompile_std(format: &dyn FileFormat, std: &StdFile, ty_ctx: &TypeSystem, decompile_kind: DecompileKind) -> Result<ast::Script, SimpleError> {
    let instr_format = format.instr_format();
    let script = &std.script;

    let code = llir::raise_instrs_to_sub_ast(instr_format, script, &ty_ctx.regs_and_instrs)?;

    let mut script = ast::Script {
        mapfiles: ty_ctx.regs_and_instrs.mapfiles_to_ast(),
        items: vec! [
            sp!(ast::Item::Meta {
                keyword: sp!(ast::MetaKeyword::Meta),
                name: None,
                fields: sp!(std.make_meta(format)),
            }),
            sp!(ast::Item::AnmScript {
                number: None,
                name: sp!("main".parse().unwrap()),
                code: ast::Block(code),
            }),
        ],
    };
    crate::passes::postprocess_decompiled(&mut script, ty_ctx, decompile_kind)?;
    Ok(script)
}

fn unsupported(span: &crate::pos::Span) -> CompileError {
    error!(
        message("feature not supported by format"),
        primary(span, "not supported by STD files"),
    )
}

fn compile_std(
    format: &dyn FileFormat,
    script: &ast::Script,
    ty_ctx: &mut TypeSystem,
) -> Result<StdFile, CompileError> {
    use ast::Item;

    let script = {
        use ast::VisitMut;

        let mut script = script.clone();

        let mut visitor = crate::passes::const_simplify::Visitor::new();
        visitor.visit_script(&mut script);
        visitor.finish()?;

        ty_ctx.resolve_names(&mut script)?;

        let mut visitor = crate::passes::compile_loop::Visitor::new(ty_ctx);
        visitor.visit_script(&mut script);
        visitor.finish()?;

        script
    };

    let (meta, main_sub) = {
        let (mut found_meta, mut found_main_sub) = (None, None);
        for item in script.items.iter() {
            match &item.value {
                Item::Meta { keyword: Sp { span: kw_span, value: ast::MetaKeyword::Meta }, name: None, fields: meta } => {
                    if let Some((prev_kw_span, _)) = found_meta.replace((kw_span, meta)) {
                        return Err(error!(
                            message("'meta' supplied multiple times"),
                            primary(kw_span, "duplicate 'meta'"),
                            secondary(prev_kw_span, "previously supplied here"),
                        ));
                    }
                },
                Item::Meta { keyword: Sp { value: ast::MetaKeyword::Meta, .. }, name: Some(name), .. } => return Err(error!(
                    message("unexpected named meta '{}' in STD file", name),
                    primary(name, "unexpected name"),
                )),
                Item::Meta { keyword, .. } => return Err(error!(
                    message("unexpected '{}' in STD file", keyword),
                    primary(keyword, "not valid in STD files"),
                )),
                Item::AnmScript { number: Some(number), .. } => return Err(error!(
                    message("unexpected numbered script in STD file"),
                    primary(number, "unexpected number"),
                )),
                Item::AnmScript { number: None, name, code } => {
                    if name != "main" {
                        return Err(error!(
                            message("STD script must be called 'main'"),
                            primary(name, "invalid name for STD script"),
                        ));
                    }
                    if let Some((prev_item, _)) = found_main_sub.replace((item, code)) {
                        return Err(error!(
                            message("redefinition of 'main' script"),
                            primary(item, "this defines a script called 'main'..."),
                            secondary(prev_item, "...but 'main' was already defined here"),
                        ));
                    }
                },
                Item::FileList { .. } => return Err(unsupported(&item.span)),
                Item::Func { .. } => return Err(unsupported(&item.span)),
            }
        }
        match (found_meta, found_main_sub) {
            (Some((_, meta)), Some((_, main))) => (meta, main),
            (None, _) => return Err(error!(message("missing 'main' sub"))),
            (Some(_), None) => return Err(error!(message("missing 'meta' section"))),
        }
    };

    let mut out = StdFile::init_from_meta(format, meta)?;
    out.script = crate::llir::lower_sub_ast_to_instrs(format.instr_format(), &main_sub.0, ty_ctx)?;
    Ok(out)
}

// =============================================================================

fn read_std(format: &dyn FileFormat, bytes: &[u8]) -> ReadResult<StdFile> {
    let mut f = bytes;

    let num_objects = f.read_u16()? as usize;
    let num_quads = f.read_u16()? as usize;
    let instances_offset = f.read_u32()? as usize;
    let script_offset = f.read_u32()? as usize;
    let unknown = f.read_u32()?;
    let extra = format.read_extra(&mut f)?;

    let object_offsets = (0..num_objects).map(|_| f.read_u32()).collect::<ReadResult<Vec<_>>>()?;
    let objects = (0..num_objects)
        .map(|i| {
            let key = sp!(format!("object{}", i).parse::<Ident>().unwrap());
            let value = read_object(i, &mut &bytes[object_offsets[i] as usize..])?;
            Ok((key, value))
        }).collect::<ReadResult<IndexMap<_, _>>>()?;
    assert_eq!(num_quads, objects.values().map(|x| x.quads.len()).sum::<usize>());

    let instances = {
        let mut f = &bytes[instances_offset..];
        let mut vec = vec![];
        while let Some(instance) = read_instance(&mut f, &objects)? {
            vec.push(instance);
        }
        vec
    };

    let script = llir::read_instrs(&mut &bytes[script_offset..], format.instr_format(), 0, None)?;

    Ok(StdFile { unknown, extra, objects, instances, script })
}

fn write_std(f: &mut dyn BinWrite, format: &dyn FileFormat, std: &StdFile) -> WriteResult {
    let start_pos = f.pos()?;

    f.write_u16(std.objects.len() as u16)?;
    f.write_u16(std.objects.values().map(|x| x.quads.len()).sum::<usize>() as u16)?;

    let instances_offset_pos = f.pos()?;
    f.write_u32(0)?;
    let script_offset_pos = f.pos()?;
    f.write_u32(0)?;

    f.write_u32(std.unknown)?;

    format.write_extra(f, &std.extra)?;

    let object_offsets_pos = f.pos()?;
    for _ in &std.objects {
        f.write_u32(0)?;
    }

    let mut object_offsets = vec![];
    for (object_id, object) in std.objects.values().enumerate() {
        object_offsets.push(f.pos()? - start_pos);
        write_object(f, &*format, object_id, object)?;
    }

    let instances_offset = f.pos()? - start_pos;
    for instance in &std.instances {
        write_instance(f, instance, &std.objects)?;
    }
    write_terminal_instance(f)?;

    let instr_format = format.instr_format();

    let script_offset = f.pos()? - start_pos;
    llir::write_instrs(f, instr_format, &std.script)?;

    let end_pos = f.pos()?;
    f.seek_to(instances_offset_pos)?;
    f.write_u32(instances_offset as u32)?;
    f.seek_to(script_offset_pos)?;
    f.write_u32(script_offset as u32)?;
    f.seek_to(object_offsets_pos)?;
    for offset in object_offsets {
        f.write_u32(offset as u32)?;
    }
    f.seek_to(end_pos)?;
    Ok(())
}

fn read_string_128(f: &mut dyn BinRead) -> ReadResult<BString> {
    let mut out = [0u8; 128];
    f.read_exact(&mut out)?;
    
    let mut out = out.as_bstr().to_owned();
    while let Some(0) = out.last() {
        out.pop();
    }
    Ok(out)
}
fn write_string_128(f: &mut dyn BinWrite, s: &BStr) -> WriteResult {
    let mut buf = [0u8; 128];
    if s.len() >= 128 {
        bail!("string too long (max 127 bytes): {:?}", s);
    }

    buf[..s.len()].copy_from_slice(&s[..]);
    f.write_all(&mut buf)?;
    Ok(())
}

fn read_object(expected_id: usize, bytes: &mut dyn BinRead) -> ReadResult<Object> {
    let mut f = bytes;
    let id = f.read_u16()?;
    if id as usize != expected_id {
        fast_warning!("object has non-sequential id (expected {}, got {})", expected_id, id);
    }

    let unknown = f.read_u16()?;
    let pos = f.read_f32s_3()?;
    let size = f.read_f32s_3()?;
    let mut quads = vec![];
    while let Some(quad) = read_quad(&mut f)? {
        quads.push(quad);
    }
    Ok(Object { unknown, pos, size, quads })
}

fn write_object(f: &mut dyn BinWrite, format: &dyn FileFormat, id: usize, x: &Object) -> WriteResult {
    f.write_u16(id as u16)?;
    f.write_u16(x.unknown)?;
    f.write_f32s(&x.pos)?;
    f.write_f32s(&x.size)?;
    for quad in &x.quads {
        write_quad(f, format, quad)?;
    }
    write_terminal_quad(f)
}

fn read_quad(f: &mut dyn BinRead) -> ReadResult<Option<Quad>> {
    let kind = f.read_i16()?;
    let size = f.read_u16()?;
    match (kind, size) {
        (-1, 4) => return Ok(None), // no more quads
        (0, 0x1c) => false,
        (1, 0x24) => true,
        (-1, _) | (0, _) | (1, _) => {
            bail!("unexpected size for type {} quad: {:#x}", kind, size);
        },
        _ => bail!("unknown quad type: {}", kind),
    };

    let anm_script = f.read_u16()?;
    match f.read_u16()? {
        0 => {},  // This word is zero in the file, and used to store an index in-game.
        s => bail!("unexpected data in quad index field: {:#04x}", s),
    };

    Ok(Some(Quad {
        anm_script,
        extra: match kind {
            0 => QuadExtra::Rect {
                pos: f.read_f32s_3()?,
                size: f.read_f32s_2()?,
            },
            1 => QuadExtra::Strip {
                start: f.read_f32s_3()?,
                end: f.read_f32s_3()?,
                width: f.read_f32()?,
            },
            _ => unreachable!(),
        },
    }))
}

fn write_quad(f: &mut dyn BinWrite, format: &dyn FileFormat, quad: &Quad) -> WriteResult {
    let (kind, size) = match quad.extra {
        QuadExtra::Rect { .. } => (0, 0x1c),
        QuadExtra::Strip { .. } => (1, 0x24),
    };
    f.write_i16(kind)?;
    f.write_u16(size)?;
    f.write_u16(quad.anm_script)?;
    f.write_u16(0)?;
    match quad.extra {
        QuadExtra::Rect { pos, size } => {
            f.write_f32s(&pos)?;
            f.write_f32s(&size)?;
        },
        QuadExtra::Strip { start, end, width } => {
            if !format.has_strips() {
                // FIXME: Could be better with a span, maybe check earlier
                fast_warning!("'strip' quads can only be used in TH08 and TH09!")
            }
            f.write_f32s(&start)?;
            f.write_f32s(&end)?;
            f.write_f32(width)?;
        },
    }
    Ok(())
}
fn write_terminal_quad(f: &mut dyn BinWrite) -> WriteResult {
    f.write_i16(-1)?;
    f.write_u16(0x4)?; // size
    Ok(())
}


fn read_instance(f: &mut dyn BinRead, objects: &IndexMap<Sp<Ident>, Object>) -> ReadResult<Option<Instance>> {
    let object_id = f.read_u16()?;
    let unknown = f.read_u16()?;
    if object_id == 0xffff {
        return Ok(None);
    }
    let object = match objects.get_index(object_id as usize) {
        Some((ident, _)) => ident.clone(),
        None => bail!("object index too large! ({}, but there are only {} objects)", object_id, objects.len()),
    };
    let pos = f.read_f32s_3()?;
    Ok(Some(Instance { object, unknown, pos }))
}

fn write_instance(f: &mut dyn BinWrite, inst: &Instance, objects: &IndexMap<Sp<Ident>, Object>) -> WriteResult {
    match objects.get_index_of(&inst.object) {
        Some(object_index) => f.write_u16(object_index as u16)?,
        // FIXME: This should be a diagnostic. Stop using io::Result noob
        None => bail!("No object named {}", &inst.object),
    }
    f.write_u16(inst.unknown)?;
    f.write_f32s(&inst.pos)?;
    Ok(())
}
fn write_terminal_instance(f: &mut dyn BinWrite) -> WriteResult {
    for _ in 0..4 {
        f.write_i32(-1)?;
    }
    Ok(())
}

fn game_format(game: Game) -> Box<dyn FileFormat> {
    if Game::Th095 <= game {
        let instr_format = InstrFormat10 { game };
        Box::new(FileFormat10 { instr_format })
    } else {
        let has_strips = match game {
            Game::Th06 | Game::Th07 => false,
            Game::Th08 | Game::Th09 => true,
            _ => unreachable!(),
        };

        let instr_format = InstrFormat06 { game };
        Box::new(FileFormat06 { has_strips, instr_format })
    }
}

// =============================================================================

/// STD format, EoSD to PoFV.
struct FileFormat06 {
    has_strips: bool,
    instr_format: InstrFormat06,
}
/// STD format, StB to present.
struct FileFormat10 {
    instr_format: InstrFormat10,
}

trait FileFormat {
    fn extra_from_meta<'m>(&self, meta: &mut meta::ParseObject<'m>) -> Result<StdExtra, FromMetaError<'m>>;
    fn extra_to_meta(&self, extra: &StdExtra, b: &mut meta::BuildObject);
    fn read_extra(&self, f: &mut dyn BinRead) -> ReadResult<StdExtra>;
    fn write_extra(&self, f: &mut dyn BinWrite, x: &StdExtra) -> WriteResult;
    fn instr_format(&self) -> &dyn InstrFormat;
    fn has_strips(&self) -> bool;
}

impl FileFormat for FileFormat06 {
    fn extra_from_meta<'m>(&self, m: &mut meta::ParseObject<'m>) -> Result<StdExtra, FromMetaError<'m>> {
        Ok(StdExtra::Th06 {
            stage_name: m.expect_field("stage_name")?,
            bgm: m.expect_field("bgm")?,
        })
    }

    fn extra_to_meta(&self, extra: &StdExtra, b: &mut meta::BuildObject) {
        match extra {
            StdExtra::Th10 { .. } => unreachable!(),
            StdExtra::Th06 { stage_name, bgm } => {
                b.field("stage_name", stage_name);
                b.field("bgm", bgm);
            },
        }
    }

    fn read_extra(&self, f: &mut dyn BinRead) -> ReadResult<StdExtra> {
        let stage_name = read_string_128(f)?;
        let bgm_names = (0..4).map(|_| read_string_128(f)).collect::<Result<Vec<_>, _>>()?;
        let bgm_paths = (0..4).map(|_| read_string_128(f)).collect::<Result<Vec<_>, _>>()?;
        let mut bgms = bgm_names.into_iter().zip(bgm_paths).map(|(name, path)| Std06Bgm { name, path });
        Ok(StdExtra::Th06 {
            stage_name,
            bgm: [bgms.next().unwrap(), bgms.next().unwrap(), bgms.next().unwrap(), bgms.next().unwrap()],
        })
    }

    fn write_extra(&self, f: &mut dyn BinWrite, x: &StdExtra) -> WriteResult {
        match x {
            StdExtra::Th06 { stage_name, bgm } => {
                write_string_128(f, stage_name.as_ref())?;
                let bgm_names = bgm.iter().map(|Std06Bgm { name, .. }| name);
                let bgm_paths = bgm.iter().map(|Std06Bgm { path, .. }| path);
                for s in bgm_names.chain(bgm_paths) {
                    write_string_128(f, s.as_ref())?;
                }
            },
            StdExtra::Th10 { .. } => unreachable!(),
        };
        Ok(())
    }

    fn instr_format(&self) -> &dyn InstrFormat { &self.instr_format }
    fn has_strips(&self) -> bool { self.has_strips }
}

impl FileFormat for FileFormat10 {
    fn extra_from_meta<'m>(&self, m: &mut meta::ParseObject<'m>) -> Result<StdExtra, FromMetaError<'m>> {
        Ok(StdExtra::Th10 {
            anm_path: m.expect_field("anm_path")?,
        })
    }

    fn extra_to_meta(&self, extra: &StdExtra, b: &mut meta::BuildObject) {
        match extra {
            StdExtra::Th10 { anm_path } => { b.field("anm_path", anm_path); },
            StdExtra::Th06 { .. } => unreachable!(),
        }
    }

    fn read_extra(&self, f: &mut dyn BinRead) -> ReadResult<StdExtra> {
        Ok(StdExtra::Th10 { anm_path: read_string_128(f)? })
    }

    fn write_extra(&self, f: &mut dyn BinWrite, x: &StdExtra) -> WriteResult {
        match x {
            StdExtra::Th10 { anm_path } => write_string_128(f, anm_path.as_ref())?,
            StdExtra::Th06 { .. } => unreachable!(),
        };
        Ok(())
    }

    fn instr_format(&self) -> &dyn InstrFormat { &self.instr_format }
    fn has_strips(&self) -> bool { false }
}

pub struct InstrFormat06 { game: Game }
pub struct InstrFormat10 { game: Game }
impl InstrFormat10 {
    const HEADER_SIZE: usize = 8;
}

impl InstrFormat for InstrFormat06 {
    fn read_instr(&self, f: &mut dyn BinRead) -> ReadResult<Option<Instr>> {
        let time = f.read_i32()?;
        let opcode = f.read_i16()?;
        let argsize = f.read_u16()?;
        if opcode == -1 {
            return Ok(None)
        }
        assert_eq!(argsize, 12);

        let args = llir::read_dword_args_upto_size(f, 12, 0)?;
        Ok(Some(Instr { time, opcode: opcode as u16, args }))
    }

    fn intrinsic_opcode_pairs(&self) -> Vec<(llir::IntrinsicInstrKind, u16)> {
        if Game::Th07 <= self.game && self.game <= Game::Th09 {
            vec![
                (llir::IntrinsicInstrKind::Jmp, 4),
                (llir::IntrinsicInstrKind::InterruptLabel, 31),
            ]
        } else {
            vec![]  // lul
        }
    }

    fn write_instr(&self, f: &mut dyn BinWrite, instr: &Instr) -> WriteResult {
        f.write_i32(instr.time)?;
        f.write_u16(instr.opcode)?;
        f.write_u16(12)?;  // this version writes argsize rather than instr size
        for arg in &instr.args {
            f.write_u32(arg.expect_raw().bits)?;
        }
        for _ in instr.args.len()..3 {
            f.write_u32(0)?;  // padding args
        }
        Ok(())
    }

    fn write_terminal_instr(&self, f: &mut dyn BinWrite) -> WriteResult {
        for _ in 0..5 {
            f.write_i32(-1)?;
        }
        Ok(())
    }

    fn instr_size(&self, _instr: &Instr) -> usize { 20 }

    fn encode_label(&self, offset: usize) -> u32 {
        assert_eq!(offset % 20, 0);
        (offset / 20) as u32
    }
    fn decode_label(&self, bits: u32) -> usize {
        (bits * 20) as usize
    }
}

impl InstrFormat for InstrFormat10 {
    fn read_instr(&self, f: &mut dyn BinRead) -> ReadResult<Option<Instr>> {
        let time = f.read_i32()?;
        let opcode = f.read_i16()?;
        let size = f.read_u16()? as usize;
        if opcode == -1 {
            return Ok(None)
        }

        let args = llir::read_dword_args_upto_size(f, size - Self::HEADER_SIZE, 0)?;
        Ok(Some(Instr { time, opcode: opcode as u16, args }))
    }

    fn intrinsic_opcode_pairs(&self) -> Vec<(llir::IntrinsicInstrKind, u16)> {
        let mut out = vec![(llir::IntrinsicInstrKind::Jmp, 1)];

        // TH095 and TH10 are missing this
        if Game::Th11 <= self.game {
            out.push((llir::IntrinsicInstrKind::InterruptLabel, 16));
        }
        out
    }

    fn write_instr(&self, f: &mut dyn BinWrite, instr: &Instr) -> WriteResult {
        f.write_i32(instr.time)?;
        f.write_u16(instr.opcode)?;
        f.write_u16(self.instr_size(instr) as u16)?;
        for x in &instr.args {
            f.write_u32(x.expect_raw().bits)?;
        }
        Ok(())
    }

    fn write_terminal_instr(&self, f: &mut dyn BinWrite) -> WriteResult {
        for _ in 0..5 {
            f.write_i32(-1)?;
        }
        Ok(())
    }

    fn instr_size(&self, instr: &Instr) -> usize { Self::HEADER_SIZE + 4 * instr.args.len() }
}
