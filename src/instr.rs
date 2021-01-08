use std::collections::{HashMap};

use enum_map::EnumMap;
use anyhow::{Context, bail, ensure};

use crate::error::{GatherErrorIteratorExt, CompileError, SimpleError, group_anyhow};
use crate::pos::{Sp, Span};
use crate::ast::{self, Expr};
use crate::ident::Ident;
use crate::scope::VarId;
use crate::type_system::{RegsAndInstrs, TypeSystem, Signature, ArgEncoding, ScalarType};
use crate::binary_io::{BinRead, BinWrite, ReadResult, WriteResult};

#[derive(Debug, Clone, PartialEq)]
pub enum LowLevelStmt {
    /// Represents a single instruction in the compiled file.
    Instr(Instr),
    /// An intrinsic that represents a label that can be jumped to.
    Label(Sp<Ident>),
    /// An intrinsic that begins the scope of a register-allocated local.
    RegAlloc { var: VarId, cause: Span },
    /// An intrinsic that ends the scope of a register-allocated local.
    RegFree { var: VarId },
}
#[derive(Debug, Clone, PartialEq)]
pub struct Instr {
    pub time: i32,
    pub opcode: u16,
    pub args: Vec<InstrArg>,
}
#[derive(Debug, Clone, PartialEq)]
pub enum InstrArg {
    /// A fully encoded argument (an immediate or a register).
    Raw(RawArg),
    /// A register-allocated local.
    Local(VarId),
    /// A label that has not yet been converted to an integer argument.
    ///
    /// This may be present in the input to [`InstrFormat::instr_size`], but will be replaced with
    /// a dword before [`InstrFormat::write_instr`] is called.
    Label(Sp<Ident>),
    /// A `timeof(label)` that has not yet been converted to an integer argument.
    ///
    /// This may be present in the input to [`InstrFormat::instr_size`], but will be replaced with
    /// a dword before [`InstrFormat::write_instr`] is called.
    TimeOf(Sp<Ident>),
}
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct RawArg {
    pub bits: u32,
    pub is_var: bool,
}

impl InstrArg {
    /// Call this at a time when the arg is known to have a fully resolved value.
    ///
    /// Such times are:
    /// * During decompilation.
    /// * Within [`InstrFormat::write_instr`].
    #[track_caller]
    pub fn expect_raw(&self) -> RawArg {
        match *self {
            InstrArg::Raw(x) => x,
            _ => panic!("unexpected unresolved argument (bug!): {:?}", self),
        }
    }

    #[track_caller]
    pub fn expect_immediate_int(&self) -> i32 {
        match *self {
            InstrArg::Raw(x) => {
                assert!(!x.is_var);
                x.bits as i32
            },
            _ => panic!("unexpected unresolved argument (bug!): {:?}", self),
        }
    }

    #[track_caller]
    pub fn expect_immediate_float(&self) -> f32 {
        match *self {
            InstrArg::Raw(x) => {
                assert!(!x.is_var);
                f32::from_bits(x.bits)
            },
            _ => panic!("unexpected unresolved argument (bug!): {:?}", self),
        }
    }
}

impl RawArg {
    pub fn from_reg(number: i32, ty: ScalarType) -> RawArg {
        let bits = match ty {
            ScalarType::Int => number as u32,
            ScalarType::Float => (number as f32).to_bits(),
        };
        RawArg { bits, is_var: true }
    }
}

impl From<u32> for RawArg {
    fn from(x: u32) -> RawArg { RawArg { bits: x, is_var: false } }
}

impl From<i32> for RawArg {
    fn from(x: i32) -> RawArg { RawArg { bits: x as u32, is_var: false } }
}

impl From<f32> for RawArg {
    fn from(x: f32) -> RawArg { RawArg { bits: x.to_bits(), is_var: false } }
}

fn unsupported(span: &crate::pos::Span) -> CompileError {
    error!(
        message("feature not supported by format"),
        primary(span, "not supported by format"),
    )
}

// =============================================================================

/// Reads the instructions of a complete script, attaching useful information on errors.
///
/// Though it primarily uses the `None` output of [`InstrFormat::read_instr`] to determine when to stop reading
/// instructions, it also may be given an end offset. This will cause it to stop with a warning if it lands on this
/// offset without receiving a `None` result, or to fail outright if it goes past this offset.  This enables the
/// reading of TH095's `front.anm`, which contains the only ANM script in existence to have no end marker.  *Sigh.*
pub fn read_instrs(
    f: &mut dyn BinRead,
    format: &dyn InstrFormat,
    starting_offset: usize,
    end_offset: Option<usize>,
) -> ReadResult<Vec<Instr>> {
    let mut script = vec![];
    let mut offset = starting_offset;
    for index in 0.. {
        if let Some(instr) = format.read_instr(f).with_context(|| format!("in instruction {} (offset {:#x})", index, offset))? {
            offset += format.instr_size(&instr);
            script.push(instr);

            if let Some(end_offset) = end_offset {
                match offset.cmp(&end_offset) {
                    std::cmp::Ordering::Less => {},
                    std::cmp::Ordering::Equal => {
                        fast_warning!("original file is missing an end-of-script marker; one will be added on recompilation");
                        break;
                    },
                    std::cmp::Ordering::Greater => {
                        bail!("script read past expected end at offset {:#x} (we're now at offset {:#x}!)", end_offset, offset);
                    },
                }
            }
        } else {
            break;  // no more instructions
        }
    }
    Ok(script)
}

/// Writes the instructions of a complete script, attaching useful information on errors.
pub fn write_instrs(
    f: &mut dyn BinWrite,
    format: &dyn InstrFormat,
    instrs: &[Instr],
) -> WriteResult {
    for (index, instr) in instrs.iter().enumerate() {
        format.write_instr(f, instr).with_context(|| format!("while writing instruction {}", index))?;
    }
    format.write_terminal_instr(f).with_context(|| format!("while writing the script end marker"))?;
    Ok(())
}

// =============================================================================

