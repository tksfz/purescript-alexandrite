use building_types::{QueryResult};
use checking::{CheckedModule, Evidence};
use corefn::{CoreFnModule, Declaration, Expr, Var, Literal, Binder, Binding, CaseAlternative, CaseResult};
use files::FileId;
use indexing::{IndexedModule};
use lowering::{LoweredModule, ExpressionKind, TermItemIr, BinderKind, LetBindingChunk};
use resolving::ResolvedModule;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

pub struct ElaborationContext<'a> {
    _file_id: FileId,
    lowered: &'a LoweredModule,
    checked: &'a CheckedModule,
    _indexed: &'a IndexedModule,
    binder_names: FxHashMap<lowering::BinderId, SmolStr>,
    let_names: FxHashMap<lowering::LetBindingNameGroupId, SmolStr>,
}

pub fn elaborate_module(
    file_id: FileId,
    lowered: &LoweredModule,
    checked: &CheckedModule,
    _resolved: &ResolvedModule,
    indexed: &IndexedModule,
) -> QueryResult<CoreFnModule> {
    let mut ctx = ElaborationContext {
        _file_id: file_id,
        lowered,
        checked,
        _indexed: indexed,
        binder_names: FxHashMap::default(),
        let_names: FxHashMap::default(),
    };

    for (id, kind) in lowered.info.iter_binder() {
        if let BinderKind::Variable { variable: Some(name) } = kind {
            ctx.binder_names.insert(id, name.clone());
        }
    }

    let mut declarations = Vec::new();

    for (id, _) in indexed.items.iter_terms() {
        let Some(term_item) = lowered.info.get_term_item(id) else { continue };
        if let Some(name) = &indexed.items[id].name {
            match term_item {
                TermItemIr::ValueGroup { equations, .. } => {
                    if let Some(expr) = elaborate_value_group(&mut ctx, equations) {
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
        imports: vec![],
        exports: vec![],
        declarations,
    })
}

fn elaborate_value_group(
    ctx: &mut ElaborationContext,
    equations: &[lowering::Equation],
) -> Option<Expr> {
    if let [equation] = equations {
        return elaborate_equation(ctx, equation);
    }
    None
}

fn elaborate_equation(
    ctx: &mut ElaborationContext,
    equation: &lowering::Equation,
) -> Option<Expr> {
    let mut body = if let Some(guarded) = &equation.guarded {
        elaborate_guarded(ctx, guarded)?
    } else {
        return None;
    };

    for binder_id in equation.binders.iter().rev() {
        let binder = elaborate_binder(ctx, *binder_id)?;
        body = Expr::Abs(binder, Box::new(body));
    }

    Some(body)
}

fn elaborate_guarded(
    ctx: &mut ElaborationContext,
    guarded: &lowering::GuardedExpression,
) -> Option<Expr> {
    match guarded {
        lowering::GuardedExpression::Unconditional { where_expression } => {
            if let Some(where_expression) = where_expression {
                let mut body = elaborate_expression(ctx, where_expression.expression?)?;
                if !where_expression.bindings.is_empty() {
                    let mut bindings = Vec::new();
                    for chunk in where_expression.bindings.iter() {
                        bindings.extend(elaborate_let_binding_chunk(ctx, chunk)?);
                    }
                    body = Expr::Let(bindings, Box::new(body));
                }
                Some(body)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn elaborate_let_binding_chunk(
    ctx: &mut ElaborationContext,
    chunk: &LetBindingChunk,
) -> Option<Vec<Binding>> {
    match chunk {
        LetBindingChunk::Names { bindings, .. } => {
            let mut result = Vec::new();
            for &group_id in bindings.iter() {
                let name_info = ctx.lowered.info.get_let_binding(group_id)?;
                let name = indexed_name_for_let(ctx, group_id);
                if let [equation] = &name_info.equations[..] {
                    let expr = elaborate_equation(ctx, equation)?;
                    result.push(Binding { name, expression: expr });
                }
            }
            Some(result)
        }
        _ => None,
    }
}

fn indexed_name_for_let(ctx: &mut ElaborationContext, id: lowering::LetBindingNameGroupId) -> SmolStr {
    if let Some(name) = ctx.let_names.get(&id) {
        return name.clone();
    }
    let name = SmolStr::from(format!("let_{}", id.into_raw().into_u32()));
    ctx.let_names.insert(id, name.clone());
    name
}

fn elaborate_expression(
    ctx: &mut ElaborationContext,
    id: lowering::ExpressionId,
) -> Option<Expr> {
    let kind = ctx.lowered.info.get_expression_kind(id)?;
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
                Some(lowering::TermVariableResolution::Binder(b_id)) => {
                    let name = ctx.binder_names.get(&b_id).cloned().unwrap_or_else(|| SmolStr::new("unknown_binder"));
                    Var::Local(name)
                }
                Some(lowering::TermVariableResolution::Let(l_id)) => {
                    let name = indexed_name_for_let(ctx, *l_id);
                    Var::Local(name)
                }
                Some(lowering::TermVariableResolution::RecordPun(_)) => {
                    Var::Local(SmolStr::new("pun_placeholder"))
                }
                None => return None,
            };
            Expr::Var(var)
        }
        ExpressionKind::Application { function, arguments } => {
            let mut current = elaborate_expression(ctx, (*function)?)?;
            for arg in arguments.iter() {
                match arg {
                    lowering::ExpressionArgument::Term(Some(term_id)) => {
                        let arg_expr = elaborate_expression(ctx, *term_id)?;
                        current = Expr::App(Box::new(current), Box::new(arg_expr));
                    }
                    _ => {}
                }
            }
            current
        }
        ExpressionKind::Lambda { binders, expression } => {
            let mut current = elaborate_expression(ctx, (*expression)?)?;
            for binder_id in binders.iter().rev() {
                let binder = elaborate_binder(ctx, *binder_id)?;
                current = Expr::Abs(binder, Box::new(current));
            }
            current
        }
        ExpressionKind::LetIn { bindings, expression } => {
            let mut body = elaborate_expression(ctx, (*expression)?)?;
            let mut all_bindings = Vec::new();
            for chunk in bindings.iter() {
                all_bindings.extend(elaborate_let_binding_chunk(ctx, chunk)?);
            }
            Expr::Let(all_bindings, Box::new(body))
        }
        ExpressionKind::Constructor { resolution } => {
            let (f, i) = (*resolution)?;
            Expr::Constructor(f, i)
        }
        ExpressionKind::Array { array } => {
            let mut exprs = Vec::new();
            for id_expr in array.iter() {
                exprs.push(elaborate_expression(ctx, *id_expr)?);
            }
            Expr::Literal(Literal::Array(exprs))
        }
        ExpressionKind::Record { record } => {
            let mut fields = FxHashMap::default();
            for item in record.iter() {
                match item {
                    lowering::ExpressionRecordItem::RecordField { name: Some(name), value: Some(value) } => {
                        fields.insert(name.clone(), elaborate_expression(ctx, *value)?);
                    }
                    _ => {}
                }
            }
            Expr::Literal(Literal::Object(fields))
        }
        ExpressionKind::IfThenElse { if_, then, else_ } => {
            let cond = elaborate_expression(ctx, (*if_)?)?;
            let then_expr = elaborate_expression(ctx, (*then)?)?;
            let else_expr = elaborate_expression(ctx, (*else_)?)?;
            
            Expr::Case(
                vec![cond],
                vec![
                    CaseAlternative {
                        binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(true))],
                        result: CaseResult::Expression(then_expr),
                    },
                    CaseAlternative {
                        binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                        result: CaseResult::Expression(else_expr),
                    },
                ]
            )
        }
        ExpressionKind::Parenthesized { parenthesized } => {
            elaborate_expression(ctx, (*parenthesized)?)?
        }
        ExpressionKind::CaseOf { trunk, branches } => {
            let trunk_exprs = trunk.iter().map(|e_id| elaborate_expression(ctx, *e_id)).collect::<Option<Vec<_>>>()?;
            
            let core_branches = branches.iter().map(|branch| {
                let binders = branch.binders.iter().map(|b_id| {
                    elaborate_binder(ctx, *b_id)
                }).collect::<Option<Vec<_>>>()?;
                
                let result = match &branch.guarded_expression {
                    Some(lowering::GuardedExpression::Unconditional { where_expression }) => {
                        let body = elaborate_expression(ctx, where_expression.as_ref()?.expression?)?;
                        CaseResult::Expression(body)
                    }
                    _ => return None,
                };
                
                Some(CaseAlternative {
                    binders,
                    result,
                })
            }).collect::<Option<Vec<_>>>()?;
            
            Expr::Case(trunk_exprs, core_branches)
        }
        _ => return None,
    };

    if let Some(evidence) = ctx.checked.nodes.evidence.get(&id) {
        expr = inject_evidence(ctx, expr, evidence);
    }

    Some(expr)
}

fn elaborate_binder(
    ctx: &mut ElaborationContext,
    id: lowering::BinderId,
) -> Option<Binder> {
    let kind = ctx.lowered.info.get_binder_kind(id)?;
    match kind {
        BinderKind::Variable { variable } => {
            let name = variable.clone().unwrap_or_else(|| SmolStr::new("_"));
            ctx.binder_names.insert(id, name.clone());
            Some(Binder::Var(name))
        }
        BinderKind::Wildcard => Some(Binder::Wildcard),
        _ => None,
    }
}

fn inject_evidence(
    ctx: &mut ElaborationContext,
    expr: Expr,
    evidence: &Evidence,
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
                let ev_expr = inject_evidence_inner(ctx, ev);
                current = Expr::App(Box::new(current), Box::new(ev_expr));
            }
            current
        }
        _ => expr,
    }
}

fn inject_evidence_inner(
    _ctx: &mut ElaborationContext,
    evidence: &Evidence,
) -> Expr {
    match evidence {
        Evidence::Given(type_id) => {
            let name = SmolStr::from(format!("dict_{}", type_id.id.get()));
            Expr::Var(Var::Local(name))
        }
        _ => Expr::Literal(Literal::String(SmolStr::new("compiler_magic"))),
    }
}
