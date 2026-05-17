use std::fmt::Write;
use std::sync::Arc;

use analyzer::{QueryEngine};
use diagnostics::{DiagnosticsContext, ToDiagnostics, format_rustc};
use files::FileId;
use indexing::{TermItemId, TypeItemId};
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

    // Register FFIs for the current module's foreign imports
    let lowered = engine.lowered(id).unwrap();
    let indexed = engine.indexed(id).unwrap();
    for (item_id, term_item) in lowered.info.iter_term_item() {
        if let lowering::TermItemIr::Foreign { .. } = term_item {
            if let Some(name) = &indexed.items[item_id].name {
                let ffi_val = match name.as_str() {
                    "eq" => Some(eq_ffi()),
                    "add" => Some(add_ffi()),
                    "sub" => Some(sub_ffi()),
                    "log" => Some(log_ffi()),
                    _ => None,
                };
                if let Some(val) = ffi_val {
                    env.modules.write().unwrap().insert((id, item_id), val);
                }
            }
        }
    }

    // Also register in locals for easy access if not imported but defined as lambda
    env.locals.insert(SmolStr::new("eq"), eq_ffi());
    env.locals.insert(SmolStr::new("add"), add_ffi());
    env.locals.insert(SmolStr::new("sub"), sub_ffi());
    env.locals.insert(SmolStr::new("log"), log_ffi());

    // Populate module environment with current module values to support recursion
    for _ in 0..3 {
        for (item_id, _) in indexed.items.iter_terms() {
            if let Some(name) = &indexed.items[item_id].name {
                if let Some(decl) = elaborated.declarations.iter().find(|d| match d {
                    corefn::Declaration::Value { name: d_name, .. } => d_name == name,
                    _ => false,
                }) {
                    if let corefn::Declaration::Value { expression, .. } = decl {
                        if !env.modules.read().unwrap().contains_key(&(id, item_id)) {
                            if let Ok(val) = evaluating::eval(expression, &env) {
                                env.modules.write().unwrap().insert((id, item_id), val);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut out = String::default();
    writeln!(out, "module {} (evaluated)", elaborated.name).unwrap();

    for decl in &elaborated.declarations {
        if let corefn::Declaration::Value { name, expression } = decl {
            if name == "test" || name == "main" {
                match evaluating::eval(expression, &env) {
                    Ok(val) => {
                        // If it's an Effect, run it once by applying to unit
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