pub fn lower_sub_ast_to_instrs(
    instr_format: &dyn InstrFormat,
    code: &[Sp<ast::Stmt>],
    ty_ctx: &mut TypeSystem,
) -> Result<Vec<Instr>, CompileError> {
    let mut lowerer = Lowerer {
        out: vec![],
        intrinsic_instrs: instr_format.intrinsic_instrs(),
        ty_ctx,
        instr_format,
    };
    lowerer.lower_sub_ast(code)?;
    let mut out = lowerer.out;

    // And now postprocess
    encode_labels(&mut out, instr_format, 0)?;
    assign_registers(&mut out, instr_format, ty_ctx)?;

    Ok(out.into_iter().filter_map(|x| match x {
        LowLevelStmt::Instr(instr) => Some(instr),
        LowLevelStmt::Label(_) => None,
        LowLevelStmt::RegAlloc { .. } => None,
        LowLevelStmt::RegFree { .. } => None,
    }).collect())
}

/// Helper responsible for converting an AST into [`LowLevelStmt`]s.
struct Lowerer<'ts> {
    out: Vec<LowLevelStmt>,
    intrinsic_instrs: IntrinsicInstrs,
    instr_format: &'ts dyn InstrFormat,
    ty_ctx: &'ts mut TypeSystem,
}

pub struct IntrinsicInstrs {
    intrinsic_opcodes: HashMap<IntrinsicInstrKind, u16>,
    opcode_intrinsics: HashMap<u16, IntrinsicInstrKind>,
}
impl IntrinsicInstrs {
    pub fn from_pairs(pairs: impl IntoIterator<Item=(IntrinsicInstrKind, u16)>) -> Self {
        let intrinsic_opcodes: HashMap<_, _> = pairs.into_iter().collect();
        let opcode_intrinsics = intrinsic_opcodes.iter().map(|(&k, &v)| (v, k)).collect();
        IntrinsicInstrs { opcode_intrinsics, intrinsic_opcodes }
    }

    pub fn get_opcode(&self, intrinsic: IntrinsicInstrKind, span: Span, descr: &str) -> Result<u16, CompileError> {
        match self.intrinsic_opcodes.get(&intrinsic) {
            Some(&opcode) => Ok(opcode),
            None => Err(error!(
                message("feature not supported by format"),
                primary(span, "{} not supported in this game", descr),
            )),
        }
    }

    pub fn get_intrinsic(&self, opcode: u16) -> Option<IntrinsicInstrKind> {
        self.opcode_intrinsics.get(&opcode).copied()
    }
}

