use std::fmt;

use crate::raw;
use crate::resolve::{DefId, LoopId, NodeId, RegId};
use crate::ident::{Ident, ResIdent};
use crate::pos::{Sp, Span};
use crate::game::LanguageKey;
use crate::value;
use crate::diff_switch_utils as ds_util;

pub use meta::Meta;
pub mod meta;

pub mod pseudo;

// =============================================================================

/// Type used in the AST for the span of a single token with no useful data.
///
/// This can be used for things like keywords.  We use [`Sp`]`<()>` instead of [`Span`]
/// because the latter would have an impact on equality tests.
pub type TokenSpan = Sp<()>;

/// Represents a complete script file.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptFile {
    pub mapfiles: Vec<Sp<LitString>>,
    pub image_sources: Vec<Sp<LitString>>,
    pub items: Vec<Sp<Item>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Func(ItemFunc),
    AnmScript {
        keyword: TokenSpan,
        number: Option<Sp<raw::LangInt>>,
        ident: Sp<Ident>,  // not `ResIdent` because it doesn't define something in all languages
        code: Block,
    },
    Timeline {
        keyword: TokenSpan,
        number: Option<Sp<raw::LangInt>>,
        ident: Option<Sp<Ident>>,
        code: Block,
    },
    Meta {
        keyword: Sp<MetaKeyword>,
        fields: Sp<meta::Fields>,
    },
    ConstVar {
        ty_keyword: Sp<TypeKeyword>,
        vars: Vec<Sp<(Sp<Var>, Sp<Expr>)>>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ItemFunc {
    pub qualifier: Option<Sp<FuncQualifier>>,
    pub ty_keyword: Sp<TypeKeyword>,
    pub ident: Sp<ResIdent>,
    pub params: Vec<Sp<FuncParam>>,
    /// `Some` for definitions, `None` for declarations.
    pub code: Option<Block>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncParam {
    pub qualifier: Option<Sp<ParamQualifier>>,
    pub ty_keyword: Sp<TypeKeyword>,
    pub ident: Option<Sp<ResIdent>>,
}

string_enum! {
    // 'const' parameters will exist *some day*, this is here now to reduce churn.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum ParamQualifier {}
}

impl Item {
    pub fn descr(&self) -> &'static str { match self {
        Item::Func(ItemFunc { qualifier: Some(sp_pat![token![const]]), .. }) => "const function definition",
        Item::Func(ItemFunc { qualifier: Some(sp_pat![token![inline]]), .. }) => "inline function definition",
        Item::Func(ItemFunc { qualifier: None, .. }) => "exported function definition",
        Item::AnmScript { .. } => "script",
        Item::Timeline { .. } => "timeline",
        Item::Meta { .. } => "meta",
        Item::ConstVar { .. } => "const definition",
    }}
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum FuncQualifier {
        /// `entry` block for a texture in ANM.
        #[strum(serialize = "const")] Const,
        #[strum(serialize = "inline")] Inline,
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum MetaKeyword {
        /// `entry` block for a texture in ANM.
        #[strum(serialize = "entry")] Entry,
        #[strum(serialize = "meta")] Meta,
    }
}

// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub node_id: Option<NodeId>,
    pub diff_label: Option<Sp<DiffLabel>>,
    pub kind: StmtKind,
}

/// Difficulty label. `{"ENH"}:`
#[derive(Debug, Clone, PartialEq)]
pub struct DiffLabel {
    /// Cached bitflag form of the difficulty mask.  This may be `None` before
    /// [`crate::passes::resolution::compute_diff_label_masks`] runs.
    pub mask: Option<crate::bitset::BitSet32>,
    pub string: Sp<LitString>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    /// Some items are allowed to appear as statements. (`const`s and functions)
    Item(Box<Sp<Item>>),

    /// Unconditional goto.  `goto label @ time;`
    Jump(StmtJumpKind),

    /// Conditional goto.  `if (a == b) goto label @ time;`
    CondJump {
        keyword: Sp<CondKeyword>,
        cond: Sp<Expr>,
        jump: StmtJumpKind,
    },

    /// `return;` or `return expr;`
    Return {
        keyword: TokenSpan,
        value: Option<Sp<Expr>>,
    },

    /// A chain of conditional blocks.  `if (...) { ... } else if (...) { ... } else { ... }`
    CondChain(StmtCondChain),

    /// Unconditional loop.  `loop { ... }`
    Loop {
        loop_id: Option<LoopId>,
        keyword: TokenSpan,
        block: Block,
    },

    /// While loop.  `while (...) { ... }` or `do { ... } while (...);`
    While {
        loop_id: Option<LoopId>,
        while_keyword: TokenSpan,
        do_keyword: Option<TokenSpan>,
        cond: Sp<Expr>,
        block: Block,
    },

    /// Times loop.  `times(n) { ... }`
    Times {
        loop_id: Option<LoopId>,
        keyword: TokenSpan,
        clobber: Option<Sp<Var>>,
        count: Sp<Expr>,
        block: Block,
    },

    /// Expression followed by a semicolon.
    ///
    /// This is primarily for void-type "expressions" like raw instruction
    /// calls (which are grammatically indistinguishable from value-returning
    /// function calls).
    Expr(Sp<Expr>),

    /// Free-standing block.
    Block(Block),

    /// `a = expr;` or `a += expr;`
    Assignment {
        var: Sp<Var>,
        op: Sp<AssignOpKind>,
        value: Sp<Expr>,
    },

    /// Local variable declaration `int a = 20;`. (`const` vars fall under [`Self::Item`] instead)
    Declaration {
        ty_keyword: Sp<TypeKeyword>,
        vars: Vec<Sp<(Sp<Var>, Option<Sp<Expr>>)>>,
    },

