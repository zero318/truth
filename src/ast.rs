use bstr::{BString};

use crate::meta::Meta;
use crate::ident::Ident;
use crate::pos::Spanned;

// Quick little util for stringly enums.
macro_rules! string_enum {
    (
        $(#[$($Enum_attr:tt)+])*
        $vis:vis enum $Enum:ident {
            $(
                $(#[doc = $variant_doc:literal])*
                #[str = $variant_str:literal] $Variant:ident,
            )*
        }
    ) => {
        $(#[$($Enum_attr)+])*
        $vis enum $Enum {
            $( $(#[doc = $variant_doc])* $Variant, )*
        }

        // used mainly for error messages
        impl ::std::fmt::Display for $Enum {
            fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                ::std::fmt::Display::fmt(match self {
                    $( $Enum::$Variant => $variant_str, )*
                }, f)
            }
        }

        impl crate::fmt::Format for $Enum {
            fn fmt<W: ::std::io::Write>(&self, out: &mut crate::fmt::Formatter<W>) -> crate::fmt::Result {
                out.fmt(format_args!("{}", self))
            }
        }
    }
}

// =============================================================================

/// Represents a complete script file.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Func {
        inline: bool,
        keyword: FuncKeyword,
        name: Ident,
        params: Vec<(VarDeclKeyword, Ident)>,
        /// `Some` for definitions, `None` for declarations.
        code: Option<Block>,
    },
    AnmScript {
        number: Option<i32>,
        name: Ident,
        code: Block,
    },
    Meta {
        keyword: MetaKeyword,
        name: Option<Ident>,
        meta: Meta,
    },
    FileList {
        keyword: FileListKeyword,
        files: Vec<LitString>
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FuncKeyword {
    Type(FuncReturnType),
    Sub,
    Timeline,
}

string_enum!{
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum FuncReturnType {
        #[str = "int"] Int,
        #[str = "float"] Float,
        #[str = "void"] Void,
    }
}

string_enum!{
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum FileListKeyword {
        #[str = "anim"] Anim,
        #[str = "ecli"] Ecli,
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum MetaKeyword {
        /// `entry` block for a texture in ANM.
        #[str = "entry"] Entry,
        #[str = "meta"] Meta,
    }
}

// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub time: i32,
    pub labels: Vec<StmtLabel>,
    pub body: StmtBody,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtLabel {
    Label(Ident),
    Difficulty {
        /// If `true`, the difficulty reverts to `"*"` after the next statement.
        temporary: bool,
        flags: DifficultyLabel,
    },
}

/// Represents a statement, including the ';' if required, but
/// without any labels.
#[derive(Debug, Clone, PartialEq)]
pub enum StmtBody {
    Jump(StmtGoto),
    CondJump {
        kind: CondKind,
        cond: Box<Expr>,
        jump: StmtGoto,
    },
    Return {
        value: Option<Box<Expr>>,
    },
    CondChain(StmtCondChain),
    While {
        is_do_while: bool,
        cond: Box<Expr>,
        block: Block,
    },
    Times {
        count: Box<Expr>,
        block: Block,
    },
    /// Expression followed by a semicolon.
    ///
    /// This is primarily for void-type "expressions" like raw instruction
    /// calls (which are grammatically indistinguishable from value-returning
    /// function calls), but may also represent a stack push in ECL.
    Expr(Box<Expr>),
    Assignment {
        var: Var,
        op: AssignOpKind,
        value: Box<Expr>,
    },
    Declaration {
        ty: VarDeclKeyword,
        vars: Vec<(Ident, Option<Box<Expr>>)>,
    },
    /// An explicit subroutine call. (ECL only)
    ///
    /// Will always have at least one of either the `@` or `async`.
    /// (otherwise, it will fall under `Expr` instead)
    CallSub {
        at_symbol: bool,
        async_: Option<CallAsyncKind>,
        func: Ident,
        args: Vec<Box<Expr>>,
    }
}

/// The body of a `goto` statement, without the `;`.
#[derive(Debug, Clone, PartialEq)]
pub struct StmtGoto {
    pub destination: Ident,
    pub time: Option<i32>,
}

// FIXME: This has been extracted just because the parser needs to build one incrementally.
//        Make a more sensible design.
#[derive(Debug, Clone, PartialEq)]
pub struct StmtCondChain {
    pub cond_blocks: Vec<CondBlock>,
    pub else_block: Option<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CondBlock {
    pub kind: CondKind,
    pub cond: Box<Expr>,
    pub block: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallAsyncKind {
    CallAsync,
    CallAsyncId(Box<Expr>),
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum CondKind {
        #[str = "if"] If,
        #[str = "unless"] Unless,
    }
}

// TODO: Parse
pub type DifficultyLabel = BString;

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum AssignOpKind {
        #[str = "="] Assign,
        #[str = "+="] Add,
        #[str = "-="] Sub,
        #[str = "*="] Mul,
        #[str = "/="] Div,
        #[str = "%="] Rem,
        #[str = "|="] BitOr,
        #[str = "^="] BitXor,
        #[str = "&="] BitAnd,
    }
}

/// A braced series of statements, typically written at an increased
/// indentation level.
#[derive(Debug, Clone, PartialEq)]
pub struct Block(pub Vec<Spanned<Stmt>>);

// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Ternary {
        cond: Box<Expr>,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Binop(Box<Expr>, BinopKind, Box<Expr>),
    Call {
        func: Ident,
        args: Vec<Box<Expr>>,
    },
    Decrement {
        var: Var,
    },
    Unop(UnopKind, Box<Expr>),
    LitInt {
        value: i32,
        /// A hint to the formatter that it should use hexadecimal.
        /// (may not necessarily represent the original radix of a parsed token)
        hex: bool,
    },
    LitFloat { value: f32 },
    LitString(LitString),
    Var(Var),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Var {
    Named {
        ty: Option<VarReadType>,
        ident: Ident,
    },
    Unnamed {
        ty: VarReadType,
        number: i32,
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum BinopKind {
        #[str = "+"] Add,
        #[str = "-"] Sub,
        #[str = "*"] Mul,
        #[str = "/"] Div,
        #[str = "%"] Rem,
        #[str = "=="] Eq,
        #[str = "!="] Ne,
        #[str = "<"] Lt,
        #[str = "<="] Le,
        #[str = ">"] Gt,
        #[str = ">="] Ge,
        #[str = "|"] BitOr,
        #[str = "^"] BitXor,
        #[str = "&"] BitAnd,
        #[str = "||"] LogicOr,
        #[str = "&&"] LogicAnd,
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum UnopKind {
        #[str = "!"] Not,
        #[str = "-"] Neg,
    }
}

impl UnopKind {
    pub fn eval_const_int(&self, x: i32) -> i32 {
        match self {
            UnopKind::Neg => i32::wrapping_neg(x),
            UnopKind::Not => (x != 0) as i32,
        }
    }
}

impl BinopKind {
    pub fn eval_const_int(&self, a: i32, b: i32) -> i32 {
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
}

impl Expr {
    pub fn const_eval_int(&self) -> Option<i32> {
        match self {
            &Expr::Call { .. } => None,
            &Expr::Ternary { ref cond, ref left, ref right } => {
                match cond.const_eval_int()? {
                    0 => right.const_eval_int(),
                    _ => left.const_eval_int(),
                }
            },
            &Expr::Binop(ref a, op, ref b) => Some(op.eval_const_int(a.const_eval_int()?, b.const_eval_int()?)),
            &Expr::Unop(op, ref x) => Some(op.eval_const_int(x.const_eval_int()?)),
            &Expr::Decrement { .. } => None,
            &Expr::LitInt { value, hex: _ } => Some(value),
            &Expr::LitFloat { .. } => None,
            &Expr::LitString(_) => None,
            &Expr::Var(_) => None,
        }
    }
}

impl Var {
    pub fn ty(&self) -> Option<VarReadType> {
        match self {
            &Var::Unnamed { ty, .. } => Some(ty),
            &Var::Named { ty, .. } => ty,
        }
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum VarDeclKeyword {
        #[str = "int"] Int,
        #[str = "float"] Float,
        #[str = "var"] Var,
    }
}

impl From<i32> for Box<Expr> {
    fn from(value: i32) -> Box<Expr> { Box::new(Expr::LitInt { value, hex: false })}
}
impl From<f32> for Box<Expr> {
    fn from(value: f32) -> Box<Expr> { Box::new(Expr::LitFloat { value })}
}

// =============================================================================

/// The hinted type of a variable at a usage site.
///
/// E.g. a variable's type may be hinted with the use of `$` or `%` prefixes.
/// (or it might not be hinted, meaning its type must be determined through other means)
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VarReadType {
    Int,
    Float,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LitString<S=BString> {
    pub string: S,
}