impl Lowerer<'_> {
    pub fn lower_sub_ast(
        &mut self,
        code: &[Sp<ast::Stmt>],
    ) -> Result<(), CompileError> {
        let mut th06_anm_end_span = None;
        code.iter().map(|stmt| {
            if let Some(end) = th06_anm_end_span {
                if !matches!(&stmt.body.value, ast::StmtBody::NoInstruction) { return Err(error!(
                    message("statement after end of script"),
                    primary(&stmt.body, "forbidden statement"),
                    secondary(&end, "marks the end of the script"),
                    note("In EoSD ANM, every script must have a single exit point (opcode 0 or 15), as the final instruction."),
                ))}
            }

            for label in &stmt.labels {
                match &label.value {
                    ast::StmtLabel::Label(ident) => self.out.push(LowLevelStmt::Label(ident.clone())),
                    ast::StmtLabel::Difficulty { .. } => return Err(unsupported(&label.span)),
                }
            }

            match &stmt.body.value {
                ast::StmtBody::Jump(goto) => {
                    if goto.time.is_some() && !self.instr_format.jump_has_time_arg() {
                        return Err(error!(
                            message("feature not supported by format"),
                            primary(stmt.body, "'goto @ time' not supported in this game"),
                        ));
                    }

                    let (label_arg, time_arg) = lower_goto_args(goto);

                    self.out.push(LowLevelStmt::Instr(Instr {
                        time: stmt.time,
                        opcode: self.get_opcode(IKind::Jmp, stmt.body.span, "'goto'")?,
                        args: vec![label_arg, time_arg],
                    }));
                },


                ast::StmtBody::Assignment { var, op, value } => {
                    self.assign_op(stmt.body.span, stmt.time, var, op, value)?;
                },


                ast::StmtBody::InterruptLabel(interrupt_id) => {
                    self.out.push(LowLevelStmt::Instr(Instr {
                        time: stmt.time,
                        opcode: self.get_opcode(IKind::InterruptLabel, stmt.body.span, "interrupt label")?,
                        args: vec![InstrArg::Raw(interrupt_id.value.into())],
                    }));
                },


                ast::StmtBody::CondJump { keyword, cond, jump } => {
                    self.cond_jump(stmt.body.span, stmt.time, keyword, cond, jump)?;
                },


                ast::StmtBody::Expr(expr) => match &expr.value {
                    ast::Expr::Call { func, args } => {
                        let opcode = self.func_stmt(stmt, func, args)?;
                        if self.instr_format.is_th06_anm_terminating_instr(opcode) {
                            th06_anm_end_span = Some(func);
                        }
                    },
                    _ => return Err(unsupported(&stmt.body.span)),
                }, // match expr

                ast::StmtBody::NoInstruction => {}

                _ => return Err(unsupported(&stmt.body.span)),
            }
            Ok(())
        }).collect_with_recovery()
    }

    /// Lowers `func(<ARG1>, <ARG2>, <...>);`
    fn func_stmt<'a>(
        &mut self,
        stmt: &Sp<ast::Stmt>,
        func: &Sp<Ident>,
        args: &[Sp<Expr>],
    ) -> Result<u16, CompileError> {
        // all function statements currently refer to single instructions
        let opcode = match self.ty_ctx.regs_and_instrs.resolve_func_aliases(func).as_ins() {
            Some(opcode) => opcode,
            None => return Err(error!(
                message("unknown instruction '{}'", func),
                primary(func, "not an instruction"),
            )),
        };

        self.instruction(stmt, opcode as _, func, args)
    }

    /// Lowers `func(<ARG1>, <ARG2>, <...>);` where `func` is an instruction alias.
    fn instruction(
        &mut self,
        stmt: &Sp<ast::Stmt>,
        opcode: u16,
        name: &Sp<Ident>,
        args: &[Sp<Expr>],
    ) -> Result<u16, CompileError> {
        let siggy = match self.ty_ctx.regs_and_instrs.ins_signature(opcode) {
            Some(siggy) => siggy,
            None => return Err(error!(
                message("signature of '{}' not known", name),
                primary(name, "don't know how to compile this instruction"),
            )),
        };
        let encodings = siggy.arg_encodings();
        if !(siggy.min_args() <= args.len() && args.len() <= siggy.max_args()) {
            return Err(error!(
                message("wrong number of arguments to '{}'", name),
                primary(name, "expects {} arguments, got {}", encodings.len(), args.len()),
            ));
        }

        let mut temp_var_ids = vec![];
        let low_level_args = encodings.iter().zip(args).enumerate().map(|(arg_index, (enc, expr))| {
            let (lowered, actual_ty) = match try_lower_simple_arg(expr, self.ty_ctx)? {
                ExprClass::Simple(arg, arg_ty) => (arg, arg_ty),
                ExprClass::Complex(_) => {
                    // Save this expression to a temporary
                    let arg_ty = self.ty_ctx.compute_type_shallow(expr)?;
                    let (var_id, _) = self.define_temporary(stmt.time, arg_ty, expr)?;
                    let arg = InstrArg::Local(var_id);

                    temp_var_ids.push(var_id); // so we can free the register later

                    (arg, arg_ty)
                },
            };

            let expected_ty = match enc {
                ArgEncoding::Padding |
                ArgEncoding::Color |
                ArgEncoding::Dword => ScalarType::Int,
                ArgEncoding::Float => ScalarType::Float,
            };
            if actual_ty != expected_ty {
                return Err(error!(
                    message("argument {} to '{}' has wrong type", arg_index+1, name),
                    primary(expr, "wrong type"),
                    secondary(name, "expects {}", expected_ty.descr()),
                ));
            }
            Ok(lowered)
        }).collect_with_recovery()?;

        self.out.push(LowLevelStmt::Instr(Instr {
            time: stmt.time,
            opcode: opcode as _,
            args: low_level_args,
        }));

        for var_id in temp_var_ids.into_iter().rev() {
            self.undefine_temporary(var_id)?;
        }

        Ok(opcode)
    }

    /// Lowers `a = <B>;`  or  `a *= <B>;`
    fn assign_op(
        &mut self,
        span: Span,
        time: i32,
        var: &Sp<ast::Var>,
        assign_op: &Sp<ast::AssignOpKind>,
        rhs: &Sp<ast::Expr>,
    ) -> Result<(), CompileError> {
        match (assign_op.value, &rhs.value) {
            // a = <expr> + <expr>
            (ast::AssignOpKind::Assign, Expr::Binop(a, binop, b)) => {
                self.assign_direct_binop(span, time, var, assign_op, rhs.span, a, binop, b)?;
            },

            // a += <expr>
            (_, _) => {
                let (arg_var, ty_var) = lower_var_to_arg(var, self.ty_ctx)?;
                match try_lower_simple_arg(rhs, self.ty_ctx)? {
                    ExprClass::Simple(arg_rhs, ty_rhs) => {
                        let ty = ty_var.check_same(ty_rhs, assign_op.span, (var.span, rhs.span))?;
                        self.out.push(LowLevelStmt::Instr(Instr {
                            time,
                            opcode: self.get_opcode(IKind::AssignOp(assign_op.value, ty), span, "update assignment with this operation")?,
                            args: vec![arg_var, arg_rhs],
                        }));
                    },
                    // split out to: `tmp = <expr>;  a += tmp;`
                    ExprClass::Complex(_) => {
                        let (tmp_var_id, tmp_var_expr) = self.define_temporary(time, ty_var, rhs)?;
                        self.assign_op(span, time, var, assign_op, &tmp_var_expr)?;
                        self.undefine_temporary(tmp_var_id)?;
                    },
                }
            },
        }
        Ok(())
    }

    /// Lowers `a = <B> * <C>;`
    fn assign_direct_binop(
        &mut self,
        span: Span,
        time: i32,
        var: &Sp<ast::Var>,
        eq_sign: &Sp<ast::AssignOpKind>,
        rhs_span: Span,
        a: &Sp<Expr>,
        binop: &Sp<ast::BinopKind>,
        b: &Sp<Expr>,
    ) -> Result<(), CompileError> {
        // So right here, we have something like `a = <B> * <C>`. If <B> and <C> are both simple arguments (literals or
        // variables), we can emit this as one instruction. Otherwise, we need to break it up.  In the general case this
        // would mean producing
        //
        //     int tmp;
        //     tmp = <B>;      // recursive call
        //     a = tmp * <C>;  // recursive call
        //
        // but sometimes it's possible to do this without a temporary by reusing the destination variable `a`, such as:
        //
        //     a = <B>;        // recursive call
        //     a = tmp * <C>;  // recursive call

        let (arg_var, ty_var) = lower_var_to_arg(var, self.ty_ctx)?;
        let classified_args = [try_lower_simple_arg(a, self.ty_ctx)?, try_lower_simple_arg(b, self.ty_ctx)?];

        // Preserve execution order by always splitting out the first large subexpression.
        let split_out_index = (0..2).filter(|&i| classified_args[i].as_complex().is_some()).next();
        match split_out_index {
            Some(split_out_index) => {
                let other_index = 1 - split_out_index;

                // If the other expression does not use our destination variable, we can reuse it.
                let mut temp_var_id = None;
                let mut split_out_var = var.clone();
                let split_out_expr = [&a, &b][split_out_index];
                let split_out_span = split_out_expr.span;
                let split_out_op = sp!(split_out_span => ast::AssignOpKind::Assign);
                if expr_uses_var([&a, &b][other_index], var) {
                    // It's used, so we need a temporary.

                    let subexpr_ty = self.ty_ctx.compute_type_shallow(split_out_expr)?;
                    let (var_id, tmp_var, _) = self.allocate_temporary(split_out_span, subexpr_ty);

                    temp_var_id = Some(var_id);
                    split_out_var = tmp_var;
                };

                // first statement
                self.assign_op(split_out_span, time, &split_out_var, &split_out_op, split_out_expr)?;

                // second statement:  reconstruct the outer expression, replacing the part we split out
                let mut parts: [&Sp<ast::Expr>; 2] = [a, b];
                let split_out_var_as_expr = sp!(split_out_var.span => ast::Expr::Var(split_out_var));
                parts[split_out_index] = &split_out_var_as_expr;
                self.assign_direct_binop(span, time, var, eq_sign, rhs_span, parts[0], binop, parts[1])?;

                if let Some(var_id) = temp_var_id {
                    self.undefine_temporary(var_id)?;
                }
            },

            // if they're both simple, that's our base case, and we emit a single instruction
            None => {
                let (arg_a, ty_a) = classified_args[0].as_simple().unwrap();
                let (arg_b, ty_b) = classified_args[1].as_simple().unwrap();
                let ty_rhs = binop.result_type(ty_a, ty_b, (a.span, b.span))?;
                let ty = ty_var.check_same(ty_rhs, eq_sign.span, (var.span, rhs_span))?;
                self.out.push(LowLevelStmt::Instr(Instr {
                    time,
                    opcode: self.get_opcode(IKind::Binop(binop.value, ty), span, "assignment of this binary operation")?,
                    args: vec![arg_var, arg_a.clone(), arg_b.clone()],
                }));
            },
        }
        Ok(())
    }

    /// Lowers `if (<cond>) goto label @ time;`
    fn cond_jump(
        &mut self,
        stmt_span: Span,
        stmt_time: i32,
        keyword: &Sp<ast::CondKeyword>,
        cond: &Sp<ast::Cond>,
        goto: &ast::StmtGoto,
    ) -> Result<(), CompileError>{
        let (arg_label, arg_time) = lower_goto_args(goto);

        match (keyword.value, &cond.value) {
            (ast::CondKeyword::If, ast::Cond::Decrement(var)) => {
                let (arg_var, ty_var) = lower_var_to_arg(var, self.ty_ctx)?;
                if ty_var != ScalarType::Int {
                    return Err(error!(
                        message("type error"),
                        primary(cond, "expected an int, got {}", ty_var.descr()),
                        secondary(keyword, "required by this"),
                    ));
                }

                self.out.push(LowLevelStmt::Instr(Instr {
                    time: stmt_time,
                    opcode: self.get_opcode(IKind::CountJmp, stmt_span, "decrement jump")?,
                    args: vec![arg_var, arg_label, arg_time],
                }));
                Ok(())
            },

            (ast::CondKeyword::If, ast::Cond::Expr(expr)) => match &expr.value {
                Expr::Binop(a, binop, b) => self.cond_jump_binop(stmt_span, stmt_time, keyword, a, binop, b, goto),

                _ => Err(unsupported(&expr.span)),
            },
        }
    }

    /// Lowers `if (<A> != <B>) goto label @ time;` and similar
    fn cond_jump_binop(
        &mut self,
        stmt_span: Span,
        stmt_time: i32,
        keyword: &Sp<ast::CondKeyword>,
        a: &Sp<Expr>,
        binop: &Sp<ast::BinopKind>,
        b: &Sp<Expr>,
        goto: &ast::StmtGoto,
    ) -> Result<(), CompileError>{
        match (try_lower_simple_arg(a, self.ty_ctx)?, try_lower_simple_arg(b, self.ty_ctx)?) {
            // `if (<A> != <B>) ...`
            // split out to: `tmp = <A>;  if (tmp != <B>) ...`;
            (ExprClass::Complex(_), _) => {
                let ty_tmp = self.ty_ctx.compute_type_shallow(a)?;

                let (var_id, var_expr) = self.define_temporary(stmt_time, ty_tmp, a)?;
                self.cond_jump_binop(stmt_span, stmt_time, keyword, &var_expr, binop, b, goto)?;
                self.undefine_temporary(var_id)?;
            },

            // `if (a != <B>) ...`
            // split out to: `tmp = <B>;  if (a != tmp) ...`;
            (ExprClass::Simple(_, ty_tmp), ExprClass::Complex(_)) => {
                let (var_id, var_expr) = self.define_temporary(stmt_time, ty_tmp, b)?;
                self.cond_jump_binop(stmt_span, stmt_time, keyword, a, binop, &var_expr, goto)?;
                self.undefine_temporary(var_id)?;
            },

            // `if (a != b) ...`
            (ExprClass::Simple(arg_a, ty_a), ExprClass::Simple(arg_b, ty_b)) => {
                let ty_arg = binop.result_type(ty_a, ty_b, (a.span, b.span))?;
                let (arg_label, arg_time) = lower_goto_args(goto);
                self.out.push(LowLevelStmt::Instr(Instr {
                    time: stmt_time,
                    opcode: self.get_opcode(IKind::CondJmp(binop.value, ty_arg), binop.span, "conditional jump with this operator")?,
                    args: vec![arg_a, arg_b, arg_label, arg_time],
                }));
            },
        }
        Ok(())
    }

    /// Declares a new register-allocated temporary and initializes it with an expression.
    ///
    /// When done emitting instructions that use the temporary, one should call [`Self::undefine_temporary`].
    fn define_temporary(
        &mut self,
        time: i32,
        ty: ScalarType,
        expr: &Sp<Expr>,
    ) -> Result<(VarId, Sp<Expr>), CompileError> {
        let (var_id, var, var_as_expr) = self.allocate_temporary(expr.span, ty);

        let eq_sign = sp!(expr.span => ast::AssignOpKind::Assign);
        self.assign_op(expr.span, time, &var, &eq_sign, expr)?;

        Ok((var_id, var_as_expr))
    }

    /// Emits an intrinsic that cleans up a register-allocated temporary.
    fn undefine_temporary(&mut self, var_id: VarId) -> Result<(), CompileError> {
        self.out.push(LowLevelStmt::RegFree { var: var_id });
        Ok(())
    }

    /// Grabs a new unique [`VarId`] and constructs an [`ast::Var`] as well as an [`ast::Expr`] for using the
    /// variable in an expression.  Emits an intrinsic to allocate a register to it.
    ///
    /// Call [`Self::undefine_temporary`] afterwards to clean up.
    fn allocate_temporary(
        &mut self,
        span: Span,
        ty: ScalarType,
    ) -> (VarId, Sp<ast::Var>, Sp<ast::Expr>) {
        let var_id = self.ty_ctx.variables_mut().declare_temporary(Some(ty));
        let var = sp!(span => ast::Var::Local { var_id, ty_sigil: None });
        let var_as_expr = sp!(span => ast::Expr::Var(var.clone()));

        self.out.push(LowLevelStmt::RegAlloc { var: var_id, cause: span });

        (var_id, var, var_as_expr)
    }

    fn get_opcode(&self, kind: IntrinsicInstrKind, span: Span, descr: &str) -> Result<u16, CompileError> {
        self.intrinsic_instrs.get_opcode(kind, span, descr)
    }
}

