use building_types::{QueryResult};
use checking::{CheckedModule, Evidence};
use corefn::{CoreFnModule, Declaration, Expr, Var, Literal};
use files::FileId;
use indexing::{IndexedModule};
use lowering::{LoweredModule, ExpressionKind, TermItemIr};
use resolving::ResolvedModule;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

pub fn elaborate_module(
    file_id: FileId,
    lowered: &LoweredModule,
    checked: &CheckedModule,
    _resolved: &ResolvedModule,
    indexed: &IndexedModule,
) -> QueryResult<CoreFnModule> {
    let mut declarations = Vec::new();

    // Iterate over term items from lowering info
    for (id, _) in indexed.items.iter_terms() {
        let Some(term_item) = lowered.info.get_term_item(id) else { continue };
        if let Some(name) = &indexed.items[id].name {
            match term_item {
                TermItemIr::ValueGroup { equations, .. } => {
                    if let Some(expr) = elaborate_value_group(equations, lowered, checked) {
                        declarations.push(Declaration::Value {
                            name: name.clone(),
                            expression: expr,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    Ok(CoreFnModule {
        file_id,
        name: indexed.name.clone().unwrap_or_else(|| SmolStr::new("Main")),
        imports: vec![], // TODO
        exports: vec![], // TODO
        declarations,
    })
}

fn elaborate_value_group(
    equations: &[lowering::Equation],
    lowered: &LoweredModule,
    checked: &CheckedModule,
) -> Option<Expr> {
    if let [equation] = equations {
        if equation.binders.is_empty() {
            if let Some(guarded) = &equation.guarded {
                 return elaborate_guarded(guarded, lowered, checked);
            }
        }
    }
    None
}

fn elaborate_guarded(
    guarded: &lowering::GuardedExpression,
    lowered: &LoweredModule,
    checked: &CheckedModule,
) -> Option<Expr> {
    match guarded {
        lowering::GuardedExpression::Unconditional { where_expression } => {
            if let Some(where_expression) = where_expression {
                if let Some(expr_id) = where_expression.expression {
                    elaborate_expression(expr_id, lowered, checked)
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    }
}

fn elaborate_expression(
    id: lowering::ExpressionId,
    lowered: &LoweredModule,
    checked: &CheckedModule,
) -> Option<Expr> {
    let kind = lowered.info.get_expression_kind(id)?;
    let mut expr = match kind {
        ExpressionKind::Integer { value } => {
             Expr::Literal(Literal::Int(value.unwrap_or(0)))
        }
        ExpressionKind::Number { value, .. } => {
             Expr::Literal(Literal::Number(value.clone().unwrap_or_default()))
        }
        ExpressionKind::String { value, .. } => {
             Expr::Literal(Literal::String(value.clone().unwrap_or_default()))
        }
        ExpressionKind::Char { value } => {
             Expr::Literal(Literal::Char(value.unwrap_or('\0')))
        }
        ExpressionKind::Boolean { boolean } => Expr::Literal(Literal::Boolean(*boolean)),
        ExpressionKind::Variable { resolution } => {
            let var = match resolution {
                Some(lowering::TermVariableResolution::Reference(f, i)) => Var::Module(*f, *i),
                _ => Var::Local(SmolStr::new("local_placeholder")),
            };
            Expr::Var(var)
        }
        ExpressionKind::Application { function, arguments } => {
            let mut current = elaborate_expression((*function)?, lowered, checked)?;
            for arg in arguments.iter() {
                match arg {
                    lowering::ExpressionArgument::Term(Some(term_id)) => {
                        let arg_expr = elaborate_expression(*term_id, lowered, checked)?;
                        current = Expr::App(Box::new(current), Box::new(arg_expr));
                    }
                    _ => {}
                }
            }
            current
        }
        ExpressionKind::Array { array } => {
            let mut exprs = Vec::new();
            for id in array.iter() {
                exprs.push(elaborate_expression(*id, lowered, checked)?);
            }
            Expr::Literal(Literal::Array(exprs))
        }
        ExpressionKind::Record { record } => {
            let mut fields = FxHashMap::default();
            for item in record.iter() {
                match item {
                    lowering::ExpressionRecordItem::RecordField { name: Some(name), value: Some(value) } => {
                        fields.insert(name.clone(), elaborate_expression(*value, lowered, checked)?);
                    }
                    _ => {}
                }
            }
            Expr::Literal(Literal::Object(fields))
        }
        _ => return None,
    };

    if let Some(evidence) = checked.nodes.evidence.get(&id) {
        expr = inject_evidence(expr, evidence, lowered, checked);
    }

    Some(expr)
}

fn inject_evidence(
    expr: Expr,
    evidence: &Evidence,
    lowered: &LoweredModule,
    checked: &CheckedModule,
) -> Expr {
    match evidence {
        Evidence::Instance(_id, _sub_evidences) => {
            Expr::Literal(Literal::String(SmolStr::from("instance_placeholder")))
        }
        Evidence::Given(type_id) => {
            let name = SmolStr::from(format!("dict_{}", type_id.id.get()));
            Expr::App(Box::new(expr), Box::new(Expr::Var(Var::Local(name))))
        }
        Evidence::Multiple(evidences) => {
            let mut current = expr;
            for ev in evidences {
                let ev_expr = inject_evidence_inner(ev, lowered, checked);
                current = Expr::App(Box::new(current), Box::new(ev_expr));
            }
            current
        }
        _ => expr,
    }
}

fn inject_evidence_inner(
    evidence: &Evidence,
    _lowered: &LoweredModule,
    _checked: &CheckedModule,
) -> Expr {
    match evidence {
        Evidence::Given(type_id) => {
            let name = SmolStr::from(format!("dict_{}", type_id.id.get()));
            Expr::Var(Var::Local(name))
        }
        _ => Expr::Literal(Literal::String(SmolStr::new("compiler_magic"))),
    }
}
