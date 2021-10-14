//! Structs that carry important global compiler state.

use std::path::PathBuf;

use crate::ident::GensymContext;
use crate::resolve::Resolutions;
use crate::resolve::rib::Rib;
use crate::resolve::NodeId;

pub use defs::Defs;
pub mod defs;

pub use consts::Consts;
pub mod consts;

pub use crate::diagnostic::RootEmitter;

/// Context object for the majority of compilation.
///
/// This is a context object that holds a significant portion of the mutable state that is shared between
/// compiler passes (in particular passes that traverse the AST or that convert between the AST and
/// low-level representations).
///
/// It provides some methods for creating definitions and returning [`DefId`]s.  This is partly historical,
/// but also because it's not clear how to move these methods to [`Defs`] where they might conceptually belong.
///
/// # Limitation of scope
///
//   NOTE: To the future me who is about to delete this section from the documentation:
//         Please consider the examples of concessions in the section.
//         Are you absolutely certain you cannot do something similar to achieve your current goal?
//
/// While there is no doubt a great deal of code which depends on this type (or at least on one or more of
/// its fields), there are a number of phases of compilation that are **forbidden** to depend on this type
/// or any of its fields (except [`RootEmitter`]), just as a matter of principle.  These are:
///
/// * Parsing of text to AST
/// * Formatting of AST to text
/// * Reading binary files to the low-level representation
/// * Writing the low-level representation to a binary file
///
/// There have been numerous instances of things in the past which appeared that they may require breaking
/// this rule, but it has always been found possible to make concessions in favor of keeping this separation.
/// (e.g. [`llir::RawInstr`] holds an args blob so that reading/writing doesn't require signatures.
/// [`crate::passes::resolve_names::assign_res_ids`] allows the parser to not require `Resolutions`, and
/// [`crate::passes::debug::make_idents_unique`] does the same for the formatter)
#[derive(Debug)]
pub struct CompilerContext<'ctx> {
    pub emitter: &'ctx RootEmitter,

    /// Catalogues all loaded mapfiles for generating imports.
    mapfiles: Vec<PathBuf>,
    /// Results of name resolution.  Maps [`ResId`]s to [`DefId`]s.
    pub resolutions: Resolutions,
    /// Stores information about [`DefId`]s.
    pub defs: Defs,
    /// For generating identifiers.
    pub gensym: GensymContext,
    /// Cached const values.
    pub consts: Consts,
    /// The initial set of ribs for name resolution, containing names from mapfiles and meta.
    pub initial_ribs: Vec<Rib>,

    /// The location where any data behind a `&'ctx` reference is *actually* stored.
    _scope: &'ctx Scope,

    /// Next unused node ID for new AST nodes.
    next_node_id: NodeId,

    // The lifetime would *probably* eventually have to become invariant if we added arenas (as we
    // may eventually have AST nodes inside a struct inside a RefCell), so let's force this constraint now.
    _make_invariant: std::marker::PhantomData<*mut &'ctx ()>,
}

impl<'ctx> CompilerContext<'ctx> {
    pub fn new(scope: &'ctx Scope) -> Self {
        CompilerContext {
            emitter: &scope.emitter,
            mapfiles: Default::default(),
            resolutions: Default::default(),
            defs: Default::default(),
            gensym: Default::default(),
            consts: Default::default(),
            initial_ribs: Default::default(),
            _scope: scope,
            next_node_id: NodeId(std::num::NonZeroU32::new(1).unwrap()),
            _make_invariant: Default::default(),
        }
    }
}

/// The object that the `'ctx` lifetime on [`Truth`] primarily originates from.
/// May be used to store the following things:
///
/// * Arenas, as these must outlive [`Truth`].
/// * Things like [`RootEmitter`], where a piece of code may want to be able to use
///   both a `&RootEmitter` and a `&mut CompilerContext` at the same time.
///
/// [`Truth`]: [`crate::api::Truth`]
#[derive(Debug)]
pub struct Scope {
    emitter: RootEmitter,
}

impl Scope {
    pub fn new(emitter: RootEmitter) -> Self {
        Scope { emitter }
    }
}
