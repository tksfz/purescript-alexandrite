use building_types::{QueryProxy, QueryResult};
use checking::{CheckedModule, Evidence};
use corefn::{CoreFnModule, Declaration, Expr, Var, Literal, Binder, Binding, CaseAlternative, CaseResult};
use files::FileId;
use indexing::{IndexedModule};
use lowering::{LoweredModule, ExpressionKind, TermItemIr, BinderKind, LetBindingChunk};
use resolving::ResolvedModule;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

pub struct ElaborationContext<'a, Q> {
    _file_id: FileId,
    queries: &'a Q,
    lowered: &'a LoweredModule,
    checked: &'a CheckedModule,
    _indexed: &'a IndexedModule,
    binder_names: FxHashMap<lowering::BinderId, SmolStr>,
    let_names: FxHashMap<lowering::LetBindingNameGroupId, SmolStr>,
    pun_names: FxHashMap<lowering::RecordPunId, SmolStr>,
}

pub fn elaborate_module<Q: QueryProxy>(
    queries: &Q,
    file_id: FileId,
    lowered: &LoweredModule,
    checked: &CheckedModule,
    _resolved: &ResolvedModule,
    indexed: &IndexedModule,
) -> QueryResult<CoreFnModule> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    let mut ctx = ElaborationContext {
        _file_id: file_id,
        queries,
        lowered,
        checked,
        _indexed: indexed,
        binder_names: FxHashMap::default(),
        let_names: FxHashMap::default(),
        pun_names: FxHashMap::default(),
    };

    for (id, kind) in lowered.info.iter_binder() {
        if let BinderKind::Variable { variable: Some(name) } = kind {
            ctx.binder_names.insert(id, name.clone());
        }
    }

    let mut declarations = Vec::new();

    for (id, term_item) in lowered.info.iter_term_item() {
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
                TermItemIr::Instance { members, .. } => {
                    let mut fields = FxHashMap::default();
                    for member in members.iter() {
                        if let Some((_f, t)) = member.resolution {
                            if let Some(member_name) = &indexed.items[t].name {
                                if let Some(expr) =
                                    elaborate_value_group(&mut ctx, &member.equations)
                                {
                                    fields.insert(member_name.clone(), expr);
                                }
                            }
                        }
                    }
                    declarations.push(Declaration::Value {
                        name: name.clone(),
                        expression: Expr::Literal(Literal::Object(fields)),
                    });
                }
                TermItemIr::ClassMember { .. } => {
                    let dict_name = SmolStr::new("dict");
                    let expr = Expr::Abs(
                        Binder::Var(dict_name.clone()),
                        Box::new(Expr::Accessor(name.clone(), Box::new(Expr::Var(Var::Local(dict_name))))),
                    );
                    declarations.push(Declaration::Value {
                        name: name.clone(),
                        expression: expr,
                    });
                }
                _ => {}
            }
        }
    }

    for (id, type_item) in lowered.info.iter_type_item() {
        if let Some(name) = &indexed.items[id].name {
            match type_item {
                lowering::TypeItemIr::DataGroup { .. } => {
                    let mut core_constructors = Vec::new();
                    for ctor_id in indexed.pairs.data_constructors(id) {
                        if let Some(ctor_name) = &indexed.items[ctor_id].name {
                            if let Some(TermItemIr::Constructor { arguments }) =
                                lowered.info.get_term_item(ctor_id)
                            {
                                let fields = (0..arguments.len())
                                    .map(|i| SmolStr::from(format!("value{}", i)))
                                    .collect();
                                core_constructors.push(corefn::Constructor {
                                    name: ctor_name.clone(),
                                    fields,
                                });
                            }
                        }
                    }
                    declarations.push(Declaration::Data {
                        name: name.clone(),
                        constructors: core_constructors,
                    });
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

fn elaborate_value_group<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    equations: &[lowering::Equation],
) -> Option<Expr> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    if equations.is_empty() {
        return None;
    }

    if equations.len() == 1 && equations[0].binders.is_empty() {
        return elaborate_equation(ctx, &equations[0]);
    }

    let num_args = equations[0].binders.len();
    let mut arg_names = Vec::new();
    for i in 0..num_args {
        arg_names.push(SmolStr::from(format!("arg{}", i)));
    }

    let mut alternatives = Vec::new();
    for equation in equations {
        let binders = equation.binders.iter().map(|&b_id| {
            elaborate_binder(ctx, b_id)
        }).collect::<Option<Vec<_>>>()?;
        
        let result = match &equation.guarded {
            Some(lowering::GuardedExpression::Unconditional { where_expression }) => {
                CaseResult::Expression(elaborate_expression(ctx, where_expression.as_ref()?.expression?)?)
            }
            Some(lowering::GuardedExpression::Conditionals { pattern_guarded }) => {
                let mut final_expr = None;
                for conditional in pattern_guarded.iter().rev() {
                    if let [guard] = &conditional.pattern_guards[..] {
                        let cond = elaborate_expression(ctx, guard.expression?)?;
                        let result = elaborate_where(ctx, &conditional.where_expression)?;
                        
                        let alt_true = CaseAlternative {
                            binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(true))],
                            result: CaseResult::Expression(result),
                        };
                        
                        let next_alt = if let Some(prev) = final_expr {
                            CaseAlternative {
                                binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                                result: CaseResult::Expression(prev),
                            }
                        } else {
                            CaseAlternative {
                                binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                                result: CaseResult::Expression(Expr::Literal(Literal::String(SmolStr::new("guard_fail")))),
                            }
                        };
                        
                        final_expr = Some(Expr::Case(vec![cond], vec![alt_true, next_alt]));
                    }
                }
                CaseResult::Expression(final_expr?)
            }
            _ => return None,
        };
        
        alternatives.push(CaseAlternative {
            binders,
            result,
        });
    }

    let trunk = arg_names.iter().map(|name| Expr::Var(Var::Local(name.clone()))).collect();
    let mut body = Expr::Case(trunk, alternatives);

    for name in arg_names.iter().rev() {
        body = Expr::Abs(Binder::Var(name.clone()), Box::new(body));
    }

    Some(body)
}

