use bstr::{BString};

use crate::meta::Meta;
use crate::ident::Ident;
use crate::pos::Sp;

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
    pub items: Vec<Sp<Item>>,
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
    pub labels: Vec<Sp<StmtLabel>>,
    pub body: Sp<StmtBody>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtLabel {
    Label(Sp<Ident>),
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
        cond: Sp<Expr>,
        jump: StmtGoto,
    },
    Return {
        value: Option<Sp<Expr>>,
    },
    CondChain(StmtCondChain),
    While {
        is_do_while: bool,
        cond: Sp<Expr>,
        block: Block,
    },
    Times {
        count: Sp<Expr>,
        block: Block,
    },
    /// Expression followed by a semicolon.
    ///
    /// This is primarily for void-type "expressions" like raw instruction
    /// calls (which are grammatically indistinguishable from value-returning
    /// function calls), but may also represent a stack push in ECL.
    Expr(Sp<Expr>),
    Assignment {
        var: Var,
        op: AssignOpKind,
        value: Sp<Expr>,
    },
    Declaration {
        ty: VarDeclKeyword,
        vars: Vec<(Ident, Option<Sp<Expr>>)>,
    },
    /// An explicit subroutine call. (ECL only)
    ///
    /// Will always have at least one of either the `@` or `async`.
    /// (otherwise, it will fall under `Expr` instead)
    CallSub {
        at_symbol: bool,
        async_: Option<CallAsyncKind>,
        func: Ident,
        args: Vec<Sp<Expr>>,
    }
}