    /// An explicit subroutine call. (ECL only)
    ///
    /// Will always have at least one of either the `@` or `async`.
    /// (otherwise, it will fall under `Expr` instead)
    CallSub {
        at_symbol: bool,
        async_: Option<CallAsyncKind>,
        func: Sp<Ident>,
        args: Vec<Sp<Expr>>,
    },

    /// An interrupt label: `interrupt[2]:`.
    InterruptLabel(Sp<raw::LangInt>),

    /// An absolute time label: `30:` or `-30:`.
    AbsTimeLabel(Sp<raw::LangInt>),

    /// A relative time label: `+30:`.  This value cannot be negative.
    RelTimeLabel {
        delta: Sp<raw::LangInt>,
        // This is used during decompilation ONLY, in order to allow the code formatter to
        // write `//` comments with the total time on these labels without having to perform
        // its own semantic analysis.
        _absolute_time_comment: Option<raw::LangInt>,
    },

    /// A label `label:` that can be jumped to.
    Label(Sp<Ident>),

    /// A virtual statement that marks the end of a variable's lexical scope.
    ///
    /// Blocks are eliminated during early compilation passes, leaving behind these as the only
    /// remaining way of identifying the end of a variable's scope.  They are used during lowering
    /// to determine when to release resources (like registers) held by locals.
    ScopeEnd(DefId),

    /// A virtual statement that completely disappears during compilation.
    ///
    /// This is a trivial statement that doesn't even compile to a `nop();`.
    /// It is inserted at the beginning and end of code blocks in order to help implement some
    /// of the following things:
    ///
    /// * A time label at the end of a block.
    /// * A time label at the beginning of a loop's body.
    ///
    /// Note that these may also appear in the middle of a block in the AST if a transformation
    /// pass has e.g. inlined the contents of one block into another.
    NoInstruction,
}