fn elaborate_equation<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    equation: &lowering::Equation,
) -> Option<Expr> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
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

fn elaborate_guarded<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    guarded: &lowering::GuardedExpression,
) -> Option<Expr> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
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
        lowering::GuardedExpression::Conditionals { pattern_guarded } => {
            let mut final_expr = None;
            for conditional in pattern_guarded.iter().rev() {
                if let [guard] = &conditional.pattern_guards[..] {
                    let cond = elaborate_expression(ctx, guard.expression?)?;
                    let result = elaborate_where(ctx, &conditional.where_expression)?;
                    
                    let alt_true = CaseAlternative {
                        binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(true))],
                        result: CaseResult::Expression(result),
                    };
                    
                    let next_alt = if let Some(prev) = final_expr {
                        CaseAlternative {
                            binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                            result: CaseResult::Expression(prev),
                        }
                    } else {
                        CaseAlternative {
                            binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                            result: CaseResult::Expression(Expr::Literal(Literal::String(SmolStr::new("guard_fail")))),
                        }
                    };
                    
                    final_expr = Some(Expr::Case(vec![cond], vec![alt_true, next_alt]));
                }
            }
            final_expr
        }
    }
}

fn elaborate_where<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    where_expression: &Option<lowering::WhereExpression>,
) -> Option<Expr>
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    let where_expression = where_expression.as_ref()?;
    let mut body = elaborate_expression(ctx, where_expression.expression?)?;
    if !where_expression.bindings.is_empty() {
        let mut bindings = Vec::new();
        for chunk in where_expression.bindings.iter() {
            bindings.extend(elaborate_let_binding_chunk(ctx, chunk)?);
        }
        body = Expr::Let(bindings, Box::new(body));
    }
    Some(body)
}

fn elaborate_let_binding_chunk<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    chunk: &LetBindingChunk,
) -> Option<Vec<Binding>> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
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

fn indexed_name_for_let<Q: QueryProxy>(ctx: &mut ElaborationContext<Q>, id: lowering::LetBindingNameGroupId) -> SmolStr {
    if let Some(name) = ctx.let_names.get(&id) {
        return name.clone();
    }
    let name = SmolStr::from(format!("let_{}", id.into_raw().into_u32()));
    ctx.let_names.insert(id, name.clone());
    name
}

