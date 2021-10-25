use std::collections::{HashMap};

use crate::raw;
use crate::ast;
use crate::context;
use crate::error::{ErrorReported, GatherErrorIteratorExt};
use crate::llir::{InstrFormat, InstrAbi, ArgEncoding};
use crate::diagnostic::{Diagnostic, Emitter};
use crate::pos::{Span, Sp};
use crate::value::{ScalarType};

/// Maps opcodes to and from intrinsics.
#[derive(Debug, Default)] // Default is used by --no-intrinsics
pub struct IntrinsicInstrs {
    intrinsic_opcodes: HashMap<IntrinsicInstrKind, raw::Opcode>,
    intrinsic_abi_props: HashMap<IntrinsicInstrKind, IntrinsicInstrAbiProps>,
    opcode_intrinsics: HashMap<raw::Opcode, IntrinsicInstrKind>,
}

#[test]
fn fix_from_instr_format() {
    panic!("fix from_instr_format to add intrinsics from mapfiles");
}

#[test]
fn fix_null_span() {
    panic!("fix null span in IntrinsicInstrAbiProps::from_abi call (put spans on abis in Defs)");
}

impl IntrinsicInstrs {
    /// Build from the builtin intrinsics list of the format, and user mapfiles.
    ///
    /// This will perform verification of the signatures for each intrinsic.
    pub fn from_format_and_mapfiles(instr_format: &dyn InstrFormat, defs: &context::Defs, emitter: &dyn Emitter) -> Result<Self, ErrorReported> {
        let intrinsic_opcodes: HashMap<_, _> = instr_format.intrinsic_opcode_pairs().into_iter().collect();
        let opcode_intrinsics = intrinsic_opcodes.iter().map(|(&k, &v)| (v, k)).collect();

        let intrinsic_abi_props = {
            intrinsic_opcodes.iter()
                .map(|(&kind, &opcode)| {
                    let abi = defs.ins_abi(instr_format.language(), opcode)
                        .unwrap_or_else(|| unimplemented!("error when intrinsic has no ABI"));
                    let abi_props = IntrinsicInstrAbiProps::from_abi(kind, sp!(Span::NULL => abi))
                        .map_err(|e| emitter.as_sized().emit(e))?;
                    Ok((kind, abi_props))
                })
                .collect_with_recovery()?
        };
        Ok(IntrinsicInstrs { opcode_intrinsics, intrinsic_opcodes, intrinsic_abi_props })
    }

    pub(crate) fn get_opcode_and_abi_props(&self, intrinsic: IntrinsicInstrKind, span: Span, descr: &str) -> Result<(raw::Opcode, &IntrinsicInstrAbiProps), Diagnostic> {
        match self.intrinsic_opcodes.get(&intrinsic) {
            Some(&opcode) => Ok((opcode, &self.intrinsic_abi_props[&intrinsic])),
            None => Err(error!(
                message("feature not supported by format"),
                primary(span, "{} not supported in this game", descr),
            )),
        }
    }

    pub(crate) fn get_opcode_opt(&self, intrinsic: IntrinsicInstrKind) -> Option<raw::Opcode> {
        self.intrinsic_opcodes.get(&intrinsic).copied()
    }

