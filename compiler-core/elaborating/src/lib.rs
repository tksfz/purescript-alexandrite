use std::sync::Arc;

use building_types::{QueryProxy, QueryResult};
use checking::CheckedModule;
use corefn::{CoreFnModule, Declaration, Expr, Var, Binder};
use files::FileId;
use lowering::LoweredModule;
use resolving::ResolvedModule;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

pub fn elaborate_module(
    file_id: FileId,
    lowered: &LoweredModule,
    checked: &CheckedModule,
    resolved: &ResolvedModule,
) -> QueryResult<CoreFnModule> {
    let mut declarations = Vec::new();

    // 1. Translate values
    for (id, term_item) in lowered.info.iter_term_item() {
        if let Some(name) = &term_item.name {
            if let Some(expr) = elaborate_expression(id, lowered, checked) {
                declarations.push(Declaration::Value {
                    name: name.clone(),
                    expression: expr,
                });
            }
        }
    }

    Ok(CoreFnModule {
        file_id,
        name: resolved.name.clone(),
        imports: vec![], // TODO
        exports: vec![], // TODO
        declarations,
    })
}

fn elaborate_expression(
    id: indexing::TermItemId,
    lowered: &LoweredModule,
    checked: &CheckedModule,
) -> Option<Expr> {
    // Initial skeleton for expression elaboration
    None
}