fn elaborate_expression<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    id: lowering::ExpressionId,
) -> Option<Expr> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    let kind = ctx.lowered.info.get_expression_kind(id)?;
    let mut expr = match kind {
        ExpressionKind::Integer { value } => {
             Expr::Literal(Literal::Int(value.unwrap_or(0)))
        }
        ExpressionKind::Boolean { boolean } => Expr::Literal(Literal::Boolean(*boolean)),
        ExpressionKind::String { value, .. } => {
            Expr::Literal(Literal::String(value.clone().unwrap_or_default()))
        }
        ExpressionKind::Char { value } => {
            Expr::Literal(Literal::Char(value.unwrap_or('\0')))
        }
        ExpressionKind::Number { value, .. } => {
            Expr::Literal(Literal::Number(value.clone().unwrap_or_default()))
        }
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
                Some(lowering::TermVariableResolution::RecordPun(pun_id)) => {
                    let name = ctx.pun_names.get(&pun_id).cloned().unwrap_or_else(|| SmolStr::new("pun_placeholder"));
                    Var::Local(name)
                }
                None => {
                    return None;
                }
            };
            Expr::Var(var)
        }
        ExpressionKind::InfixChain { head, tail } => {
            let mut current = elaborate_expression(ctx, (*head).or_else(|| {
                None
            })?)?;
            for pair in tail.iter() {
                let op = elaborate_expression(ctx, pair.tick.or_else(|| {
                    None
                })?)?;
                let arg = elaborate_expression(ctx, pair.element.or_else(|| {
                    None
                })?)?;
                current = Expr::App(Box::new(Expr::App(Box::new(op), Box::new(current))), Box::new(arg));
            }
            current
        }
        ExpressionKind::OperatorChain { head, tail } => {
            let mut current = elaborate_expression(ctx, (*head).or_else(|| {
                None
            })?)?;
            for pair in tail.iter() {
                let op_id = pair.id.or_else(|| {
                    None
                })?;
                let op_res = ctx.lowered.info.get_term_operator(op_id).or_else(|| {
                     None
                })?;
                let op = Expr::Var(Var::Module(op_res.0, op_res.1));
                let arg = elaborate_expression(ctx, pair.element.or_else(|| {
                    None
                })?)?;
                current = Expr::App(Box::new(Expr::App(Box::new(op), Box::new(current))), Box::new(arg));
            }
            current
        }
        ExpressionKind::Application { function, arguments } => {
            let mut current = elaborate_expression(ctx, (*function).or_else(|| {
                None
            })?)?;
            for arg in arguments.iter() {
                match arg {
                    lowering::ExpressionArgument::Term(Some(term_id)) => {
                        if let Some(arg_expr) = elaborate_expression(ctx, *term_id) {
                            current = Expr::App(Box::new(current), Box::new(arg_expr));
                        }
                    }
                    _ => {}
                }
            }
            current
        }
        ExpressionKind::Lambda { binders, expression } => {
            let mut current = elaborate_expression(ctx, (*expression).or_else(|| {
                None
            })?)?;
            for (i, binder_id) in binders.iter().rev().enumerate() {
                let binder = elaborate_binder(ctx, *binder_id).or_else(|| {
                    None
                })?;
                current = Expr::Abs(binder, Box::new(current));
            }
            current
        }
        ExpressionKind::LetIn { bindings, expression } => {
            let mut body = elaborate_expression(ctx, (*expression).or_else(|| {
                None
            })?)?;
            let mut all_bindings = Vec::new();
            for (i, chunk) in bindings.iter().enumerate() {
                all_bindings.extend(elaborate_let_binding_chunk(ctx, chunk).or_else(|| {
                    None
                })?);
            }
            Expr::Let(all_bindings, Box::new(body))
        }
        ExpressionKind::OperatorName { resolution } => {
            let (f, i) = (*resolution).or_else(|| {
                None
            })?;
            Expr::Var(Var::Module(f, i))
        }
        ExpressionKind::Constructor { resolution } => {
            let (f, i) = (*resolution).or_else(|| {
                None
            })?;
            Expr::Constructor(f, i)
        }
        ExpressionKind::Array { array } => {
            let mut exprs = Vec::new();
            for (i, id_expr) in array.iter().enumerate() {
                exprs.push(elaborate_expression(ctx, *id_expr).or_else(|| {
                    None
                })?);
            }
            Expr::Literal(Literal::Array(exprs))
        }
        ExpressionKind::Record { record } => {
            let mut fields = FxHashMap::default();
            for item in record.iter() {
                match item {
                    lowering::ExpressionRecordItem::RecordField { name: Some(name), value: Some(value) } => {
                        fields.insert(name.clone(), elaborate_expression(ctx, *value).or_else(|| {
                            None
                        })?);
                    }
                    _ => {
                    }
                }
            }
            Expr::Literal(Literal::Object(fields))
        }
        ExpressionKind::RecordUpdate { record, updates } => {
            let record_expr = elaborate_expression(ctx, (*record).or_else(|| {
                None
            })?)?;
            let core_updates = updates.iter().map(|u| elaborate_record_update(ctx, u)).collect::<Option<Vec<_>>>()?;
            Expr::RecordUpdate(Box::new(record_expr), core_updates)
        }
        ExpressionKind::RecordAccess { record, labels } => {
            let mut current = elaborate_expression(ctx, (*record).or_else(|| {
                None
            })?)?;
            if let Some(labels) = labels {
                for label in labels.iter() {
                    current = Expr::Accessor(label.clone(), Box::new(current));
                }
            }
            current
        }
        ExpressionKind::IfThenElse { if_, then, else_ } => {
            let cond = elaborate_expression(ctx, (*if_).or_else(|| {
                 None
            })?)?;
            let then_expr = elaborate_expression(ctx, (*then).or_else(|| {
                 None
            })?)?;
            let else_expr = elaborate_expression(ctx, (*else_).or_else(|| {
                 None
            })?)?;
            
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
            elaborate_expression(ctx, (*parenthesized).or_else(|| {
                None
            })?)?
        }
        ExpressionKind::CaseOf { trunk, branches } => {
            let trunk_exprs = trunk.iter().map(|e_id| elaborate_expression(ctx, *e_id).or_else(|| {
                None
            })).collect::<Option<Vec<_>>>()?;
            
            let core_branches = branches.iter().map(|branch| {
                let binders = branch.binders.iter().map(|b_id| {
                    elaborate_binder(ctx, *b_id).or_else(|| {
                        None
                    })
                }).collect::<Option<Vec<_>>>()?;
                
                let result = match &branch.guarded_expression {
                    Some(lowering::GuardedExpression::Unconditional { where_expression }) => {
                        let body = elaborate_expression(ctx, where_expression.as_ref()?.expression?).or_else(|| {
                            None
                        })?;
                        CaseResult::Expression(body)
                    }
                    Some(lowering::GuardedExpression::Conditionals { pattern_guarded }) => {
                         let mut final_expr = None;
                         for conditional in pattern_guarded.iter().rev() {
                             if let [guard] = &conditional.pattern_guards[..] {
                                 let cond = elaborate_expression(ctx, guard.expression?)?;
                                 let result = elaborate_where(ctx, &conditional.where_expression)?;
                                 
                                 let alt_true = CaseAlternative {
                                     binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(true))],
                                     result: CaseResult::Expression(result),
                                 };
                                 
                                 let next_alt = if let Some(prev) = final_expr {
                                     CaseAlternative {
                                         binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                                         result: CaseResult::Expression(prev),
                                     }
                                 } else {
                                     CaseAlternative {
                                         binders: vec![Binder::Literal(corefn::LiteralBinder::Boolean(false))],
                                         result: CaseResult::Expression(Expr::Literal(Literal::String(SmolStr::new("guard_fail")))),
                                     }
                                 };
                                 
                                 final_expr = Some(Expr::Case(vec![cond], vec![alt_true, next_alt]));
                             }
                         }
                         CaseResult::Expression(final_expr?)
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
        ExpressionKind::Do { bind, discard, statements } => {
            elaborate_do(ctx, bind, discard, statements)?
        }
        ExpressionKind::Ado { map, apply, pure, statements, expression } => {
            elaborate_ado(ctx, map, apply, pure, statements, expression)?
        }
        _ => {
            return None;
        }
    };

    if let Some(evidence) = ctx.checked.nodes.evidence.get(&id) {
        expr = inject_evidence(ctx, expr, evidence);
    }

    Some(expr)
}

fn elaborate_do<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    bind: &Option<lowering::TermVariableResolution>,
    discard: &Option<lowering::TermVariableResolution>,
    statements: &[lowering::DoStatementId],
) -> Option<Expr>
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    elaborate_do_statements(ctx, bind, discard, statements)
}

