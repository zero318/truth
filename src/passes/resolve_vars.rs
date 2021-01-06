use crate::ast::{self, Visit, VisitMut};
use crate::error::CompileError;
use crate::pos::Sp;
use crate::scope::{ScopeId, VarId, Variables, NameResolver, EMPTY_SCOPE};
use crate::type_system::{TypeSystem, ScalarType};

pub struct Visitor<'ts> {
    resolver: NameResolver,
    scope_stack: Vec<ScopeId>,
    ty_ctx: &'ts mut TypeSystem,
    variables: Variables,
    errors: CompileError,
}

impl<'ts> Visitor<'ts> {
    pub fn new(ty_ctx: &'ts mut TypeSystem) -> Self {
        let (variables, root_scope) = initial_variables(ty_ctx);
        let mut resolver = NameResolver::new();
        resolver.enter_descendant(root_scope, &variables);
        Visitor {
            resolver, variables, ty_ctx,
            scope_stack: vec![],
            errors: CompileError::new_empty(),
        }
    }

    pub fn finish(self) -> Result<(), CompileError> {
        self.errors.into_result(())
    }
}

impl VisitMut for Visitor<'_> {
    fn visit_block(&mut self, x: &mut ast::Block) {
        self.scope_stack.push(self.resolver.current_scope());

        ast::walk_mut_block(self, x);

        let original = self.scope_stack.pop().expect("(BUG!) unbalanced scope_stack usage!");
        self.resolver.return_to_ancestor(original, &self.variables);
    }

    fn visit_stmt_body(&mut self, x: &mut Sp<ast::StmtBody>) {
        match &mut x.value {
            ast::StmtBody::Declaration { keyword, vars } => {
                let ty = match keyword {
                    ast::VarDeclKeyword::Int => Some(ScalarType::Int),
                    ast::VarDeclKeyword::Float => Some(ScalarType::Float),
                    ast::VarDeclKeyword::Var => None,
                };

                for (var, init_value) in vars {
                    if let ast::Var::Named { ty_sigil, ident } = &var.value {
                        assert!(ty_sigil.is_none());

                        // a variable should not be allowed to appear in its own initializer, so walk the expression first.
                        if let Some(init_value) = init_value {
                            self.visit_expr(init_value);
                        }

                        // now declare the variable and enter its scope so that it can be used in future expressions
                        let var_id = self.variables.declare(self.resolver.current_scope(), ident.clone(), ty);
                        self.resolver.enter_descendant(self.variables.get_scope(var_id), &self.variables);

                        // swap out the AST node
                        var.value = ast::Var::Local { ty_sigil: None, var_id };
                    }
                }
            }
            _ => ast::walk_mut_stmt_body(self, x),
        }

        let original = self.scope_stack.pop().expect("(BUG!) unbalanced scope_stack usage!");
        self.resolver.return_to_ancestor(original, &self.variables);
    }

    fn visit_var(&mut self, var: &mut Sp<ast::Var>) {
        if let ast::Var::Named { ty_sigil, ref ident } = var.value {
            match self.resolver.resolve(ident) {
                Some(var_id) => {
                    var.value = ast::Var::Local { ty_sigil, var_id };
                },
                None => self.errors.append(error!(
                    message("no such variable {}", ident),
                    primary(var, "not found in this scope"),
                )),
            };
        }
    }
}

/// Given a [`TypeSystem`] that only contains register aliases from mapfiles, create a [`Variables`]
/// with these names and get the scope containing all of the variables.
fn initial_variables(initial_ty_ctx: &TypeSystem) -> (Variables, ScopeId) {
    let mut variables = Variables::new();
    let mut scope = EMPTY_SCOPE;
    for (alias, &raw_id) in &initial_ty_ctx.reg_map {
        let ty = initial_ty_ctx.reg_default_types.get(&raw_id).copied();
        let new_var_id = variables.declare(scope, alias.clone(), ty);
        scope = variables.get_scope(new_var_id);
    }
    (variables, scope)
}

// --------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmt::Formatter;

    const SIMPLE_MAPFILE: &'static str = "\
        !anmmap\n\
        !gvar_names\n0 A\n1 B\n2 C\n3 D\n4 X\n5 Y\n6 Z\n7 W\n\
        !gvar_types\n0 $\n1 $\n2 $\n3 $\n4 %\n5 %\n6 %\n7 %\n";

    fn compile_exprs(text: &str) -> String {
        let general_use_gvars = enum_map!{
            ScalarType::Int => vec![0, 1, 2, 3],
            ScalarType::Float => vec![4, 5, 6, 7],
        };

        let eclmap = crate::eclmap::Eclmap::parse(SIMPLE_MAPFILE).unwrap();
        let mut ty_ctx = crate::type_system::TypeSystem::new();
        ty_ctx.extend_from_eclmap("DUMMY.anmmap".as_ref(), &eclmap);

        let mut f = Formatter::new(vec![]).with_max_columns(99999);
        let mut files = crate::pos::Files::new();
        let mut script = files.parse::<ast::Script>("<input>", text.as_bytes()).unwrap_or_else(|e| panic!("{}", e));

        let mut visitor = Visitor::new(general_use_gvars, &ty_ctx);
        ast::walk_mut_script(&mut visitor, &mut script);
        visitor.finish().unwrap();

        f.fmt(&script).unwrap();
        String::from_utf8(f.into_inner().unwrap()).unwrap()
    }

    #[test]
    fn lol() {
        assert_snapshot!("halp", compile_exprs(r#"void main() { A = (B + 2) * (B + 3) * (B + 4); }"#).trim());
    }

    #[test]
    fn lol2() {
        assert_snapshot!("bleh", compile_exprs(r#"void main() { A = 3 * (B + 2); }"#).trim());
    }

    #[test]
    fn lol3() {
        assert_snapshot!("blue", compile_exprs(r#"void main() { A = (B + 2) * 3; }"#).trim());
    }
}