    pub(crate) fn get_intrinsic_and_props(&self, opcode: raw::Opcode) -> Option<(IntrinsicInstrKind, &IntrinsicInstrAbiPropsKind)> {
        self.opcode_intrinsics.get(&opcode)
            .map(|&kind| (kind, &self.intrinsic_abi_props[&kind].kind))
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntrinsicInstrKind {
    /// Like `goto label @ t;` (and `goto label;`)
    ///
    /// Args: `label, t`, in an order defined by the ABI. (use [`JumpIntrinsicArgOrder`])
    Jmp,
    /// Like `interrupt[n]:`
    ///
    /// Args: `n`.
    InterruptLabel,
    /// Like `a = b;` or `a += b;`
    ///
    /// Args: `a, b`.
    AssignOp(ast::AssignOpKind, ScalarType),
    /// Like `a = b + c;`
    ///
    /// Args: `a, b, c`.
    Binop(ast::BinopKind, ScalarType),
    /// Like `a = sin(b);` (or `a = -a;`, but it seems no formats actually have this?)
    ///
    /// This is not used for casts like `a = _S(b);`.  Casts have no explicit representation in
    /// a compiled script; they're just a fundamental part of how the engine reads variables.
    ///
    /// Args: `a, b`.
    Unop(ast::UnopKind, ScalarType),
    /// Like `if (--x) goto label @ t`.
    ///
    /// Args: `x, label, t`, in an order defined by the ABI. (use [`JumpIntrinsicArgOrder`])
    CountJmp,
    /// Like `if (a == c) goto label @ t;`
    ///
    /// Args: `a, b, label, t`, in an order defined by the ABI. (use [`JumpIntrinsicArgOrder`])
    CondJmp(ast::BinopKind, ScalarType),
    /// First part of a conditional jump in languages where it is comprised of 2 instructions.
    /// Sets a hidden compare register.
    ///
    /// Args: `a, b`
    CondJmp2A(ScalarType),
    /// Second part of a 2-instruction conditional jump in languages where it is comprised of 2 instructions.
    /// Jumps based on the hidden compare register.
    ///
    /// Args: `label, t`, in an order defined by the ABI. (use [`JumpIntrinsicArgOrder`])
    CondJmp2B(ast::BinopKind),
}

impl IntrinsicInstrKind {
    pub fn descr(&self) -> &'static str {
        match self {
            Self::Jmp { .. } => "unconditional jump",
            Self::InterruptLabel { .. } => "interrupt label",
            Self::AssignOp { .. } => "assign op",
            Self::Binop { .. } => "binary op",
            Self::Unop { .. } => "unary op",
            Self::CountJmp { .. } => "decrement jump",
            Self::CondJmp { .. } => "conditional jump",
            Self::CondJmp2A { .. } => "dedicated cmp",
            Self::CondJmp2B { .. } => "conditional jump after cmp",
        }
    }
}

impl IntrinsicInstrKind {
    /// Add intrinsic pairs for binary operations in `a = b op c` form in their canonical order,
    /// which is `+, -, *, /, %`, with each operator having an int version and a float version.
    pub fn register_binary_ops(pairs: &mut Vec<(IntrinsicInstrKind, raw::Opcode)>, start: raw::Opcode) {
        use ast::BinopKind as B;

        let mut opcode = start;
        for op in vec![B::Add, B::Sub, B::Mul, B::Div, B::Rem] {
            for ty in vec![ScalarType::Int, ScalarType::Float] {
                pairs.push((IntrinsicInstrKind::Binop(op, ty), opcode));
                opcode += 1;
            }
        }
    }

    /// Add intrinsic pairs for assign ops in their cannonical order: `=, +=, -=, *=, /=, %=`,
    /// with each operator having an int version and a float version.
    pub fn register_assign_ops(pairs: &mut Vec<(IntrinsicInstrKind, raw::Opcode)>, start: raw::Opcode) {
        use ast::AssignOpKind as As;

        let mut opcode = start;
        for op in vec![As::Assign, As::Add, As::Sub, As::Mul, As::Div, As::Rem] {
            for ty in vec![ScalarType::Int, ScalarType::Float] {
                pairs.push((IntrinsicInstrKind::AssignOp(op, ty), opcode));
                opcode += 1;
            }
        }
    }

    /// Add intrinsic pairs for conditional jumps in their cannonical order: `==, !=, <, <=, >, >=`,
    /// with each operator having an int version and a float version.
    pub fn register_cond_jumps(pairs: &mut Vec<(IntrinsicInstrKind, raw::Opcode)>, start: raw::Opcode) {
        use ast::BinopKind as B;

        let mut opcode = start;
        for op in vec![B::Eq, B::Ne, B::Lt, B::Le, B::Gt, B::Ge] {
            for ty in vec![ScalarType::Int, ScalarType::Float] {
                pairs.push((IntrinsicInstrKind::CondJmp(op, ty), opcode));
                opcode += 1;
            }
        }
    }