fn elaborate_do_statements<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    bind: &Option<lowering::TermVariableResolution>,
    discard: &Option<lowering::TermVariableResolution>,
    statements: &[lowering::DoStatementId],
) -> Option<Expr>
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    let (first, rest) = statements.split_first()?;
    let statement = ctx.lowered.info.get_do_statement(*first)?;

    if rest.is_empty() {
        return match statement {
            lowering::DoStatement::Discard { expression } => {
                elaborate_expression(ctx, (*expression)?)
            }
            _ => {
                None
            }
        };
    }

    match statement {
        lowering::DoStatement::Bind { binder, expression } => {
            let bind_res = bind.as_ref().or_else(|| {
                None
            })?;
            let bind_expr = Expr::Var(match bind_res {
                lowering::TermVariableResolution::Reference(f, i) => Var::Module(*f, *i),
                _ => return None,
            });

            let m = elaborate_expression(ctx, (*expression)?)?;
            let binder = elaborate_binder(ctx, (*binder)?)?;
            let body = elaborate_do_statements(ctx, bind, discard, rest)?;

            Some(Expr::App(
                Box::new(Expr::App(Box::new(bind_expr), Box::new(m))),
                Box::new(Expr::Abs(binder, Box::new(body))),
            ))
        }
        lowering::DoStatement::Let { statements } => {
            let mut all_bindings = Vec::new();
            for chunk in statements.iter() {
                all_bindings.extend(elaborate_let_binding_chunk(ctx, chunk)?);
            }
            let body = elaborate_do_statements(ctx, bind, discard, rest)?;
            Some(Expr::Let(all_bindings, Box::new(body)))
        }
        lowering::DoStatement::Discard { expression } => {
            let discard_res = discard.as_ref().or_else(|| {
                None
            })?;
            let discard_expr = Expr::Var(match discard_res {
                lowering::TermVariableResolution::Reference(f, i) => Var::Module(*f, *i),
                _ => return None,
            });

            let m = elaborate_expression(ctx, (*expression)?)?;
            let body = elaborate_do_statements(ctx, bind, discard, rest)?;

            // discard m (\_ -> body)
            Some(Expr::App(
                Box::new(Expr::App(Box::new(discard_expr), Box::new(m))),
                Box::new(Expr::Abs(Binder::Wildcard, Box::new(body))),
            ))
        }
    }
}

