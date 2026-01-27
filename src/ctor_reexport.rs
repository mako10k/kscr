use crate::ast;

pub fn resolve_qualified_ctor_via_type_alias(
    module: &ast::Module,
    qctor: &ast::ResolvedName,
) -> Option<ast::ResolvedName> {
    let binding = qctor.qualified_text();
    let (_qual, ctor) = match binding.rsplit_once('.') {
        Some(x) => x,
        None => return None,
    };
    // Find `type T ... = OtherQual.T ...` aliases and allow re-exporting ctors via `export T(..)`.
    // Minimal: if the module exports `T(..)` and has a type alias `T = Prelude.T`, then
    // map `qual.C` to `Prelude.C`.
    //
    // Note: we only need Prelude.Maybe for now.
    let mut alias_to_qual: std::collections::HashMap<String, String> = Default::default();
    for it in &module.items {
        if let ast::Item::TypeAlias(ta) = it {
            // Alias head must be a qualified type var: `Prelude.Maybe`
            if let ast::Type::Var(rhs) = &ta.ty {
                if let Some((rhs_qual, rhs_name)) = rhs.rsplit_once('.') {
                    alias_to_qual.insert(ta.name.clone(), rhs_qual.to_string());
                    // Only if alias points to same name.
                    if rhs_name != ta.name {
                        // still record qual, but mismatched names shouldn't be used for ctor re-export.
                    }
                }
            }
        }
    }

    // Heuristic: if ctor is Just/Nothing, and module has `type Maybe = Prelude.Maybe`, rewrite.
    if matches!(ctor, "Just" | "Nothing") {
        if let Some(target_qual) = alias_to_qual.get("Maybe") {
            // Preserve module id (identity), but adjust printed qualifier to target module.
            return Some(match qctor {
                ast::ResolvedName::Unresolved(_) => ast::ResolvedName::unresolved(format!(
                    "{target_qual}.{ctor}"
                )),
                ast::ResolvedName::Resolved { module, .. } => ast::ResolvedName::Resolved {
                    module: *module,
                    module_name: target_qual.clone(),
                    name: ctor.to_string(),
                },
            });
        }
    }
    None
}