enum ExprClass<'a> {
    Simple(InstrArg, ScalarType),
    Complex(&'a Sp<Expr>),
}

impl ExprClass<'_> {
    fn as_complex(&self) -> Option<&Sp<Expr>> {
        match self { ExprClass::Complex(x) => Some(x), _ => None }
    }
    fn as_simple(&self) -> Option<(&InstrArg, ScalarType)> {
        match self { &ExprClass::Simple(ref a, ty) => Some((a, ty)), _ => None }
    }
}

fn try_lower_simple_arg<'a>(arg: &'a Sp<ast::Expr>, ty_ctx: &TypeSystem) -> Result<ExprClass<'a>, CompileError> {
    match arg.value {
        ast::Expr::LitInt { value, .. } => Ok(ExprClass::Simple(InstrArg::Raw(value.into()), ScalarType::Int)),
        ast::Expr::LitFloat { value, .. } => Ok(ExprClass::Simple(InstrArg::Raw(value.into()), ScalarType::Float)),
        ast::Expr::Var(ref var) => {
            let (out, ty) = lower_var_to_arg(var, ty_ctx)?;
            Ok(ExprClass::Simple(out, ty))
        },
        _ => Ok(ExprClass::Complex(arg)),
    }
}

fn lower_var_to_arg(var: &Sp<ast::Var>, ty_ctx: &TypeSystem) -> Result<(InstrArg, ScalarType), CompileError> {
    let ty = ty_ctx.var_type(var).ok_or(error!(
        message("variable requires a type prefix"),
        primary(var, "needs a '$' or '%' prefix"),
    ))?;

    match ty_ctx.regs_and_instrs.reg_id(var) {
        Some(opcode) => {
            let lowered = InstrArg::Raw(RawArg::from_reg(opcode, ty));
            Ok((lowered, ty))
        },
        None => match var.value {
            ast::Var::Local { var_id, .. } => Ok((InstrArg::Local(var_id), ty)),
            _ => Err(error!(
                message("unrecognized variable"),
                primary(var, "not a known global or local variable"),
            ))
        },
    }
}