fn elaborate_ado<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    map: &Option<lowering::TermVariableResolution>,
    apply: &Option<lowering::TermVariableResolution>,
    pure: &Option<lowering::TermVariableResolution>,
    statements: &[lowering::DoStatementId],
    expression: &Option<lowering::ExpressionId>,
) -> Option<Expr>
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    if statements.is_empty() {
        let pure_res = pure.as_ref().or_else(|| {
             None
        })?;
        let pure_expr = Expr::Var(match pure_res {
            lowering::TermVariableResolution::Reference(f, i) => Var::Module(*f, *i),
            lowering::TermVariableResolution::Let(l_id) => Var::Local(indexed_name_for_let(ctx, *l_id)),
            lowering::TermVariableResolution::Binder(b_id) => {
                Var::Local(ctx.binder_names.get(b_id).cloned().unwrap_or_else(|| SmolStr::new("unknown_binder")))
            }
            _ => {
                return None;
            }
        });
        let e = elaborate_expression(ctx, (*expression).or_else(|| {
             None
        })?)?;
        return Some(Expr::App(Box::new(pure_expr), Box::new(e)));
    }

    let mut binders = Vec::new();
    let mut expressions = Vec::new();
    let mut lets = Vec::new();

    for (i, &stmt_id) in statements.iter().enumerate() {
        let stmt = ctx.lowered.info.get_do_statement(stmt_id).or_else(|| {
             None
        })?;
        match stmt {
            lowering::DoStatement::Bind { binder, expression } => {
                binders.push(elaborate_binder(ctx, (*binder).or_else(|| {
                     None
                })?)?);
                expressions.push(elaborate_expression(ctx, (*expression).or_else(|| {
                     None
                })?)?);
            }
            lowering::DoStatement::Discard { expression } => {
                binders.push(Binder::Wildcard);
                expressions.push(elaborate_expression(ctx, (*expression).or_else(|| {
                     None
                })?)?);
            }
            lowering::DoStatement::Let { statements } => {
                for chunk in statements.iter() {
                    lets.extend(elaborate_let_binding_chunk(ctx, chunk)?);
                }
            }
        }
    }

    let mut body = elaborate_expression(ctx, (*expression).or_else(|| {
         None
    })?)?;
    if !lets.is_empty() {
        body = Expr::Let(lets, Box::new(body));
    }

    for binder in binders.iter().rev() {
        body = Expr::Abs(binder.clone(), Box::new(body));
    }

    let (first_expr, rest_exprs) = expressions.split_first().or_else(|| {
         None
    })?;

    let map_res = map.as_ref().or_else(|| {
         None
    })?;
    let map_expr = Expr::Var(match map_res {
        lowering::TermVariableResolution::Reference(f, i) => Var::Module(*f, *i),
        lowering::TermVariableResolution::Let(l_id) => Var::Local(indexed_name_for_let(ctx, *l_id)),
        lowering::TermVariableResolution::Binder(b_id) => {
            Var::Local(ctx.binder_names.get(b_id).cloned().unwrap_or_else(|| SmolStr::new("unknown_binder")))
        }
        _ => {
             return None;
        }
    });

    let mut current = Expr::App(
        Box::new(Expr::App(Box::new(map_expr), Box::new(body))),
        Box::new(first_expr.clone()),
    );

    if !rest_exprs.is_empty() {
        let apply_res = apply.as_ref().or_else(|| {
             None
        })?;
        let apply_expr = Expr::Var(match apply_res {
            lowering::TermVariableResolution::Reference(f, i) => Var::Module(*f, *i),
            lowering::TermVariableResolution::Let(l_id) => Var::Local(indexed_name_for_let(ctx, *l_id)),
            lowering::TermVariableResolution::Binder(b_id) => {
                Var::Local(ctx.binder_names.get(b_id).cloned().unwrap_or_else(|| SmolStr::new("unknown_binder")))
            }
            _ => {
                 return None;
            }
        });

        for (i, m) in rest_exprs.iter().enumerate() {
            current = Expr::App(
                Box::new(Expr::App(Box::new(apply_expr.clone()), Box::new(current))),
                Box::new(m.clone()),
            );
        }
    }

    Some(current)
}