    /// Register a sequence of six comparison based ops in the order used by EoSD ECL: `<, <=, ==, >, >=, !=`
    pub fn register_olde_ecl_comp_ops(
        pairs: &mut Vec<(IntrinsicInstrKind, raw::Opcode)>,
        start: raw::Opcode,
        kind_fn: impl Fn(ast::BinopKind) -> IntrinsicInstrKind,
    ) {
        use ast::BinopKind as B;

        let mut opcode = start;
        for op in vec![B::Lt, B::Le, B::Eq, B::Gt, B::Ge, B::Ne] {
            pairs.push((kind_fn(op), opcode));
            opcode += 1;
        }
    }
}

pub mod abi_props {
    /// Indicates that the ABI contains this many padding dwords at the end,
    /// which cannot be represented in the AST if they are nonzero.
    #[derive(Debug, Copy, Clone)]
    pub struct UnrepresentablePadding {
        pub index: usize,
        pub count: usize,
    }

    /// Describes the order of jump arguments.
    #[derive(Debug, Copy, Clone)]
    pub struct JumpArgOrder {
        pub index: usize,
        pub kind: JumpArgOrderKind,
    }

    #[derive(Debug, Copy, Clone)]
    pub enum JumpArgOrderKind {
        TimeLoc,
        LocTime,
        /// Always uses timeof(destination).
        Loc,
    }

    /// Describes how an output register is encoded (e.g. is a float register encoded as "f" or "S")?
    #[derive(Debug, Copy, Clone)]
    pub struct OutOperandType {
        pub index: usize,
        pub kind: OutOperandTypeKind,
    }

    #[derive(Debug, Copy, Clone)]
    pub enum OutOperandTypeKind {
        /// It's a float register, but is written to file as an integer. (used by EoSD ECL)
        FloatAsInt,
        /// The register is saved to file using the same datatype as it has in the AST.
        Natural,
    }