fn lower_goto_args(goto: &ast::StmtGoto) -> (InstrArg, InstrArg) {
    let label_arg = InstrArg::Label(goto.destination.clone());
    let time_arg = match goto.time {
        Some(time) => InstrArg::Raw(time.into()),
        None => InstrArg::TimeOf(goto.destination.clone()),
    };
    (label_arg, time_arg)
}

pub fn raise_instrs_to_sub_ast(
    ty_ctx: &RegsAndInstrs,
    instr_format: &dyn InstrFormat,
    script: &[Instr],
) -> Result<Vec<Sp<ast::Stmt>>, SimpleError> {
    let intrinsic_instrs = instr_format.intrinsic_instrs();

    // For now we give every instruction a label and strip the unused ones later.
    let mut offset = 0;
    let code = script.iter().map(|instr| {
        let this_instr_label = sp!(ast::StmtLabel::Label(default_instr_label(offset)));
        offset += instr_format.instr_size(instr);

        let body = raise_instr(instr_format, instr, ty_ctx, &intrinsic_instrs)?;
        Ok(sp!(ast::Stmt {
            time: instr.time,
            labels: vec![this_instr_label],
            body: sp!(body),
        }))
    }).collect();
    code
}

fn default_instr_label(offset: usize) -> Sp<Ident> {
    sp!(format!("label_{}", offset).parse::<Ident>().unwrap())
}