fn elaborate_record_update<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    update: &lowering::RecordUpdate,
) -> Option<corefn::RecordUpdateItem>
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    match update {
        lowering::RecordUpdate::Leaf { name, expression } => {
            let expr = elaborate_expression(ctx, (*expression)?)?;
            Some(corefn::RecordUpdateItem::Leaf(name.clone()?, expr))
        }
        lowering::RecordUpdate::Branch { name, updates } => {
            let core_updates = updates.iter().map(|u| elaborate_record_update(ctx, u)).collect::<Option<Vec<_>>>()?;
            Some(corefn::RecordUpdateItem::Branch(name.clone()?, core_updates))
        }
    }
}

fn elaborate_binder<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    id: lowering::BinderId,
) -> Option<Binder> 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    let kind = ctx.lowered.info.get_binder_kind(id)?;
    match kind {
        BinderKind::Variable { variable } => {
            let name = variable.clone().unwrap_or_else(|| SmolStr::new("_"));
            ctx.binder_names.insert(id, name.clone());
            Some(Binder::Var(name))
        }
        BinderKind::Wildcard => Some(Binder::Wildcard),
        BinderKind::Constructor { resolution, arguments } => {
            let (f, i) = (*resolution)?;
            let core_args = arguments.iter().map(|&a_id| {
                elaborate_binder(ctx, a_id)
            }).collect::<Option<Vec<_>>>()?;
            Some(Binder::Constructor(f, i, core_args))
        }
        BinderKind::Integer { value } => {
            Some(Binder::Literal(corefn::LiteralBinder::Int(value.unwrap_or(0))))
        }
        BinderKind::Boolean { boolean } => {
            Some(Binder::Literal(corefn::LiteralBinder::Boolean(*boolean)))
        }
        BinderKind::String { value, .. } => {
            Some(Binder::Literal(corefn::LiteralBinder::String(value.clone().unwrap_or_default())))
        }
        BinderKind::Char { value } => {
            Some(Binder::Literal(corefn::LiteralBinder::Char(value.unwrap_or('\0'))))
        }
        BinderKind::Parenthesized { parenthesized } => {
            elaborate_binder(ctx, (*parenthesized)?)
        }
        BinderKind::Named { named, binder } => {
            let name = named.clone()?;
            let inner = elaborate_binder(ctx, (*binder)?)?;
            ctx.binder_names.insert(id, name.clone());
            Some(Binder::Named(name, Box::new(inner)))
        }
        BinderKind::Typed { binder, .. } => {
            elaborate_binder(ctx, (*binder)?)
        }
        BinderKind::Array { array } => {
            let mut core_args = Vec::new();
            for b_id in array.iter() {
                core_args.push(elaborate_binder(ctx, *b_id)?);
            }
            Some(Binder::Literal(corefn::LiteralBinder::Array(core_args)))
        }
        BinderKind::Record { record } => {
            let mut fields = FxHashMap::default();
            for item in record.iter() {
                match item {
                    lowering::BinderRecordItem::RecordField { name: Some(name), value: Some(value) } => {
                        fields.insert(name.clone(), elaborate_binder(ctx, *value)?);
                    }
                    lowering::BinderRecordItem::RecordPun { id: pun_id, name: Some(name) } => {
                        ctx.pun_names.insert(*pun_id, name.clone());
                        fields.insert(name.clone(), Binder::Var(name.clone()));
                    }
                    _ => {}
                }
            }
            Some(Binder::Literal(corefn::LiteralBinder::Object(fields)))
        }
        BinderKind::Number { value, .. } => {
            Some(Binder::Literal(corefn::LiteralBinder::Number(value.clone().unwrap_or_default())))
        }
        _ => {
            None
        }
    }
}


