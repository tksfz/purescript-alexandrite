use std::fmt::Write;
use std::sync::Arc;

use analyzer::{QueryEngine};
use diagnostics::{DiagnosticsContext, ToDiagnostics, format_rustc};
use files::FileId;
use smol_str::SmolStr;

macro_rules! pos {
    ($content:expr, $stabilized:expr, $id:expr) => {{
        let cst = $stabilized.ast_ptr($id).unwrap();
        let range = cst.syntax_node_ptr().text_range();
        let line_index = line_index::LineIndex::new($content);
        let pos = line_index.line_col(range.start());
        format!("{}:{}", pos.line + 1, pos.col + 1)
    }};
}

pub fn report_lowered(engine: &QueryEngine, id: FileId, content: &str) -> String {
    let lowered = engine.lowered(id).unwrap();
    let stabilized = engine.stabilized(id).unwrap();

    let mut out = String::default();
    writeln!(out, "module {}", content).unwrap();

    for (id, kind) in lowered.info.iter_expression() {
        writeln!(out).unwrap();
        writeln!(out, "expression @ {} =", pos!(content, stabilized, id)).unwrap();
        writeln!(out, "  {kind:?}").unwrap();
    }

    for (id, kind) in lowered.info.iter_binder() {
        writeln!(out).unwrap();
        writeln!(out, "binder @ {} =", pos!(content, stabilized, id)).unwrap();
        writeln!(out, "  {kind:?}").unwrap();
    }

    out
}

pub fn report_resolved(engine: &QueryEngine, id: FileId, content: &str) -> String {
    let resolved = engine.resolved(id).unwrap();
    let _stabilized = engine.stabilized(id).unwrap();

    let mut out = String::default();
    writeln!(out, "module {}", content).unwrap();

    for (name, f_id, t_id) in resolved.locals.iter_types() {
        writeln!(out).unwrap();
        writeln!(out, "local {} -> {:?}:{:?}", name, f_id, t_id).unwrap();
    }

    out
}

pub fn report_checked(engine: &QueryEngine, id: FileId) -> String {
    let checked = engine.checked(id).unwrap();
    let (parsed, _) = engine.parsed(id).unwrap();
    let root = parsed.syntax_node();

    let mut out = String::default();
    let content = engine.content(id);
    let stabilized = engine.stabilized(id).unwrap();
    let context = DiagnosticsContext {
        queries: engine,
        content: &content,
        root: &root,
        stabilized: &stabilized,
        indexed: &engine.indexed(id).unwrap(),
        lowered: &engine.lowered(id).unwrap(),
    };

    for error in &checked.errors {
        for diagnostic in error.to_diagnostics(&context) {
            writeln!(out, "{}", format_rustc(&[diagnostic], &context.content)).unwrap();
        }
    }

    out
}

pub fn report_elaborated(engine: &QueryEngine, id: FileId) -> String {
    let elaborated = engine.elaborated(id).unwrap();

    let mut out = String::default();
    writeln!(out, "module {}", elaborated.name).unwrap();

    for decl in &elaborated.declarations {
        match decl {
            corefn::Declaration::Value { name, expression } => {
                writeln!(out).unwrap();
                writeln!(out, "value {} =", name).unwrap();
                let json = serde_json::to_string_pretty(expression).unwrap();
                for line in json.lines() {
                    writeln!(out, "  {}", line).unwrap();
                }
            }
            corefn::Declaration::Data { name, constructors } => {
                writeln!(out).unwrap();
                writeln!(out, "data {} =", name).unwrap();
                for ctor in constructors {
                    writeln!(out, "  | {} {:?}", ctor.name, ctor.fields).unwrap();
                }
            }
        }
    }

    out
}