fn raise_instr(
    instr_format: &dyn InstrFormat,
    instr: &Instr,
    ty_ctx: &RegsAndInstrs,
    intrinsic_instrs: &IntrinsicInstrs,
) -> Result<ast::StmtBody, SimpleError> {
    let Instr { opcode, ref args, .. } = *instr;
    match intrinsic_instrs.get_intrinsic(opcode) {
        Some(IKind::Jmp) => group_anyhow(|| {
            let nargs = if instr_format.jump_has_time_arg() { 2 } else { 1 };

            // This one is >= because it exists in early STD where there can be padding args.
            ensure!(args.len() >= nargs, "expected {} args, got {}", nargs, args.len());
            ensure!(args[nargs..].iter().all(|a| a.expect_raw().bits == 0), "unsupported data in padding of intrinsic");

            let dest_offset = instr_format.decode_label(args[0].expect_raw().bits);
            let dest_time = match instr_format.jump_has_time_arg() {
                true => Some(args[1].expect_immediate_int()),
                false => None,
            };
            Ok(ast::StmtBody::Jump(ast::StmtGoto {
                destination: default_instr_label(dest_offset),
                time: dest_time,
            }))
        }).with_context(|| format!("while decompiling a 'goto' operation")),


        Some(IKind::AssignOp(op, ty)) => group_anyhow(|| {
            ensure!(args.len() == 2, "expected {} args, got {}", 2, args.len());
            Ok(ast::StmtBody::Assignment {
                var: sp!(raise_arg_to_var(&args[0].expect_raw(), ty, ty_ctx)?),
                op: sp!(op),
                value: sp!(raise_arg(&args[1].expect_raw(), ty.default_encoding(), ty_ctx)?),
            })
        }).with_context(|| format!("while decompiling a '{}' operation", op)),


        Some(IKind::Binop(op, ty)) => group_anyhow(|| {
            ensure!(args.len() == 3, "expected {} args, got {}", 3, args.len());
            Ok(ast::StmtBody::Assignment {
                var: sp!(raise_arg_to_var(&args[0].expect_raw(), ty, ty_ctx)?),
                op: sp!(ast::AssignOpKind::Assign),
                value: sp!(Expr::Binop(
                    Box::new(sp!(raise_arg(&args[1].expect_raw(), ty.default_encoding(), ty_ctx)?)),
                    sp!(op),
                    Box::new(sp!(raise_arg(&args[2].expect_raw(), ty.default_encoding(), ty_ctx)?)),
                )),
            })
        }).with_context(|| format!("while decompiling a '{}' operation", op)),


        Some(IKind::InterruptLabel) => group_anyhow(|| {
            // This one is >= because it exists in STD where there can be padding args.
            ensure!(args.len() >= 1, "expected {} args, got {}", 1, args.len());
            ensure!(args[1..].iter().all(|a| a.expect_raw().bits == 0), "unsupported data in padding of intrinsic");

            Ok(ast::StmtBody::InterruptLabel(sp!(args[0].expect_immediate_int())))
        }).with_context(|| format!("while decompiling an interrupt label")),


        Some(IKind::CountJmp) => group_anyhow(|| {
            ensure!(args.len() == 3, "expected {} args, got {}", 3, args.len());
            let var = raise_arg_to_var(&args[0].expect_raw(), ScalarType::Int, ty_ctx)?;
            let dest_offset = instr_format.decode_label(args[1].expect_raw().bits);
            let dest_time = Some(args[2].expect_immediate_int());

            Ok(ast::StmtBody::CondJump {
                keyword: sp!(ast::CondKeyword::If),
                cond: sp!(ast::Cond::Decrement(sp!(var))),
                jump: ast::StmtGoto {
                    destination: default_instr_label(dest_offset),
                    time: dest_time,
                },
            })
        }).with_context(|| format!("while decompiling a decrement jump")),


        Some(IKind::CondJmp(op, ty)) => group_anyhow(|| {
            ensure!(args.len() == 4, "expected {} args, got {}", 4, args.len());
            let a = raise_arg(&args[0].expect_raw(), ty.default_encoding(), ty_ctx)?;
            let b = raise_arg(&args[1].expect_raw(), ty.default_encoding(), ty_ctx)?;
            let dest_offset = instr_format.decode_label(args[2].expect_raw().bits);
            let dest_time = Some(args[3].expect_immediate_int());

            Ok(ast::StmtBody::CondJump {
                keyword: sp!(ast::CondKeyword::If),
                cond: sp!(ast::Cond::Expr(sp!({
                    ast::Expr::Binop(Box::new(sp!(a)), sp!(op), Box::new(sp!(b)))
                }))),
                jump: ast::StmtGoto {
                    destination: default_instr_label(dest_offset),
                    time: dest_time,
                },
            })
        }).with_context(|| format!("while decompiling a conditional jump")),


        // raising of these not yet implemented
        Some(IKind::TransOp(_)) |
        None => group_anyhow(|| {
            // Default behavior for general instructions
            let ins_ident = {
                ty_ctx.opcode_names.get(&opcode).cloned()
                    .unwrap_or_else(|| Ident::new_ins(opcode))
            };

            Ok(ast::StmtBody::Expr(sp!(Expr::Call {
                args: match ty_ctx.ins_signature(opcode) {
                    Some(siggy) => raise_args(args, siggy, ty_ctx)?,
                    None => raise_args(args, &Signature::auto(args.len()), ty_ctx)?,
                },
                func: sp!(ins_ident),
            })))
        }).with_context(|| format!("while decompiling ins_{}", opcode)),
    }
}

fn raise_args(args: &[InstrArg], siggy: &Signature, ty_ctx: &RegsAndInstrs) -> Result<Vec<Sp<Expr>>, SimpleError> {
    let encodings = siggy.arg_encodings();

    if args.len() != encodings.len() {
        bail!("provided arg count ({}) does not match mapfile ({})", args.len(), encodings.len());
    }
    let mut out = encodings.iter().zip(args).enumerate().map(|(i, (&enc, arg))| {
        let arg_ast = raise_arg(&arg.expect_raw(), enc, ty_ctx).with_context(|| format!("in argument {}", i + 1))?;
        Ok(sp!(arg_ast))
    }).collect::<Result<Vec<_>, SimpleError>>()?;

    // drop early STD padding args from the end as long as they're zero
    for (enc, arg) in encodings.iter().zip(args).rev() {
        match (enc, arg) {
            (ArgEncoding::Padding, InstrArg::Raw(RawArg { bits: 0, .. })) => out.pop(),
            _ => break,
        };
    }
    Ok(out)
}

fn raise_arg(raw: &RawArg, enc: ArgEncoding, ty_ctx: &RegsAndInstrs) -> Result<Expr, SimpleError> {
    if raw.is_var {
        let ty = match enc {
            ArgEncoding::Padding |
            ArgEncoding::Color |
            ArgEncoding::Dword => ScalarType::Int,
            ArgEncoding::Float => ScalarType::Float,
        };
        Ok(Expr::Var(sp!(raise_arg_to_var(raw, ty, ty_ctx)?)))
    } else {
        raise_arg_to_literal(raw, enc)
    }
}

fn raise_arg_to_literal(raw: &RawArg, enc: ArgEncoding) -> Result<Expr, SimpleError> {
    if raw.is_var {
        bail!("expected an immediate, got a variable");
    }
    match enc {
        ArgEncoding::Padding |
        ArgEncoding::Dword => Ok(Expr::from(raw.bits as i32)),
        ArgEncoding::Color => Ok(Expr::LitInt { value: raw.bits as i32, hex: true }),
        ArgEncoding::Float => Ok(Expr::from(f32::from_bits(raw.bits))),
    }
}