/// The body of a `goto` statement, without the `;`.
#[derive(Debug, Clone, PartialEq)]
pub struct StmtGoto {
    pub destination: Sp<Ident>,
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
    pub cond: Sp<Expr>,
    pub block: Block,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallAsyncKind {
    CallAsync,
    CallAsyncId(Box<Sp<Expr>>),
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
pub struct Block(pub Vec<Sp<Stmt>>);

// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Ternary {
        cond: Box<Sp<Expr>>,
        left: Box<Sp<Expr>>,
        right: Box<Sp<Expr>>,
    },
    Binop(Box<Sp<Expr>>, BinopKind, Box<Sp<Expr>>),
    Call {
        func: Ident,
        args: Vec<Sp<Expr>>,
    },
    Decrement {
        var: Var,
    },
    Unop(UnopKind, Box<Sp<Expr>>),
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

impl From<i32> for Expr {
    fn from(value: i32) -> Expr { Expr::LitInt { value, hex: false } }
}
impl From<f32> for Expr {
    fn from(value: f32) -> Expr { Expr::LitFloat { value } }
}

// =============================================================================

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

macro_rules! generate_visitor_stuff {
    ($Visit: ident $(,$mut: tt)?) => {
        /// Recursive AST traversal trait.
        pub trait $Visit {
            fn visit_item(&mut self, e: & $($mut)? Sp<Item>) { walk_item(self, e) }
            /// This is called only on the outermost blocks of each function.
            fn visit_func_body(&mut self, e: & $($mut)? Block) { walk_block(self, e) }
            fn visit_stmt(&mut self, e: & $($mut)? Sp<Stmt>) { walk_stmt(self, e) }
            fn visit_stmt_body(&mut self, e: & $($mut)? Sp<StmtBody>) { walk_stmt_body(self, e) }
            fn visit_expr(&mut self, e: & $($mut)? Sp<Expr>) { walk_expr(self, e) }
        }

        pub fn walk_script<V>(v: &mut V, x: & $($mut)? Script)
        where V: ?Sized + $Visit,
        {
            for item in & $($mut)? x.items {
                v.visit_item(item)
            }
        }

        pub fn walk_item<V>(v: &mut V, x: & $($mut)? Sp<Item>)
        where V: ?Sized + $Visit,
        {
            match & $($mut)? x.value {
                Item::Func {
                    code, inline: _, keyword: _, name: _, params: _,
                } => {
                    if let Some(code) = code {
                        v.visit_func_body(code);
                    }
                },
                Item::AnmScript { number: _, name: _, code } => {
                    v.visit_func_body(code);
                },
                Item::Meta { .. } => {},
                Item::FileList { .. } => {},
            }
        }

        pub fn walk_block<V>(v: &mut V, x: & $($mut)? Block)
        where V: ?Sized + $Visit,
        {
            for stmt in & $($mut)? x.0 {
                v.visit_stmt(stmt);
            }
        }

        pub fn walk_stmt<V>(v: &mut V, x: & $($mut)? Sp<Stmt>)
        where V: ?Sized + $Visit,
        {
            v.visit_stmt_body(& $($mut)? x.body);
        }

        pub fn walk_stmt_body<V>(v: &mut V, x: & $($mut)? Sp<StmtBody>)
        where V: ?Sized + $Visit,
        {
            match & $($mut)? x.value {
                StmtBody::Jump(_) => {},
                StmtBody::Return { value } => {
                    if let Some(value) = value {
                        v.visit_expr(value);
                    }
                },
                StmtBody::CondJump { cond, kind: _, jump: _ } => {
                    v.visit_expr(cond);
                },
                StmtBody::CondChain(chain) => {
                    let StmtCondChain { cond_blocks, else_block } = chain;
                    for CondBlock { cond, block, kind: _ } in cond_blocks {
                        v.visit_expr(cond);
                        walk_block(v, block);
                    }
                    if let Some(block) = else_block {
                        walk_block(v, block);
                    }
                },
                StmtBody::While { is_do_while: true, cond, block } => {
                    v.visit_expr(cond);
                    walk_block(v, block);
                },
                StmtBody::While { is_do_while: false, cond, block } => {
                    walk_block(v, block);
                    v.visit_expr(cond);
                },
                StmtBody::Times { count, block } => {
                    v.visit_expr(count);
                    walk_block(v, block);
                },
                StmtBody::Expr(e) => {
                    v.visit_expr(e);
                },
                StmtBody::Assignment { var: _, op: _, value } => {
                    v.visit_expr(value);
                },
                StmtBody::Declaration { ty: _, vars } => {
                    for (_ident, value) in vars {
                        if let Some(value) = value {
                            v.visit_expr(value);
                        }
                    }
                },
                StmtBody::CallSub { at_symbol: _, async_: _, func: _, args } => {
                    for arg in args {
                        v.visit_expr(arg);
                    }
                },
            }
        }

        pub fn walk_expr<V>(v: &mut V, e: & $($mut)? Sp<Expr>)
        where V: ?Sized + $Visit,
        {
            match & $($mut)? e.value {
                Expr::Ternary { cond, left, right } => {
                    v.visit_expr(cond);
                    v.visit_expr(left);
                    v.visit_expr(right);
                },
                Expr::Binop(a, _op, b) => {
                    v.visit_expr(a);
                    v.visit_expr(b);
                },
                Expr::Call { func: _, args } => {
                    for arg in args {
                        v.visit_expr(arg);
                    }
                },
                Expr::Decrement { var: _ } => {},
                Expr::Unop(_op, x) => v.visit_expr(x),
                Expr::LitInt { value: _, hex: _ } => {},
                Expr::LitFloat { value: _ } => {},
                Expr::LitString(_s) => {},
                Expr::Var(_v) => {},
            }
        }
    };
}

mod mut_ {
    use super::*;
    generate_visitor_stuff!(VisitMut, mut);
}
pub use self::mut_::{
    VisitMut,
    walk_script as walk_mut_script,
    walk_item as walk_mut_item,
    walk_block as walk_mut_block,
    walk_stmt as walk_mut_stmt,
    walk_stmt_body as walk_mut_stmt_body,
    walk_expr as walk_mut_expr,
};
mod ref_ {
    use super::*;
    generate_visitor_stuff!(Visit);
}
pub use self::ref_::{
    Visit, walk_script, walk_item, walk_block, walk_stmt,
    walk_stmt_body, walk_expr,
};