    /// An operator input argument.
    #[derive(Debug, Copy, Clone)]
    pub struct InputOperandType { pub index: usize }
    /// An argument that must be an immediate.
    #[derive(Debug, Copy, Clone)]
    pub struct ImmediateInt { pub index: usize }
}

/// Streamlines the relationship between the ABI of an intrinsic and how it is expressed
/// in the AST.
///
/// One can think of this as defining the relationship between syntax sugar
/// (e.g. `if (a == 3) goto label`) and raw instruction syntax
/// (e.g. `ins_2(a, 3, offsetof(label), timeof(label))`).
///
/// Having one of these indicates that the ABI has been validated to be compatible with
/// this intrinsic and we know where all of the pieces of the statement belong in the
/// raw, encoded argument list.
///
/// The `usize`s are all indices into the encoded argument list.
#[derive(Debug)]
pub struct IntrinsicInstrAbiProps {
    pub num_instr_args: usize,
    pub kind: IntrinsicInstrAbiPropsKind,
}

#[derive(Debug, Copy, Clone)]
pub enum IntrinsicInstrAbiPropsKind {
    // this could maybe be a struct with Option<> fields for all of the abi_props types
    // but it's currently written as an enum to leverage unused variable warnings to make
    // sure all of the different places that work with an intrinsic are checking the same
    // set of properties.
    Jmp {
        padding: abi_props::UnrepresentablePadding,
        jump: abi_props::JumpArgOrder,
    },
    InterruptLabel {
        padding: abi_props::UnrepresentablePadding,
        label: abi_props::ImmediateInt,
    },
    AssignOp {
        dest: abi_props::OutOperandType,
        rhs: abi_props::InputOperandType,
    },
    Binop {
        dest: abi_props::OutOperandType,
        args: [abi_props::InputOperandType; 2],
    },
    Unop {
        dest: abi_props::OutOperandType,
        arg: abi_props::InputOperandType,
    },
    CountJmp {
        arg: abi_props::OutOperandType,
        jump: abi_props::JumpArgOrder,
    },
    CondJmp {
        args: [abi_props::InputOperandType; 2],
        jump: abi_props::JumpArgOrder,
    },
    CondJmp2A { args: [abi_props::InputOperandType; 2] },
    CondJmp2B { jump: abi_props::JumpArgOrder },
}

fn intrinsic_abi_error(abi_span: Span, message: &str) -> Diagnostic {
    error!(
        message("bad ABI for intrinsic: {}", message),
        primary(abi_span, "{}", message),
    )
}

impl abi_props::UnrepresentablePadding {
    fn detect_and_remove(arg_encodings: &mut Vec<(usize, ArgEncoding)>, _abi_span: Span) -> Result<Self, Diagnostic> {
        let mut count = 0;
        let mut first_index = arg_encodings.len();
        while let Some(&(index, ArgEncoding::Padding)) = arg_encodings.last() {
            // assumption that this func always runs first (nothing is deleted before us)
            assert_eq!(index, arg_encodings.len() - 1);

            arg_encodings.pop();
            count += 1;
            first_index = index;
        }
        Ok(Self { count, index: first_index })
    }
}

fn remove_first_where<T>(vec: &mut Vec<T>, pred: impl FnMut(&T) -> bool) -> Option<T> {
    match vec.iter().position(pred) {
        Some(index) => Some(vec.remove(index)),
        None => None,
    }
}

impl abi_props::JumpArgOrder {
    fn find_and_remove(arg_encodings: &mut Vec<(usize, ArgEncoding)>, abi_span: Span) -> Result<Self, Diagnostic> {
        let offset_data = remove_first_where(arg_encodings, |&(_, enc)| enc == ArgEncoding::JumpOffset);
        let time_data = remove_first_where(arg_encodings, |&(_, enc)| enc == ArgEncoding::JumpTime);

        let offset_index = offset_data.map(|(index, _)| index);
        let time_index = time_data.map(|(index, _)| index);

        let offset_index = offset_index.ok_or_else(|| {
            intrinsic_abi_error(abi_span, "missing jump offset ('o')")
        })?;

        if let Some(time_index) = time_index {
            if time_index == offset_index + 1 {
                Ok(Self { index: offset_index, kind: abi_props::JumpArgOrderKind::LocTime })
            } else if time_index + 1 == offset_index {
                Ok(Self { index: time_index, kind: abi_props::JumpArgOrderKind::TimeLoc })
            } else {
                Err(intrinsic_abi_error(abi_span, "offset ('o') and time ('t') args must be consecutive"))
            }
        } else {
            Ok(Self { index: offset_index, kind: abi_props::JumpArgOrderKind::Loc })
        }
    }
}

impl abi_props::OutOperandType {
    fn remove(arg_encodings: &mut Vec<(usize, ArgEncoding)>, abi_span: Span, ty_in_ast: ScalarType) -> Result<Self, Diagnostic> {
        // it is expected that previous detect_and_remove functions have already removed everything before the out operand.
        let (index, encoding) = match arg_encodings.len() {
            0 => return Err(intrinsic_abi_error(abi_span, "not enough arguments")),
            _ => arg_encodings.remove(0),
        };
        match (ty_in_ast, encoding) {
            | (ScalarType::Int, ArgEncoding::Dword)
            | (ScalarType::Float, ArgEncoding::Float)
            => Ok(Self { index, kind: abi_props::OutOperandTypeKind::Natural }),

            | (ScalarType::Float, ArgEncoding::Dword)
            => Ok(Self { index, kind: abi_props::OutOperandTypeKind::FloatAsInt }),

            | (_, _)
            => Err(intrinsic_abi_error(abi_span, &format!("output arg has unexpected encoding ({})", encoding.descr()))),
        }
    }
}

impl abi_props::InputOperandType {
    fn remove(arg_encodings: &mut Vec<(usize, ArgEncoding)>, abi_span: Span, ty_in_ast: ScalarType) -> Result<Self, Diagnostic> {
        let (index, encoding) = match arg_encodings.len() {
            0 => return Err(intrinsic_abi_error(abi_span, "not enough arguments")),
            _ => arg_encodings.remove(0),
        };
        match (ty_in_ast, encoding) {
            | (ScalarType::Int, ArgEncoding::Dword)
            | (ScalarType::Float, ArgEncoding::Float)
            => Ok(abi_props::InputOperandType { index }),

            | (_, _)
            => Err(intrinsic_abi_error(abi_span, &format!("output arg has unexpected encoding ({})", encoding.descr()))),
        }
    }
}

impl abi_props::ImmediateInt {
    fn remove(arg_encodings: &mut Vec<(usize, ArgEncoding)>, abi_span: Span) -> Result<Self, Diagnostic> {
        abi_props::InputOperandType::remove(arg_encodings, abi_span, ScalarType::Int)
            .map(|out| Self { index: out.index })
    }
}


impl IntrinsicInstrAbiProps {
    pub fn from_abi(intrinsic: IntrinsicInstrKind, abi: Sp<&InstrAbi>) -> Result<Self, Diagnostic> {
        use IntrinsicInstrKind as I;
        use IntrinsicInstrAbiPropsKind as P;

        let mut encodings = abi.arg_encodings().enumerate().collect::<Vec<_>>();
        let num_instr_args = encodings.len();

        let kind = match intrinsic {
            // DON'T PANIC EVERYTHING IS OK
            //
            // This code might look positively horrifying but 90% of it is impossible to get wrong.
            // (mostly arg types and the ordering of ::remove()s that aren't ::find_and_remove())
            I::Jmp => {
                let padding = abi_props::UnrepresentablePadding::detect_and_remove(&mut encodings, abi.span)?;
                let jump = abi_props::JumpArgOrder::find_and_remove(&mut encodings, abi.span)?;
                P::Jmp { padding, jump }
            },
            I::InterruptLabel => {
                let padding = abi_props::UnrepresentablePadding::detect_and_remove(&mut encodings, abi.span)?;
                let label = abi_props::ImmediateInt::remove(&mut encodings, abi.span)?;
                P::InterruptLabel { label, padding }
            }
            I::AssignOp(_op, ty) => {
                let dest = abi_props::OutOperandType::remove(&mut encodings, abi.span, ty)?;
                let rhs = abi_props::InputOperandType::remove(&mut encodings, abi.span, ty)?;
                P::AssignOp { dest, rhs }
            },
            I::Binop(op, arg_ty) => {
                let out_ty = ast::Expr::binop_ty_from_arg_ty(op, arg_ty);
                let dest = abi_props::OutOperandType::remove(&mut encodings, abi.span, out_ty)?;
                let a = abi_props::InputOperandType::remove(&mut encodings, abi.span, arg_ty)?;
                let b = abi_props::InputOperandType::remove(&mut encodings, abi.span, arg_ty)?;
                P::Binop { dest, args: [a, b] }
            },
            I::Unop(_op, ty) => {
                let dest = abi_props::OutOperandType::remove(&mut encodings, abi.span, ty)?;
                let arg = abi_props::InputOperandType::remove(&mut encodings, abi.span, ty)?;
                P::Unop { dest, arg }
            },
            I::CountJmp => {
                let jump = abi_props::JumpArgOrder::find_and_remove(&mut encodings, abi.span)?;
                let arg = abi_props::OutOperandType::remove(&mut encodings, abi.span, ScalarType::Int)?;
                P::CountJmp { jump, arg }
            },
            I::CondJmp(_op, ty) => {
                let jump = abi_props::JumpArgOrder::find_and_remove(&mut encodings, abi.span)?;
                let a = abi_props::InputOperandType::remove(&mut encodings, abi.span, ty)?;
                let b = abi_props::InputOperandType::remove(&mut encodings, abi.span, ty)?;
                P::CondJmp { jump, args: [a, b] }
            },
            I::CondJmp2A(ty) => {
                let a = abi_props::InputOperandType::remove(&mut encodings, abi.span, ty)?;
                let b = abi_props::InputOperandType::remove(&mut encodings, abi.span, ty)?;
                P::CondJmp2A { args: [a, b] }
            },
            I::CondJmp2B(_op) => {
                let jump = abi_props::JumpArgOrder::find_and_remove(&mut encodings, abi.span)?;
                P::CondJmp2B { jump }
            },
        };

        if let Some(&(index, encoding)) = encodings.get(0) {
            return Err(intrinsic_abi_error(abi.span, &format!("unexpected {} arg at index {}", encoding.descr(), index + 1)));
        }
        Ok(Self { num_instr_args, kind })
    }
}