fn raise_arg_to_var(raw: &RawArg, ty: ScalarType, ty_ctx: &RegsAndInstrs) -> Result<ast::Var, SimpleError> {
    if !raw.is_var {
        bail!("expected a variable, got an immediate");
    }
    let id = match ty {
        ScalarType::Int => raw.bits as i32,
        ScalarType::Float => {
            let float_id = f32::from_bits(raw.bits);
            if float_id != f32::round(float_id) {
                bail!("non-integer float variable [{}] in binary file!", float_id);
            }
            float_id as i32
        },
    };
    Ok(ty_ctx.reg_to_ast(id, ty))
}

fn expr_uses_var(ast: &Sp<ast::Expr>, var: &ast::Var) -> bool {
    use ast::Visit;

    struct Visitor<'a> {
        var: &'a ast::Var,
        found: bool,
    };

    impl Visit for Visitor<'_> {
        fn visit_var(&mut self, var: &Sp<ast::Var>) {
            if self.var.eq_upto_ty(var) {
                self.found = true;
            }
        }
    }

    let mut v = Visitor { var, found: false };
    v.visit_expr(ast);
    v.found
}

// =============================================================================

struct RawLabelInfo {
    time: i32,
    offset: usize,
}
fn gather_label_info(
    format: &dyn InstrFormat,
    initial_offset: usize,
    code: &[LowLevelStmt]
) -> Result<HashMap<Sp<Ident>, RawLabelInfo>, CompileError> {
    use std::collections::hash_map::Entry;

    let mut offset = initial_offset;
    let mut pending_labels = vec![];
    let mut out = HashMap::new();
    code.iter().map(|thing| {
        match thing {
            // can't insert labels until we see the time of the instructions they are labeling
            LowLevelStmt::Label(ident) => pending_labels.push(ident),
            LowLevelStmt::Instr(instr) => {
                for label in pending_labels.drain(..) {
                    match out.entry(label.clone()) {
                        Entry::Vacant(e) => {
                            e.insert(RawLabelInfo { time: instr.time, offset });
                        },
                        Entry::Occupied(e) => {
                            let old = e.key();
                            return Err(error!{
                                message("duplicate label '{}'", label),
                                primary(label, "redefined here"),
                                secondary(old, "originally defined here"),
                            });
                        },
                    }
                }
                offset += format.instr_size(instr);
            },
            _ => {},
        }
        Ok(())
    }).collect_with_recovery()?;
    assert!(pending_labels.is_empty(), "unexpected label after last instruction! (bug?)");
    Ok(out)
}

/// Eliminates all `InstrArg::Label`s by replacing them with their dword values.
fn encode_labels(
    code: &mut [LowLevelStmt],
    format: &dyn InstrFormat,
    initial_offset: usize,
) -> Result<(), CompileError> {
    let label_info = gather_label_info(format, initial_offset, code)?;

    code.iter_mut().map(|thing| {
        match thing {
            LowLevelStmt::Instr(instr) => for arg in &mut instr.args {
                match *arg {
                    | InstrArg::Label(ref label)
                    | InstrArg::TimeOf(ref label)
                    => match label_info.get(label) {
                        Some(info) => match arg {
                            InstrArg::Label(_) => *arg = InstrArg::Raw(format.encode_label(info.offset).into()),
                            InstrArg::TimeOf(_) => *arg = InstrArg::Raw(info.time.into()),
                            _ => unreachable!(),
                        },
                        None => return Err(error!{
                            message("undefined label '{}'", label),
                            primary(label, "there is no label by this name"),
                        }),
                    },
                    _ => {},
                }
            },
            _ => {},
        }
        Ok(())
    }).collect_with_recovery()
}

/// Eliminates all `InstrArg::Label`s by replacing them with their dword values.
fn assign_registers(
    code: &mut [LowLevelStmt],
    format: &dyn InstrFormat,
    ty_ctx: &TypeSystem,
) -> Result<(), CompileError> {
    let used_regs = get_used_regs(code);

    let mut unused_regs = format.general_use_regs();
    for vec in unused_regs.values_mut() {
        vec.retain(|id| !used_regs.contains(id));
        vec.reverse();  // since we'll be popping from these lists
    }

    let mut var_regs = HashMap::<VarId, (i32, ScalarType, Span)>::new();

    for stmt in code {
        match stmt {
            LowLevelStmt::RegAlloc { var: var_id, ref cause } => {
                let ty = ty_ctx.variables().get_type(*var_id).expect("(bug!) this should have been type-checked!");

                let reg = unused_regs[ty].pop().ok_or_else(|| {
                    let stringify_reg = |reg| crate::fmt::stringify(&ty_ctx.regs_and_instrs.reg_to_ast(reg, ty));

                    let mut error = crate::error::Diagnostic::error();
                    error.message(format!("expression too complex to compile"));
                    error.primary(cause, format!("no more registers of this type!"));
                    for &(scratch_reg, scratch_ty, scratch_span) in var_regs.values() {
                        if scratch_ty == ty {
                            error.secondary(scratch_span, format!("{} holds this", stringify_reg(scratch_reg)));
                        }
                    }
                    let regs_of_ty = format.general_use_regs()[ty].clone();
                    let unavailable_strs = regs_of_ty.iter().copied()
                        .filter(|id| used_regs.contains(id))
                        .map(stringify_reg)
                        .collect::<Vec<_>>();
                    if !unavailable_strs.is_empty() {
                        error.note(format!(
                            "the following registers are unavailable due to explicit use: {}",
                            unavailable_strs.join(", "),
                        ));
                    }

                    error
                })?;

                assert!(var_regs.insert(*var_id, (reg, ty, *cause)).is_none());
            },
            LowLevelStmt::RegFree { var: var_id } => {
                let ty = ty_ctx.variables().get_type(*var_id).expect("(bug!) this should have been type-checked!");
                let (reg, _, _) = var_regs.remove(&var_id).expect("(bug!) RegFree without RegAlloc!");
                unused_regs[ty].push(reg);
            },
            LowLevelStmt::Instr(instr) => {
                for arg in &mut instr.args {
                    if let InstrArg::Local(var_id) = *arg {
                        let ty = ty_ctx.variables().get_type(var_id).expect("(bug!) this should have been type-checked!");
                        *arg = InstrArg::Raw(RawArg::from_reg(var_regs[&var_id].0, ty));
                    }
                }
            },
            LowLevelStmt::Label(_) => {},
        }
    }

    Ok(())
}