impl StmtKind {
    pub fn descr(&self) -> &'static str { match self {
        StmtKind::Item(item) => item.descr(),
        StmtKind::Jump(StmtJumpKind::Goto { .. }) => "goto",
        StmtKind::Jump(StmtJumpKind::BreakContinue { .. }) => "loop jump",
        StmtKind::CondJump { jump: StmtJumpKind::Goto { .. }, .. } => "conditional goto",
        StmtKind::CondJump { jump: StmtJumpKind::BreakContinue { .. }, .. } => "conditional loop jump",
        StmtKind::Return { .. } => "return statement",
        StmtKind::CondChain { .. } => "conditional chain",
        StmtKind::Loop { .. } => "loop",
        StmtKind::While { .. } => "while(..)",
        StmtKind::Times { .. } => "times(..)",
        StmtKind::Expr { .. } => "expression statement",
        StmtKind::Block { .. } => "block statement",
        StmtKind::Assignment { .. } => "assignment",
        StmtKind::Declaration { .. } => "var declaration",
        StmtKind::CallSub { .. } => "sub call",
        StmtKind::InterruptLabel { .. } => "interrupt label",
        StmtKind::AbsTimeLabel { .. } => "time label",
        StmtKind::RelTimeLabel { .. } => "time label",
        StmtKind::Label { .. } => "label",
        StmtKind::ScopeEnd { .. } => "<ScopeEnd>",
        StmtKind::NoInstruction { .. } => "<NoInstruction>",
    }}
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtJumpKind {
    /// The body of a `goto` statement, without the `;`.
    Goto(StmtGoto) ,
    /// A `break` or `continue`.
    BreakContinue {
        keyword: Sp<BreakContinueKeyword>,
        /// This is used to prevent or detect bugs where a `break` or `continue` could somehow
        /// end up referring to the wrong loop after a code transformation.
        loop_id: Option<LoopId>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct StmtGoto {
    pub destination: Sp<Ident>,
    pub time: Option<Sp<raw::LangInt>>,
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum BreakContinueKeyword {
        #[strum(serialize = "break")] Break,
        // `continue` could be implemented in a variety of ways and there could be confusing
        // inconsistencies between loops and while loops.  Hold off on it for now...
        // #[strum(serialize = "continue")] Continue,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StmtCondChain {
    pub cond_blocks: Vec<CondBlock>,
    pub else_block: Option<Block>,
}

impl StmtCondChain {
    pub fn last_block(&self) -> &Block {
        self.else_block.as_ref()
            .unwrap_or_else(|| &self.cond_blocks.last().unwrap().block)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CondBlock {
    pub keyword: Sp<CondKeyword>,
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
    pub enum CondKeyword {
        #[strum(serialize = "if")] If,
        #[strum(serialize = "unless")] Unless,
    }
}

impl CondKeyword {
    pub fn negate(self) -> Self { match self {
        CondKeyword::If => CondKeyword::Unless,
        CondKeyword::Unless => CondKeyword::If,
    }}
}

// TODO: Parse
pub type DifficultyLabel = String;

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[derive(strum::EnumIter)]
    pub enum AssignOpKind {
        #[strum(serialize = "=")] Assign,
        #[strum(serialize = "+=")] Add,
        #[strum(serialize = "-=")] Sub,
        #[strum(serialize = "*=")] Mul,
        #[strum(serialize = "/=")] Div,
        #[strum(serialize = "%=")] Rem,
        #[strum(serialize = "|=")] BitOr,
        #[strum(serialize = "^=")] BitXor,
        #[strum(serialize = "&=")] BitAnd,
        #[strum(serialize = "<<=")] ShiftLeft,
        #[strum(serialize = ">>=")] ShiftRightSigned,
        #[strum(serialize = ">>>=")] ShiftRightUnsigned,
    }
}

impl AssignOpKind {
    pub fn class(self) -> OpClass {
        match self {
            Self::Assign => OpClass::DirectAssignment,
            Self::Add => OpClass::Arithmetic,
            Self::Sub => OpClass::Arithmetic,
            Self::Mul => OpClass::Arithmetic,
            Self::Div => OpClass::Arithmetic,
            Self::Rem => OpClass::Arithmetic,
            Self::BitOr => OpClass::Bitwise,
            Self::BitXor => OpClass::Bitwise,
            Self::BitAnd => OpClass::Bitwise,
            Self::ShiftLeft => OpClass::Shift,
            Self::ShiftRightSigned => OpClass::Shift,
            Self::ShiftRightUnsigned => OpClass::Shift,
        }
    }

    /// Iterate over all assign ops.
    pub fn iter() -> impl Iterator<Item=AssignOpKind> {
        <Self as strum::IntoEnumIterator>::iter()
    }

    pub fn corresponding_binop(self) -> Option<BinOpKind> {
        match self {
            token![=] => None,
            token![+=] => Some(token![+]),
            token![-=] => Some(token![-]),
            token![*=] => Some(token![*]),
            token![/=] => Some(token![/]),
            token![%=] => Some(token![%]),
            token![|=] => Some(token![|]),
            token![^=] => Some(token![^]),
            token![&=] => Some(token![&]),
            token![<<=] => Some(token![<<]),
            token![>>=] => Some(token![>>]),
            token![>>>=] => Some(token![>>>]),
        }
    }
}

/// A braced series of statements, typically written at an increased indentation level.
///
/// Every Block always has at least two [`Stmt`]s, as on creation it is bookended by dummy
/// statements to ensure it has a well-defined start and end time.
#[derive(Debug, Clone, PartialEq)]
pub struct Block(pub Vec<Sp<Stmt>>);

impl Block {
    /// Node ID for looking up e.g. the time label before the first statement in the loop body.
    pub fn start_node_id(&self) -> NodeId { self.first_stmt().node_id.unwrap() }

    /// Node ID for looking up e.g. the time label after the final statement in the loop body.
    pub fn end_node_id(&self) -> NodeId { self.last_stmt().node_id.unwrap() }

    /// Zero-length span at beginning of block interior.
    pub fn start_span(&self) -> Span { self.first_stmt().span.start_span() }

    /// Zero-length span at end of block interior.
    pub fn end_span(&self) -> Span { self.last_stmt().span.end_span() }

    pub fn first_stmt(&self) -> &Sp<Stmt> {
        self.0.get(0).expect("(bug?) unexpected empty block!")
    }

    pub fn last_stmt(&self) -> &Sp<Stmt> {
        self.0.last().expect("(bug?) unexpected empty block!")
    }
}

// =============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Ternary {
        cond: Box<Sp<Expr>>,
        question: Sp<()>,
        left: Box<Sp<Expr>>,
        colon: Sp<()>,
        right: Box<Sp<Expr>>,
    },
    BinOp(Box<Sp<Expr>>, Sp<BinOpKind>, Box<Sp<Expr>>),
    UnOp(Sp<UnOpKind>, Box<Sp<Expr>>),
    /// `--` or `++`.
    XcrementOp {
        op: Sp<XcrementOpKind>,
        order: XcrementOpOrder,
        var: Sp<Var>,
    },
    Var(Sp<Var>),
    Call(ExprCall),
    /// (a:b:c:d) syntax.  Always has at least two items.  Items aside from the first may be omitted (None).
    DiffSwitch(ds_util::DiffSwitchVec<Sp<Expr>>),
    LitInt {
        value: raw::LangInt,
        /// A hint to the formatter on how it should write the integer.
        /// (may not necessarily represent the original radix of a parsed token)
        radix: IntRadix,
    },
    LitFloat { value: raw::LangFloat },
    LitString(LitString),
    LabelProperty {
        label: Sp<Ident>,
        keyword: Sp<LabelPropertyKeyword>
    },
    EnumConst {
        enum_name: Sp<Ident>,
        ident: Sp<ResIdent>,
    },
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IntRadix {
    /// Display as decimal.
    Dec,
    /// Display as hexadecimal, with an `0x` prefix.
    Hex,
    /// Display as potentially negative hexadecimal, with an `0x` prefix.
    SignedHex,
    /// Display as binary, with an `0b` prefix.
    Bin,
    /// Use `true` and `false` if the value is `1` or `0`.  Otherwise, fall back to decimal.
    Bool,
}

impl Expr {
    pub fn int_of_ty(value: i32, ty: value::ScalarType) -> Self { match ty {
        value::ScalarType::Int => value.into(),
        value::ScalarType::Float => (value as f32).into(),
        value::ScalarType::String => panic!("Expr::zero() called on type String"),
    }}
    pub fn descr(&self) -> &'static str { match self {
        Expr::Ternary { .. } => "ternary",
        Expr::BinOp { .. } => "binary operator",
        Expr::UnOp { .. } => "unary operator",
        Expr::XcrementOp { .. } => "in-place operator",
        Expr::Call { .. } => "call expression",
        Expr::DiffSwitch { .. } => "difficulty switch",
        Expr::LitInt { .. } => "literal integer",
        Expr::LabelProperty { .. } => "label property",
        Expr::LitFloat { .. } => "literal float",
        Expr::LitString { .. } => "literal string",
        Expr::EnumConst { .. } => "enum const",
        Expr::Var { .. } => "var expression",
    }}
    pub fn can_lower_to_immediate(&self) -> bool { match self {
        Expr::LabelProperty { .. } => true,
        Expr::LitInt { .. } => true,
        Expr::LitFloat { .. } => true,
        Expr::LitString { .. } => true,
        // FIXME: this just plain feels funny, but it works for the current uses of this function.
        //        There is an integration test to ensure we run into problems if we try to use
        //        this to validate 'const' function args
        Expr::DiffSwitch(cases) => {
            cases.iter().flat_map(|opt| opt.as_ref())
                .all(|case| case.can_lower_to_immediate())
        },
        _ => false,
    }}
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExprCall {
    // note: deliberately called 'name' instead of 'ident' so that you can
    //       match both this and the inner ident without shadowing
    pub name: Sp<CallableName>,
    /// Args beginning with `@` that represent raw pieces of an instruction.
    pub pseudos: Vec<Sp<PseudoArg>>,
    /// Positional args.  One should check [`Self::actually_has_args`] before using this.
    pub args: Vec<Sp<Expr>>,
}

impl ExprCall {
    /// Extract a `@blob` pseudo-arg if one exists.
    pub fn blob(&self) -> Option<&Sp<PseudoArg>> {
        self.pseudos.iter().find(|p| matches!(p.kind.value, token![blob]))
    }

    /// This returns `true` if [`Self::args`] actually represents a list of arguments.
    ///
    /// A counter-example would be a call with `@blob`.  Such a function call will have an
    /// empty argument list, but should not be misinterpreted as a call with 0 arity.
    pub fn actually_has_args(&self) -> bool {
        self.blob().is_none()
    }
}

/// An identifier in a function call.
///
/// Raw instructions (`ins_`) are separately recognized so that they don't have to take part
/// in name resolution.  This makes it easier to use the AST VM [`crate::vm::AstVm`].
#[derive(Debug, Clone, PartialEq)]
pub enum CallableName {
    Normal {
        ident: ResIdent,
        /// This field is `None` until initialized by [`crate::passes::assign_languages`] (after which it may still be `None`).
        ///
        /// It is only here so that name resolution can consider instruction aliases; nothing else should ever need it,
        /// as all useful information can be found through the resolved [`DefId`].
        language_if_ins: Option<LanguageKey>,
    },
    Ins {
        opcode: u16,
        /// This field is `None` until initialized by [`crate::passes::assign_languages`] (after which it is guaranteed to be `Some`).
        /// It exists to help a variety of other passes look up e.g. type info about raw registers.
        ///
        /// Notably, in ECL, some of these may be set to [`InstrLanguage::Timeline`] instead of [`InstrLanguage::ECL`].
        language: Option<LanguageKey>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    /// A variable mentioned by name, possibly with a type sigil.
    pub ty_sigil: Option<VarSigil>,
    pub name: VarName,
}

impl VarName {
    /// Construct from the identifier of a local, parameter, or constant. (but NOT a register alias)
    pub fn new_non_reg(ident: ResIdent) -> Self {
        VarName::Normal { ident, language_if_reg: None }
    }

    /// For use internally by the parser on idents that may or may not be register aliases.
    ///
    /// Code outside of the parser should not use this; it should instead use [`Self::new_non_register`] if the
    /// ident is known not to refer to a register, else it should construct one manually with the correct language.
    pub(crate) fn from_parsed_ident(ident: ResIdent) -> Self {
        // for the parser, it is okay to use 'language_if_reg: None' on register aliases because a later pass will fill it in.
        VarName::Normal { ident, language_if_reg: None }
    }

    pub fn is_named(&self) -> bool { match self {
        VarName::Normal { .. } => true,
        VarName::Reg { .. } => false,
    }}

    /// Panic if it's not a normal ident.  This should be safe to call on vars in declarations.
    #[track_caller]
    pub fn expect_ident(&self) -> &ResIdent { match self {
        VarName::Normal { ident, .. } => ident,
        VarName::Reg { .. } => panic!("unexpected register"),
    }}
}

#[derive(Debug, Clone, PartialEq)]
pub enum VarName {
    Normal {
        ident: ResIdent,
        /// This field is `None` until initialized by [`crate::passes::assign_languages`] (after which it may still be `None`).
        ///
        /// It is only here so that name resolution can consider register aliases; nothing else should ever need it,
        /// as all useful information can be found through the resolved [`DefId`].
        language_if_reg: Option<LanguageKey>,
    },
    Reg {
        reg: RegId,
        /// This field is `None` until initialized by [`crate::passes::assign_languages`] (after which it is guaranteed to be `Some`).
        /// It exists to help a variety of other passes look up e.g. type info about raw instructions.
        language: Option<LanguageKey>,
    },
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[derive(strum::EnumIter)]
    pub enum BinOpKind {
        #[strum(serialize = "+")] Add,
        #[strum(serialize = "-")] Sub,
        #[strum(serialize = "*")] Mul,
        #[strum(serialize = "/")] Div,
        #[strum(serialize = "%")] Rem,
        #[strum(serialize = "==")] Eq,
        #[strum(serialize = "!=")] Ne,
        #[strum(serialize = "<")] Lt,
        #[strum(serialize = "<=")] Le,
        #[strum(serialize = ">")] Gt,
        #[strum(serialize = ">=")] Ge,
        #[strum(serialize = "|")] BitOr,
        #[strum(serialize = "^")] BitXor,
        #[strum(serialize = "&")] BitAnd,
        #[strum(serialize = "||")] LogicOr,
        #[strum(serialize = "&&")] LogicAnd,
        #[strum(serialize = "<<")] ShiftLeft,
        #[strum(serialize = ">>")] ShiftRightSigned,
        #[strum(serialize = ">>>")] ShiftRightUnsigned,
    }
}

impl BinOpKind {
    pub fn class(self) -> OpClass {
        use BinOpKind as B;

        match self {
            B::Add | B::Sub | B::Mul | B::Div | B::Rem => OpClass::Arithmetic,
            B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge => OpClass::Comparison,
            B::BitOr | B::BitXor | B::BitAnd => OpClass::Bitwise,
            B::LogicOr | B::LogicAnd => OpClass::Logical,
            B::ShiftLeft | B::ShiftRightSigned | B::ShiftRightUnsigned => OpClass::Shift,
        }
    }

    pub fn is_comparison(self) -> bool {
        self.class() == OpClass::Comparison
    }

    /// Iterate over all binops.
    pub fn iter() -> impl Iterator<Item=BinOpKind> {
        <Self as strum::IntoEnumIterator>::iter()
    }

    /// Iterate over the comparison ops.
    pub fn iter_comparison() -> impl Iterator<Item=BinOpKind> {
        Self::iter().filter(|&op| Self::is_comparison(op))
    }

    pub fn negate_comparison(self) -> Option<BinOpKind> { match self {
        token![==] => Some(token![!=]),
        token![!=] => Some(token![==]),
        token![<=] => Some(token![>]),
        token![>=] => Some(token![<]),
        token![<] => Some(token![>=]),
        token![>] => Some(token![<=]),
        _ => None,
    }}
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[derive(strum::EnumIter)]
    pub enum UnOpKind {
        #[strum(serialize = "!")] Not,
        #[strum(serialize = "-")] Neg,
        #[strum(serialize = "~")] BitNot,
        #[strum(serialize = "sin")] Sin,
        #[strum(serialize = "cos")] Cos,
        #[strum(serialize = "sqrt")] Sqrt,
        #[strum(serialize = "$")] EncodeI,
        #[strum(serialize = "%")] EncodeF,
        #[strum(serialize = "int")] CastI,
        #[strum(serialize = "float")] CastF,
    }
}

impl UnOpKind {
    pub fn class(&self) -> OpClass {
        match self {
            UnOpKind::Not => OpClass::Logical,
            UnOpKind::Neg => OpClass::Arithmetic,
            UnOpKind::BitNot => OpClass::Bitwise,
            UnOpKind::Sin => OpClass::FloatMath,
            UnOpKind::Cos => OpClass::FloatMath,
            UnOpKind::Sqrt => OpClass::FloatMath,
            UnOpKind::CastI => OpClass::Cast,
            UnOpKind::CastF => OpClass::Cast,
            UnOpKind::EncodeI => OpClass::TySigil,
            UnOpKind::EncodeF => OpClass::TySigil,
        }
    }

    /// Iterate over all unops.
    pub fn iter() -> impl Iterator<Item=UnOpKind> {
        <Self as strum::IntoEnumIterator>::iter()
    }

    /// Convert the `%` and `$` operators into var sigils.
    pub fn as_ty_sigil(&self) -> Option<VarSigil> {
        match self {
            token![unop $] => Some(token![$]),
            token![unop %] => Some(token![%]),
            _ => None,
        }
    }

    /// Convert the `%`, `$`, `int`, and `float` operators into var sigils.
    pub fn as_ty_sigil_with_auto_cast(&self) -> Option<VarSigil> {
        match self {
            token![unop int] => Some(token![$]),
            token![unop float] => Some(token![%]),
            _ => self.as_ty_sigil(),
        }
    }

    pub fn is_cast_of_type(&self, ty: value::ScalarType) -> bool {
        match (self, ty) {
            (token![unop int], value::ScalarType::Int) => true,
            (token![unop float], value::ScalarType::Float) => true,
            _ => false,
        }
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[derive(strum::EnumIter)]
    pub enum XcrementOpKind {
        #[strum(serialize="++")] Inc,
        #[strum(serialize="--")] Dec,
    }
}

impl XcrementOpKind {
    /// Iterate over all xcrement ops.
    pub fn iter() -> impl Iterator<Item=XcrementOpKind> {
        <Self as strum::IntoEnumIterator>::iter()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[derive(strum::EnumIter)]
pub enum XcrementOpOrder { Pre, Post }

impl XcrementOpOrder {
    /// Iterate over all xcrement orders.
    pub fn iter() -> impl Iterator<Item=XcrementOpOrder> {
        <Self as strum::IntoEnumIterator>::iter()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OpClass {
    Arithmetic,
    Comparison,
    Bitwise,
    Shift,
    Logical,
    FloatMath,
    TySigil,
    Cast,
    DirectAssignment,
}

impl OpClass {
    pub fn descr(&self) -> &'static str {
        match self {
            Self::Arithmetic => "arithmetic",
            Self::Comparison => "comparison",
            Self::Bitwise => "bitwise",
            Self::Shift => "shift",
            Self::Logical => "logical",
            Self::FloatMath => "mathematical",
            Self::Cast => "cast",
            Self::TySigil => "sigil",
            Self::DirectAssignment => "direct",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PseudoArg {
    pub at_sign: Sp<()>,
    pub kind: Sp<PseudoArgKind>,
    pub eq_sign: Sp<()>,
    pub value: Sp<Expr>,
}

impl PseudoArg {
    /// Get the span that represents the "heading" of a pseudo arg. (everything but the value)
    pub fn tag_span(&self) -> Span {
        self.at_sign.span.merge(self.eq_sign.span)
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum PseudoArgKind {
        #[strum(serialize = "mask")] Mask,
        #[strum(serialize = "pop")] Pop,
        #[strum(serialize = "blob")] Blob,
        #[strum(serialize = "arg0")] ExtraArg,
    }
}

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum LabelPropertyKeyword {
        #[strum(serialize = "offsetof")] OffsetOf,
        #[strum(serialize = "timeof")] TimeOf,
    }
}

impl From<raw::LangInt> for Expr {
    fn from(value: raw::LangInt) -> Expr { Expr::LitInt { value, radix: IntRadix::Dec } }
}
impl From<raw::LangFloat> for Expr {
    fn from(value: raw::LangFloat) -> Expr { Expr::LitFloat { value } }
}
impl From<String> for Expr {
    fn from(string: String) -> Expr { Expr::LitString(LitString { string }) }
}

impl From<Sp<raw::LangInt>> for Sp<Expr> {
    fn from(num: Sp<raw::LangInt>) -> Sp<Expr> { sp!(num.span => Expr::from(num.value)) }
}
impl From<Sp<raw::LangFloat>> for Sp<Expr> {
    fn from(num: Sp<raw::LangFloat>) -> Sp<Expr> { sp!(num.span => Expr::from(num.value)) }
}
impl From<Sp<Var>> for Sp<Expr> {
    fn from(var: Sp<Var>) -> Sp<Expr> { sp!(var.span => Expr::Var(var)) }
}
impl From<Sp<String>> for Sp<Expr> {
    fn from(string: Sp<String>) -> Sp<Expr> { sp!(string.span => Expr::from(string.value)) }
}

// =============================================================================

string_enum! {
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum TypeKeyword {
        #[strum(serialize = "int")] Int,
        #[strum(serialize = "float")] Float,
        #[strum(serialize = "string")] String,
        #[strum(serialize = "var")] Var,
        #[strum(serialize = "void")] Void,
    }
}

impl TypeKeyword {
    /// Get the type, when used on a keyword that comes from a [`Var`].
    pub fn var_ty(self) -> value::VarType {
        match self {
            TypeKeyword::Int => value::ScalarType::Int.into(),
            TypeKeyword::Float => value::ScalarType::Float.into(),
            TypeKeyword::String => value::ScalarType::String.into(),
            TypeKeyword::Var => value::VarType::Untyped { explicit: false },
            TypeKeyword::Void => unreachable!("void var"),
        }
    }

    /// Get the type, when used on a keyword for a return type.
    pub fn expr_ty(self) -> value::ExprType {
        match self {
            TypeKeyword::Int => value::ScalarType::Int.into(),
            TypeKeyword::Float => value::ScalarType::Float.into(),
            TypeKeyword::String => value::ScalarType::String.into(),
            TypeKeyword::Void => value::ExprType::Void,
            TypeKeyword::Var => unreachable!("var return type"),
        }
    }
}

impl From<value::ScalarType> for TypeKeyword {
    fn from(ty: value::ScalarType) -> Self { match ty {
        value::ScalarType::Int => Self::Int,
        value::ScalarType::Float => Self::Float,
        value::ScalarType::String => Self::String,
    }}
}

impl From<value::VarType> for TypeKeyword {
    fn from(ty: value::VarType) -> Self { match ty {
        value::VarType::Typed(ty) => ty.into(),
        value::VarType::Untyped { .. } => Self::Var,
    }}
}

impl From<value::ExprType> for TypeKeyword {
    fn from(ty: value::ExprType) -> Self { match ty {
        value::ExprType::Value(ty) => ty.into(),
        value::ExprType::Void => Self::Void,
    }}
}

string_enum! {
    /// A type sigil on a variable.
    #[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[derive(enum_map::Enum)]
    pub enum VarSigil {
        #[strum(serialize = "$")] Int,
        #[strum(serialize = "%")] Float,
    }
}

impl VarSigil {
    pub fn from_ty(x: value::ScalarType) -> Option<VarSigil> {
        match x {
            value::ScalarType::Int => Some(VarSigil::Int),
            value::ScalarType::Float => Some(VarSigil::Float),
            value::ScalarType::String => None,
        }
    }
}

impl From<VarSigil> for value::ScalarType {
    fn from(x: VarSigil) -> value::ScalarType {
        match x {
            VarSigil::Int => value::ScalarType::Int,
            VarSigil::Float => value::ScalarType::Float,
        }
    }
}

/// A string literal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LitString {
    pub string: String,
}

impl From<String> for LitString {
    fn from(string: String) -> Self { LitString { string } }
}

impl From<&str> for LitString {
    fn from(str: &str) -> Self { LitString { string: str.to_owned() } }
}

// =============================================================================

impl std::fmt::Display for CallableName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CallableName::Normal { ident, language_if_ins: _ } => fmt::Display::fmt(ident, f),
            CallableName::Ins { opcode, language: _ } => write!(f, "ins_{}", opcode),
        }
    }
}

// =============================================================================

/// Trait for using [`Visit`] and [`VisitMut`] in a generic context.
///
/// The methods on this trait very simply just statically dispatch to the appropriate method
/// on those other traits.  E.g. if the node is an [`Item`] then [`Visitable::visit_with`] will
/// call the [`Visit::visit_item`] method, and etc.
///
/// The public API for most passes in [`crate::passes`] is a function with a bound on this trait,
/// because this is a lot nicer than directly exposing the visitors.  In particular, it saves the
/// caller from needing to import traits, or from having to worry about whether the visitor has a
/// method that they need to call in order to find out if an error occurred.
///
/// Note: For [`Block`], this specifically dispatches to [`Visit::visit_root_block`].
pub trait Visitable {
    /// Calls the method of [`Visit`] appropriate to this type, e.g. [`Visit::visit_expr`]
    /// if `Self` is an `Expr`.
    fn visit_with<V: Visit>(&self, f: &mut V);

    /// Calls the method of [`VisitMut`] appropriate to this type, e.g. [`VisitMut::visit_expr`]
    /// if `Self` is an `Expr`.
    fn visit_mut_with<V: VisitMut>(&mut self, f: &mut V);
}

macro_rules! generate_visitor_stuff {
    ($Visit:ident, Visitable::$visit:ident $(,$mut:tt)?) => {
        /// Recursive AST traversal trait.
        pub trait $Visit {
            fn visit_file(&mut self, e: & $($mut)? ScriptFile) { walk_file(self, e) }
            fn visit_item(&mut self, e: & $($mut)? Sp<Item>) { walk_item(self, e) }
            /// This is called only on the outermost blocks of each function.
            ///
            /// Overriding this is useful for things that need to know the full contents of a function
            /// (e.g. things that deal with labels). It also lets the visitor know if it is about to
            /// enter a nested function (if those ever become a thing).
            ///
            /// The implementation of [`Visitable`] for [`Block`] calls this instead of [`Self::visit_block`].
            /// The reasoning behind this is that any visitor which does override [`Self::visit_root_block`] is
            /// likely to only ever be used on blocks that do in fact represent a full function body.
            ///
            /// The default definition simply delegates to [`Self::visit_block`].
            fn visit_root_block(&mut self, e: & $($mut)? Block) { self.visit_block(e) }
            fn visit_block(&mut self, e: & $($mut)? Block) { walk_block(self, e) }
            fn visit_stmt(&mut self, e: & $($mut)? Sp<Stmt>) { walk_stmt(self, e) }
            fn visit_jump(&mut self, e: & $($mut)? StmtJumpKind) { walk_jump(self, e) }
            fn visit_expr(&mut self, e: & $($mut)? Sp<Expr>) { walk_expr(self, e) }
            /// Called on expressions that appear in conditions for e.g. `if`/`while`.
            ///
            /// The default definition simply delegates to [`Self::visit_expr`].
            fn visit_cond(&mut self, e: & $($mut)? Sp<Expr>) { self.visit_expr(e) }
            fn visit_var(&mut self, e: & $($mut)? Sp<Var>) { walk_var(self, e) }
            fn visit_callable_name(&mut self, e: & $($mut)? Sp<CallableName>) { walk_callable_name(self, e) }
            fn visit_meta(&mut self, e: & $($mut)? Sp<meta::Meta>) { walk_meta(self, e) }
            fn visit_res_ident(&mut self, _: & $($mut)? ResIdent) { }
            fn visit_node_id(&mut self, _: & $($mut)? Option<NodeId>) { }
            // this is factored out like this because otherwise the caller would have to repeat
            // the logic for matching over the different loop types.
            /// Called immediately before [`Self::visit_block`] on a loop or switch body.
            fn visit_loop_begin(&mut self, _: & $($mut)? Option<LoopId>) { }
            /// Called immediately after [`Self::visit_block`] on a loop or switch body.
            fn visit_loop_end(&mut self, _: & $($mut)? Option<LoopId>) { }
        }

        pub fn walk_file<V>(v: &mut V, x: & $($mut)? ScriptFile)
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
                Item::Func(ItemFunc {
                    code, qualifier: _, ty_keyword: _, ident, params,
                }) => {
                    v.visit_res_ident(ident);
                    if let Some(code) = code {
                        v.visit_root_block(code);
                    }

                    for sp_pat!(FuncParam { ident, ty_keyword: _, qualifier: _ }) in params {
                        if let Some(ident) = ident {
                            v.visit_res_ident(ident);
                        }
                    }
                },
                Item::AnmScript { keyword: _, number: _, ident: _, code } => {
                    v.visit_root_block(code);
                },
                Item::Timeline { keyword: _, number: _, ident: _, code } => {
                    v.visit_root_block(code);
                },
                Item::Meta { keyword: _, fields } => {
                    walk_meta_fields(v, fields);
                },
                Item::ConstVar { ty_keyword: _, vars } => {
                    for sp_pat![(var, expr)] in vars {
                        v.visit_var(var);
                        v.visit_expr(expr);
                    }
                },
            }
        }

        pub fn walk_meta<V>(v: &mut V, x: & $($mut)? Sp<meta::Meta>)
        where V: ?Sized + $Visit,
        {
            match & $($mut)? x.value {
                meta::Meta::Scalar(expr) => {
                    v.visit_expr(expr);
                },
                meta::Meta::Array(array) => {
                    for value in array {
                        v.visit_meta(value);
                    }
                },
                meta::Meta::Object(fields) => {
                    walk_meta_fields(v, fields);
                },
                meta::Meta::Variant { name: _, fields } => {
                    walk_meta_fields(v, fields);
                },
            }
        }

        fn walk_meta_fields<V>(v: &mut V, x: & $($mut)? Sp<meta::Fields>)
        where V: ?Sized + $Visit,
        {
            for (_key, value) in & $($mut)? x.value {
                v.visit_meta(value);
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
            let Stmt { node_id, kind, diff_label } = & $($mut)? x.value;

            v.visit_node_id(node_id);

            if let Some(diff_label) = diff_label {
                let DiffLabel { string, mask: _ } = & $($mut)? diff_label.value;
                let _: Sp<LitString> = *string;
            }

            match kind {
                StmtKind::Item(item) => v.visit_item(item),
                StmtKind::Jump(goto) => {
                    v.visit_jump(goto);
                },
                StmtKind::Return { value, keyword: _ } => {
                    if let Some(value) = value {
                        v.visit_expr(value);
                    }
                },
                StmtKind::Loop { block, keyword: _, loop_id } => {
                    v.visit_loop_begin(loop_id);
                    v.visit_block(block);
                    v.visit_loop_end(loop_id);
                },
                StmtKind::CondJump { cond, jump, keyword: _ } => {
                    v.visit_cond(cond);
                    v.visit_jump(jump);
                },
                StmtKind::CondChain(chain) => {
                    let StmtCondChain { cond_blocks, else_block } = chain;
                    for CondBlock { cond, block, keyword: _ } in cond_blocks {
                        v.visit_cond(cond);
                        v.visit_block(block);
                    }
                    if let Some(block) = else_block {
                        v.visit_block(block);
                    }
                },
                StmtKind::While { do_keyword: Some(_), while_keyword: _, loop_id, cond, block } => {
                    v.visit_cond(cond);
                    v.visit_loop_begin(loop_id);
                    v.visit_block(block);
                    v.visit_loop_end(loop_id);
                },
                StmtKind::While { do_keyword: None, while_keyword: _, loop_id, cond, block } => {
                    v.visit_loop_begin(loop_id);
                    v.visit_block(block);
                    v.visit_loop_end(loop_id);
                    v.visit_cond(cond);
                },
                StmtKind::Times { clobber, count, block, loop_id, keyword: _ } => {
                    if let Some(clobber) = clobber {
                        v.visit_var(clobber);
                    }
                    v.visit_expr(count);
                    v.visit_loop_begin(loop_id);
                    v.visit_block(block);
                    v.visit_loop_end(loop_id);
                },
                StmtKind::Expr(e) => {
                    v.visit_expr(e);
                },
                StmtKind::Block(block) => {
                    v.visit_block(block);
                },
                StmtKind::Assignment { var, op: _, value } => {
                    v.visit_var(var);
                    v.visit_expr(value);
                },
                StmtKind::Declaration { ty_keyword: _, vars } => {
                    for sp_pat![(var, value)] in vars {
                        v.visit_var(var);
                        if let Some(value) = value {
                            v.visit_expr(value);
                        }
                    }
                },
                StmtKind::CallSub { at_symbol: _, async_: _, func: _, args } => {
                    for arg in args {
                        v.visit_expr(arg);
                    }
                },
                StmtKind::Label(_) => {},
                StmtKind::InterruptLabel(_) => {},
                StmtKind::AbsTimeLabel { .. } => {},
                StmtKind::RelTimeLabel { .. } => {},
                StmtKind::ScopeEnd(_) => {},
                StmtKind::NoInstruction => {},
            }
        }

        pub fn walk_jump<V>(_: &mut V, e: & $($mut)? StmtJumpKind)
        where V: ?Sized + $Visit,
        {
            match e {
                StmtJumpKind::Goto(StmtGoto { destination, time }) => {
                    let _: Option<Sp<raw::LangInt>> = *time;
                    let _: Sp<Ident> = *destination;
                },
                StmtJumpKind::BreakContinue { keyword: _, loop_id: _ } => {},
            }
        }

        pub fn walk_expr<V>(v: &mut V, e: & $($mut)? Sp<Expr>)
        where V: ?Sized + $Visit,
        {
            match & $($mut)? e.value {
                Expr::Ternary { cond, left, right, question: _, colon: _ } => {
                    v.visit_expr(cond);
                    v.visit_expr(left);
                    v.visit_expr(right);
                },
                Expr::BinOp(a, _op, b) => {
                    v.visit_expr(a);
                    v.visit_expr(b);
                },
                Expr::DiffSwitch(cases) => {
                    for case in cases {
                        if let Some(case) = case {
                            v.visit_expr(case);
                        }
                    }
                },
                Expr::Call(ExprCall { name, args, pseudos }) => {
                    v.visit_callable_name(name);
                    for sp_pat![PseudoArg { value, kind: _, at_sign: _, eq_sign: _ }] in pseudos {
                        v.visit_expr(value);
                    }
                    for arg in args {
                        v.visit_expr(arg);
                    }
                },
                Expr::UnOp(_op, x) => v.visit_expr(x),
                Expr::XcrementOp { op: _, order: _, var } => {
                    v.visit_var(var);
                },
                Expr::LitInt { value: _, radix: _ } => {},
                Expr::LitFloat { value: _ } => {},
                Expr::LitString(_s) => {},
                Expr::LabelProperty { .. } => {},
                Expr::EnumConst { enum_name: _, ident } => {
                    v.visit_res_ident(ident);
                },
                Expr::Var(var) => v.visit_var(var),
            }
        }

        pub fn walk_callable_name<V>(v: &mut V, x: & $($mut)? Sp<CallableName>)
        where V: ?Sized + $Visit,
        {
            match & $($mut)? x.value {
                CallableName::Normal { language_if_ins: _, ident } => v.visit_res_ident(ident),
                CallableName::Ins { language: _, opcode: _ } => {},
            }
        }

        pub fn walk_var<V>(v: &mut V, x: & $($mut)? Sp<Var>)
        where V: ?Sized + $Visit,
        {
            let Var { name, ty_sigil: _ } = & $($mut)? x.value;
            match name {
                VarName::Normal { language_if_reg: _, ident } => v.visit_res_ident(ident),
                VarName::Reg { language: _, reg: _ } => {},
            }
        }
    };
}

macro_rules! impl_visitable {
    ($Node:ty, $visit_node:ident) => {
        impl Visitable for $Node {
            fn visit_with<V: Visit>(&self, v: &mut V) { <V as Visit>::$visit_node(v, self) }
            fn visit_mut_with<V: VisitMut>(&mut self, v: &mut V) { <V as VisitMut>::$visit_node(v, self) }
        }
    }
}

// * Sp<A> always has an impl so that `A: Parse, Sp<A>: Visitable` bounds work
// * If A's Visit method takes no span then `A` also implements Visitable,
//   as it will probably be needed.  (since this suggests that A can appear
//   in the AST with no span)
// * Some types might not implement this despite having a visit method,
//   if it seems unlikely that they'd ever be used as a top-level AST construct.
impl_visitable!(ScriptFile, visit_file);
impl_visitable!(Sp<ScriptFile>, visit_file);
impl_visitable!(Sp<Item>, visit_item);
impl_visitable!(Sp<Meta>, visit_meta);
impl_visitable!(Block, visit_root_block);
impl_visitable!(Sp<Block>, visit_root_block);
impl_visitable!(Sp<Stmt>, visit_stmt);
impl_visitable!(Sp<Expr>, visit_expr);

// used by AstVm to gather time labels
impl Visitable for [Sp<Stmt>] {
    fn visit_with<V: Visit>(&self, v: &mut V) {
        self.iter().for_each(|stmt| <V as Visit>::visit_stmt(v, stmt))
    }
    fn visit_mut_with<V: VisitMut>(&mut self, v: &mut V) {
        self.iter_mut().for_each(|stmt| <V as VisitMut>::visit_stmt(v, stmt))
    }
}

mod mut_ {
    use super::*;
    generate_visitor_stuff!(VisitMut, Visitable::visit_mut, mut);
}
pub use self::mut_::{
    VisitMut,
    walk_block as walk_block_mut,
    walk_callable_name as walk_callable_name_mut,
    walk_expr as walk_expr_mut,
    walk_file as walk_file_mut,
    walk_item as walk_item_mut,
    walk_jump as walk_jump_mut,
    walk_meta as walk_meta_mut,
    walk_stmt as walk_stmt_mut,
    walk_var as walk_var_mut,
};
mod ref_ {
    use super::*;
    generate_visitor_stuff!(Visit, Visitable::visit);
}
pub use self::ref_::{
    Visit, walk_block, walk_callable_name, walk_expr, walk_file, walk_item, walk_jump, walk_meta, walk_stmt,
    walk_var,
};