fn inject_evidence<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    expr: Expr,
    evidence: &Evidence,
) -> Expr 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    match evidence {
        Evidence::Instance(f, i, sub_evidences) => {
            let term_id = if f == &ctx._file_id {
                ctx.lowered.info.get_instance_term(*i)
            } else if let Ok(lowered) = ctx.queries.lowered(*f) {
                lowered.info.get_instance_term(*i)
            } else {
                None
            };
            
            if let Some(term_id) = term_id {
                let var = Expr::Var(Var::Module(*f, term_id));
                let mut current = var;
                for sub in sub_evidences {
                    let sub_expr = inject_evidence_inner(ctx, sub);
                    current = Expr::App(Box::new(current), Box::new(sub_expr));
                }
                return Expr::App(Box::new(expr), Box::new(current));
            }
            expr
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
        Evidence::Superclass(_f, base, index) => {
            let base_expr = inject_evidence_inner(ctx, base);
            let name = SmolStr::from(format!("superclass_{}", index));
            Expr::App(Box::new(expr), Box::new(Expr::Accessor(name, Box::new(base_expr))))
        }
        Evidence::Record(fields) => {
             let mut core_fields = FxHashMap::default();
             for (name, ev) in fields {
                 core_fields.insert(name.clone(), inject_evidence_inner(ctx, ev));
             }
             Expr::App(Box::new(expr), Box::new(Expr::Literal(Literal::Object(core_fields))))
        }
        _ => expr,
    }
}

fn inject_evidence_inner<Q: QueryProxy>(
    ctx: &mut ElaborationContext<Q>,
    evidence: &Evidence,
) -> Expr 
where
    Q::Lowered: std::ops::Deref<Target = LoweredModule>,
{
    match evidence {
        Evidence::Instance(f, i, sub_evidences) => {
            let term_id = if f == &ctx._file_id {
                ctx.lowered.info.get_instance_term(*i)
            } else if let Ok(lowered) = ctx.queries.lowered(*f) {
                lowered.info.get_instance_term(*i)
            } else {
                None
            };
            
            if let Some(term_id) = term_id {
                let var = Expr::Var(Var::Module(*f, term_id));
                let mut current = var;
                for sub in sub_evidences {
                    let sub_expr = inject_evidence_inner(ctx, sub);
                    current = Expr::App(Box::new(current), Box::new(sub_expr));
                }
                return current;
            }
            Expr::Literal(Literal::String(SmolStr::new("missing_instance")))
        }
        Evidence::Given(type_id) => {
            let name = SmolStr::from(format!("dict_{}", type_id.id.get()));
            Expr::Var(Var::Local(name))
        }
        Evidence::Superclass(_f, base, index) => {
            let base_expr = inject_evidence_inner(ctx, base);
            let name = SmolStr::from(format!("superclass_{}", index));
            Expr::Accessor(name, Box::new(base_expr))
        }
        Evidence::Record(fields) => {
            let mut core_fields = FxHashMap::default();
            for (name, ev) in fields {
                core_fields.insert(name.clone(), inject_evidence_inner(ctx, ev));
            }
            Expr::Literal(Literal::Object(core_fields))
        }
        _ => Expr::Literal(Literal::String(SmolStr::new("compiler_magic"))),
    }
}