fn get_used_regs(stmts: &[LowLevelStmt]) -> Vec<i32> {
    stmts.iter()
        .filter_map(|stmt| match stmt { LowLevelStmt::Instr(instr) => Some(instr), _ => None })
        .flat_map(|instr| instr.args.iter().filter_map(|arg| match arg {
            &InstrArg::Raw(RawArg { is_var: true, bits }) => Some(bits as i32),
            _ => None,
        })).collect()
}

// =============================================================================

use IntrinsicInstrKind as IKind;
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntrinsicInstrKind {
    /// Like `goto label @ t;` (and `goto label;`)
    ///
    /// Args: `label, t`.
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
    /// Like `a = sin(b);`
    ///
    /// Args: `a, b`.
    TransOp(TransOpKind),
    /// Like `if (x--) goto label @ t`.
    ///
    /// Args: `x, label, t`.
    CountJmp,
    /// Like `if (a == c) goto label @ t;`
    ///
    /// Args: `a, b, label, t`
    CondJmp(ast::BinopKind, ScalarType),
}

/// Transcendental functions available in at least one game.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransOpKind { Sin, Cos, Tan, Acos, Atan }

/// Add intrinsic pairs for binary operations in `a = b op c` form in their canonical order,
/// which is `+, -, *, /, %`, with each operator having an int version and a float version.
pub fn register_binary_ops(pairs: &mut Vec<(IntrinsicInstrKind, u16)>, start: u16) {
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
pub fn register_assign_ops(pairs: &mut Vec<(IntrinsicInstrKind, u16)>, start: u16) {
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
pub fn register_cond_jumps(pairs: &mut Vec<(IntrinsicInstrKind, u16)>, start: u16) {
    use ast::BinopKind as B;

    let mut opcode = start;
    for op in vec![B::Eq, B::Ne, B::Lt, B::Le, B::Gt, B::Ge] {
        for ty in vec![ScalarType::Int, ScalarType::Float] {
            pairs.push((IntrinsicInstrKind::CondJmp(op, ty), opcode));
            opcode += 1;
        }
    }
}

pub trait InstrFormat {
    /// Get the number of bytes in the binary encoding of an instruction.
    fn instr_size(&self, instr: &Instr) -> usize;

    fn intrinsic_instrs(&self) -> IntrinsicInstrs {
        IntrinsicInstrs::from_pairs(self.intrinsic_opcode_pairs())
    }

    fn intrinsic_opcode_pairs(&self) -> Vec<(IntrinsicInstrKind, u16)>;

    /// Read a single script instruction from an input stream.
    ///
    /// Should return `None` when it reaches the marker that indicates the end of the script.
    /// When this occurs, it may leave the `Cursor` in an indeterminate state.
    fn read_instr(&self, f: &mut dyn BinRead) -> ReadResult<Option<Instr>>;

    /// Write a single script instruction into an output stream.
    fn write_instr(&self, f: &mut dyn BinWrite, instr: &Instr) -> WriteResult;

    /// Write a marker that goes after the final instruction in a function or script.
    fn write_terminal_instr(&self, f: &mut dyn BinWrite) -> WriteResult;

    // ---------------------------------------------------
    // Special purpose functions only overridden by a few formats

    /// List of registers available for scratch use in formats without a stack.
    fn general_use_regs(&self) -> EnumMap<ScalarType, Vec<i32>> {
        enum_map::enum_map!(_ => vec![])
    }

    /// Indicates that [`IntrinsicInstrKind::Jmp`] takes two arguments, where the second is time.
    ///
    /// TH06 ANM has no time arg. (it always sets the script clock to the destination's time)
    fn jump_has_time_arg(&self) -> bool { true }

    /// Used by TH06 to indicate that an instruction must be the last instruction in the script.
    fn is_th06_anm_terminating_instr(&self, _opcode: u16) -> bool { false }

    // Most formats encode labels as offsets from the beginning of the script (in which case
    // these functions are trivial), but early STD is a special snowflake that writes the
    // instruction *index* instead.
    fn encode_label(&self, offset: usize) -> u32 { offset as _ }
    fn decode_label(&self, bits: u32) -> usize { bits as _ }
}

/// Helper to help implement `InstrFormat::read_instr`.
///
/// Reads `size` bytes into `size/4` dword arguments and sets their `is_var` flags according to
/// the parameter mask.  (it takes `size` instead of a count to help factor out divisibility checks,
/// as a size is often what you have to work with given the format)
pub fn read_dword_args_upto_size(
    f: &mut dyn BinRead,
    size: usize,
    mut param_mask: u16,
) -> ReadResult<Vec<InstrArg>> {
    if size % 4 != 0 {
        bail!("size not divisible by 4: {}", size);
    }
    let nargs = size/4;

    let out = (0..nargs).map(|_| {
        let bits = f.read_u32()?;
        let is_var = param_mask % 2 == 1;
        param_mask /= 2;
        Ok(InstrArg::Raw(RawArg { bits, is_var }))
    }).collect::<ReadResult<_>>()?;

    if param_mask != 0 {
        fast_warning!(
            "unused bits in param_mask! (arg {} is a variable, but there are only {} args!)",
            param_mask.trailing_zeros() + nargs as u32 + 1, nargs,
        );
    }
    Ok(out)
}

impl Instr {
    pub fn compute_param_mask(&self) -> Result<u16, SimpleError> {
        if self.args.len() > 16 {
            bail!("too many arguments in instruction!");
        }
        let mut mask = 0;
        for arg in self.args.iter().rev(){
            let bit = match *arg {
                InstrArg::Raw(RawArg { is_var, .. }) => is_var as u16,
                InstrArg::TimeOf(_) |
                InstrArg::Label(_) => 0,
                InstrArg::Local(_) => 1,
            };
            mask *= 2;
            mask += bit;
        }
        Ok(mask)
    }
}
