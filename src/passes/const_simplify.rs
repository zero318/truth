//! Constant expression simplification pass.
//!
//! This pass identifies expressions in the AST that can be evaluated at compile-time and simplifies
//! them.  Expressions that cannot be simplified (e.g. calls of non-const functions or use of
//! non-const variables) will be left as-is.
//!
//! This is a crucial part of STD compilation, as STD has no mechanism for using variables at
//! runtime.  For other formats, it is moreso just an optimization.
//!
//! Use [`Visitor`]'s implementation of [`VisitMut`] to apply the pass. Call [`Visitor::finish`]
//! at the end to obtain errors; These will mostly be type errors that prevent evaluation of an
//! operation that could otherwise be computed at compile-time.
//!
//! # Example
//! ```
//! use ecl_parser::{Parse, VisitMut, Expr, pos::{Files, Sp}};
//! use ecl_parser::passes::const_simplify;
//!
//! let mut files = Files::new();
//!
//! let text = b"(3 == 3) ? (3.0 + 0.5) * x : 4";
//! let mut expr: Sp<Expr> = files.parse("<input>", text).unwrap();
//!
//! let mut visitor = const_simplify::Visitor::new();
//! visitor.visit_expr(&mut expr);
//! visitor.finish().expect("failed to simplify");
//!
//! let text_simplified = b"3.5 * x";
//! let expected: Sp<Expr> = files.parse("<input>", text_simplified).unwrap();
//! assert_eq!(expr, expected);
//! ```

use crate::value::ScalarValue;
use crate::ast::{self, VisitMut, UnopKind, BinopKind, Expr};
use crate::error::{CompileError};
use crate::pos::Sp;

impl Sp<UnopKind> {
    pub fn const_eval(&self, b: Sp<ScalarValue>) -> Result<ScalarValue, CompileError> {
        self.type_check(b.ty(), b.span)?;
        match b.value {
            ScalarValue::Int(b) => Ok(ScalarValue::Int(self.const_eval_int(b))),
            ScalarValue::Float(b) => Ok(ScalarValue::Float(self.const_eval_float(b).expect("(bug!) type_check should fail..."))),
        }
    }
}

impl UnopKind {
    pub fn const_eval_int(&self, x: i32) -> i32 {
        match self {
            UnopKind::Neg => i32::wrapping_neg(x),
            UnopKind::Not => (x != 0) as i32,
        }
    }

    pub fn const_eval_float(&self, x: f32) -> Option<f32> {
        match self {
            UnopKind::Neg => Some(-x),
            UnopKind::Not => None,
        }
    }
}

impl Sp<BinopKind> {
    pub fn const_eval(&self, a: Sp<ScalarValue>, b: Sp<ScalarValue>) -> Result<ScalarValue, CompileError> {
        self.type_check(a.ty(), b.ty(), (a.span, b.span))?;
        match (a.value, b.value) {
            (ScalarValue::Int(a), ScalarValue::Int(b)) => Ok(ScalarValue::Int(self.const_eval_int(a, b))),
            (ScalarValue::Float(a), ScalarValue::Float(b)) => Ok(self.const_eval_float(a, b).expect("(bug!) type_check should fail...")),
            _ => unreachable!("(bug!) type_check should fail..."),
        }
    }
}

impl BinopKind {
    pub fn const_eval_int(&self, a: i32, b: i32) -> i32 {
        match self {
            BinopKind::Add => i32::wrapping_add(a, b),
            BinopKind::Sub => i32::wrapping_sub(a, b),
            BinopKind::Mul => i32::wrapping_mul(a, b),
            BinopKind::Div => i32::wrapping_div(a, b),
            BinopKind::Rem => i32::wrapping_rem(a, b),
            BinopKind::Eq => (a == b) as i32,
            BinopKind::Ne => (a != b) as i32,
            BinopKind::Lt => (a < b) as i32,
            BinopKind::Le => (a <= b) as i32,
            BinopKind::Gt => (a > b) as i32,
            BinopKind::Ge => (a >= b) as i32,
            BinopKind::LogicOr => if a == 0 { b } else { a },
            BinopKind::LogicAnd => if a == 0 { 0 } else { b },
            BinopKind::BitXor => a ^ b,
            BinopKind::BitAnd => a & b,
            BinopKind::BitOr => a | b,
        }
    }

    pub fn const_eval_float(&self, a: f32, b: f32) -> Option<ScalarValue> {
        match self {
            BinopKind::Add => Some(ScalarValue::Float(a + b)),
            BinopKind::Sub => Some(ScalarValue::Float(a - b)),
            BinopKind::Mul => Some(ScalarValue::Float(a * b)),
            BinopKind::Div => Some(ScalarValue::Float(a / b)),
            BinopKind::Rem => Some(ScalarValue::Float(a % b)),
            BinopKind::Eq => Some(ScalarValue::Int((a == b) as i32)),
            BinopKind::Ne => Some(ScalarValue::Int((a != b) as i32)),
            BinopKind::Lt => Some(ScalarValue::Int((a < b) as i32)),
            BinopKind::Le => Some(ScalarValue::Int((a <= b) as i32)),
            BinopKind::Gt => Some(ScalarValue::Int((a > b) as i32)),
            BinopKind::Ge => Some(ScalarValue::Int((a >= b) as i32)),
            BinopKind::LogicOr => None,
            BinopKind::LogicAnd => None,
            BinopKind::BitXor => None,
            BinopKind::BitAnd => None,
            BinopKind::BitOr => None,
        }
    }
}

/// Visitor for const simplification.
///
/// See the [the module-level documentation][self] for more details.
pub struct Visitor {
    errors: CompileError,
}

impl Visitor {
    pub fn new() -> Self {
        Visitor { errors: CompileError::new_empty() }
    }

    pub fn finish(self) -> Result<(), CompileError> {
        self.errors.into_result(())
    }
}

impl VisitMut for Visitor {
    fn visit_expr(&mut self, e: &mut Sp<Expr>) {
        // simplify subexpressions first
        ast::walk_mut_expr(self, e);

        // now inspect this expression
        match &e.value {
            Expr::Unop(op, b) => {
                let b_const = match b.as_const() {
                    Some(b_value) => sp!(b.span => b_value),
                    _ => return, // can't simplify if subexpr is not const
                };

                match op.const_eval(b_const) {
                    Ok(new_value) => *e = sp!(e.span => new_value.into()),
                    Err(e) => {
                        self.errors.append(e);
                        return;
                    }
                }
            },

            Expr::Binop(a, op, b) => {
                let (a_const, b_const) = match (a.as_const(), b.as_const()) {
                    (Some(a_value), Some(b_value)) => (sp!(a.span => a_value), sp!(b.span => b_value)),
                    _ => return, // can't simplify if any subexpr is not const
                };

                match op.const_eval(a_const, b_const) {
                    Ok(new_value) => *e = sp!(e.span => new_value.into()),
                    Err(e) => {
                        self.errors.append(e);
                        return;
                    }
                }
            },

            Expr::Ternary { cond, left, right } => match cond.as_const() {
                // FIXME it should be possible to move somehow instead of cloning here...
                Some(ScalarValue::Int(0)) => e.value = (***right).clone(),
                Some(ScalarValue::Int(_)) => e.value = (***left).clone(),
                Some(_) => {
                    self.errors.append(error!(
                        message("type error"),
                        primary(cond, "ternary condition must be an integer")
                    ));
                    return;
                },
                _ => return, // can't simplify if subexpr is not const
            },
            _ => return, // can't simplify other expressions
        }
    }
}