pub fn report_evaluated(engine: &QueryEngine, id: FileId) -> String {
    let elaborated = engine.elaborated(id).unwrap();
    let mut env = evaluating::Environment::new();
    let output = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    // Helpers to create FFIs
    let eq_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let a = args[0].clone();
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let b = args2[0].clone();
                match (a.clone(), b) {
                    (evaluating::Value::Int(i1), evaluating::Value::Int(i2)) => {
                        Ok(evaluating::Value::Boolean(i1 == i2))
                    }
                    (evaluating::Value::String(s1), evaluating::Value::String(s2)) => {
                        Ok(evaluating::Value::Boolean(s1 == s2))
                    }
                    (evaluating::Value::Boolean(b1), evaluating::Value::Boolean(b2)) => {
                        Ok(evaluating::Value::Boolean(b1 == b2))
                    }
                    _ => Ok(evaluating::Value::Boolean(false)),
                }
            })))
        }))
    };

    let add_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let a = match args[0] {
                evaluating::Value::Int(i) => i,
                _ => 0,
            };
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let b = match args2[0] {
                    evaluating::Value::Int(i) => i,
                    _ => 0,
                };
                Ok(evaluating::Value::Int(a + b))
            })))
        }))
    };

    let sub_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let a = match args[0] {
                evaluating::Value::Int(i) => i,
                _ => 0,
            };
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let b = match args2[0] {
                    evaluating::Value::Int(i) => i,
                    _ => 0,
                };
                Ok(evaluating::Value::Int(a - b))
            })))
        }))
    };

    let bind_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let m = args[0].clone();
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let f = args2[0].clone();
                // Apply m to unit to get the value
                let val = evaluating::apply(m.clone(), evaluating::Value::Object(Default::default()))
                    .unwrap_or(m.clone());
                // Apply f to the value
                evaluating::apply(f, val)
            })))
        }))
    };

    let discard_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let m = args[0].clone();
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let f = args2[0].clone();
                // Apply m to unit to execute side effects
                let _ = evaluating::apply(m.clone(), evaluating::Value::Object(Default::default()));
                // Apply f to unit
                evaluating::apply(f, evaluating::Value::Object(Default::default()))
            })))
        }))
    };

    let pure_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            Ok(args[0].clone())
        }))
    };

    let map_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let f = args[0].clone();
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let m = args2[0].clone();
                evaluating::apply(f.clone(), m)
            })))
        }))
    };

    let applicative_apply_ffi = || {
        evaluating::Value::Foreign(Arc::new(|args| {
            let f_wrapped = args[0].clone();
            Ok(evaluating::Value::Foreign(Arc::new(move |args2| {
                let m = args2[0].clone();
                // f_wrapped is already the function (result of map f m1)
                evaluating::apply(f_wrapped.clone(), m)
            })))
        }))
    };

    let log_output = Arc::clone(&output);
    let log_ffi = move || {
        let log_output = Arc::clone(&log_output);
        evaluating::Value::Foreign(Arc::new(move |args| {
            let msg = format!("{:?}", args[0]);
            let log_output = Arc::clone(&log_output);
            Ok(evaluating::Value::Foreign(Arc::new(move |_| {
                log_output.lock().unwrap().push(msg.clone());
                Ok(evaluating::Value::Object(Default::default()))
            })))
        }))
    };

    let lowered = engine.lowered(id).unwrap();
    let indexed = engine.indexed(id).unwrap();

    // Register FFIs in locals
    env.locals.insert(SmolStr::new("eq"), eq_ffi());
    env.locals.insert(SmolStr::new("add"), add_ffi());
    env.locals.insert(SmolStr::new("sub"), sub_ffi());
    env.locals.insert(SmolStr::new("log"), log_ffi());
    env.locals.insert(SmolStr::new("bind"), bind_ffi());
    env.locals.insert(SmolStr::new("discard"), discard_ffi());
    env.locals.insert(SmolStr::new("pure"), pure_ffi());
    env.locals.insert(SmolStr::new("map"), map_ffi());
    env.locals.insert(SmolStr::new("apply"), applicative_apply_ffi());

    // Pass 0: Register placeholders and FFIs
    for (item_id, term_item) in lowered.info.iter_term_item() {
        match term_item {
            lowering::TermItemIr::Constructor { .. } => {
                let val = evaluating::Value::Constructor {
                    file_id: id,
                    term_id: item_id,
                    arguments: vec![],
                };
                env.modules.write().unwrap().insert((id, item_id), val);
            }
            lowering::TermItemIr::Instance { .. } => {
                let val = evaluating::Value::Object(Default::default());
                env.modules.write().unwrap().insert((id, item_id), val);
            }
            lowering::TermItemIr::Foreign { .. } => {
                if let Some(name) = &indexed.items[item_id].name {
                    let ffi_val = match name.as_str() {
                        "eq" => Some(eq_ffi()),
                        "add" => Some(add_ffi()),
                        "sub" => Some(sub_ffi()),
                        "log" => Some(log_ffi()),
                        "bind" => Some(bind_ffi()),
                        "discard" => Some(discard_ffi()),
                        "pure" => Some(pure_ffi()),
                        "map" => Some(map_ffi()),
                        "apply" => Some(applicative_apply_ffi()),
                        _ => None,
                    };
                    if let Some(val) = ffi_val {
                        env.modules.write().unwrap().insert((id, item_id), val);
                    } else {
                        env.modules.write().unwrap().insert((id, item_id), evaluating::Value::String(SmolStr::new("foreign_placeholder")));
                    }
                }
            }
            lowering::TermItemIr::Operator { resolution, .. } => {
                let mut registered = false;
                if let Some((f_id, t_id)) = resolution {
                    let target_indexed = if *f_id == id {
                        &indexed
                    } else {
                        // For simplicity, we only handle operators in the same file for now in this test runner.
                        &indexed
                    };
                    
                    if let Some(alias_name) = &target_indexed.items[*t_id].name {
                        let ffi_val = match alias_name.as_str() {
                            "eq" => Some(eq_ffi()),
                            "add" => Some(add_ffi()),
                            "sub" => Some(sub_ffi()),
                            "log" => Some(log_ffi()),
                            "bind" => Some(bind_ffi()),
                            "discard" => Some(discard_ffi()),
                            "pure" => Some(pure_ffi()),
                            "map" => Some(map_ffi()),
                            "apply" => Some(applicative_apply_ffi()),
                            _ => None,
                        };
                        if let Some(val) = ffi_val {
                            env.modules.write().unwrap().insert((id, item_id), val);
                            registered = true;
                        }
                    }
                }
                
                if !registered {
                    env.modules.write().unwrap().insert((id, item_id), evaluating::Value::String(SmolStr::new("operator_placeholder")));
                }
            }
            _ => {
                env.modules.write().unwrap().insert((id, item_id), evaluating::Value::String(SmolStr::new("placeholder")));
            }
        }
    }

    // Pass 1: Actual evaluation to fix recursive closures
    for _ in 0..10 {
        for (item_id, _) in lowered.info.iter_term_item() {
            if let Some(name) = &indexed.items[item_id].name {
                if let Some(decl) = elaborated.declarations.iter().find(|d| d.name() == name) {
                    if let corefn::Declaration::Value { expression, .. } = decl {
                        if let Ok(val) = evaluating::eval(expression, &env) {
                            env.modules.write().unwrap().insert((id, item_id), val);
                        }
                    }
                }
            }
        }
    }

    let mut out = String::default();
    writeln!(out, "module {} (evaluated)", elaborated.name).unwrap();

    for decl in &elaborated.declarations {
        let name = decl.name();
        if name == "test" || name == "main" {
            if let corefn::Declaration::Value { expression, .. } = decl {
                match evaluating::eval(expression, &env) {
                    Ok(val) => {
                        let final_val = match val {
                            evaluating::Value::Closure { .. } | evaluating::Value::Foreign(_) => {
                                evaluating::apply(val.clone(), evaluating::Value::Object(Default::default()))
                                    .unwrap_or(val)
                            }
                            _ => val,
                        };

                        writeln!(out).unwrap();
                        writeln!(out, "value {} = {:?}", name, final_val).unwrap();

                        let logs = output.lock().unwrap();
                        if !logs.is_empty() {
                            writeln!(out, "logs:").unwrap();
                            for log in logs.iter() {
                                writeln!(out, "  {}", log).unwrap();
                            }
                        }
                    }
                    Err(e) => {
                        writeln!(out).unwrap();
                        writeln!(out, "value {} = Error: {}", name, e).unwrap();
                    }
                }
            }
        }
    }

    out
}
