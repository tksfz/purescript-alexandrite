use std::sync::{Arc, RwLock};
use corefn::{Expr, Var, Literal, Binder, Binding, CaseAlternative, CaseResult};
use files::FileId;
use indexing::{TermItemId};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use thiserror::Error;

#[derive(Clone)]
pub enum Value {
    Int(i32),
    String(SmolStr),
    Char(char),
    Boolean(bool),
    Array(Vec<Value>),
    Object(FxHashMap<SmolStr, Value>),
    Constructor {
        file_id: FileId,
        term_id: TermItemId,
        arguments: Vec<Value>,
    },
    Closure {
        env: Environment,
        binder: Binder,
        body: Box<Expr>,
    },
    Foreign(Arc<dyn Fn(Vec<Value>) -> EvalResult<Value> + Send + Sync>),
}

impl std::fmt::Debug for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{:?}", i),
            Value::String(s) => write!(f, "{:?}", s),
            Value::Char(c) => write!(f, "{:?}", c),
            Value::Boolean(b) => write!(f, "{:?}", b),
            Value::Array(a) => write!(f, "{:?}", a),
            Value::Object(o) => write!(f, "{:?}", o),
            Value::Constructor { file_id, term_id, arguments } => {
                f.debug_struct("Constructor")
                    .field("file_id", file_id)
                    .field("term_id", term_id)
                    .field("arguments", arguments)
                    .finish()
            }
            Value::Closure { .. } => write!(f, "<closure>"),
            Value::Foreign(_) => write!(f, "<foreign>"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub locals: FxHashMap<SmolStr, Value>,
    pub modules: Arc<RwLock<FxHashMap<(FileId, TermItemId), Value>>>,
}

impl Environment {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Error, Debug)]
pub enum EvalError {
    #[error("Variable not found: {0}")]
    VariableNotFound(SmolStr),
    #[error("Module variable not found: {0:?}:{1:?}")]
    ModuleVariableNotFound(FileId, TermItemId),
    #[error("Not a function: {0}")]
    NotAFunction(String),
    #[error("FFI error: {0}")]
    FFIError(String),
}

pub type EvalResult<T> = Result<T, EvalError>;

fn apply_record_updates(
    fields: &mut FxHashMap<SmolStr, Value>,
    updates: &[corefn::RecordUpdateItem],
    env: &Environment,
) -> EvalResult<()> {
    for update in updates {
        match update {
            corefn::RecordUpdateItem::Leaf(name, expression) => {
                let val = eval(expression, env)?;
                fields.insert(name.clone(), val);
            }
            corefn::RecordUpdateItem::Branch(name, sub_updates) => {
                let val = fields.get_mut(name).ok_or_else(|| EvalError::VariableNotFound(name.clone()))?;
                match val {
                    Value::Object(sub_fields) => {
                        apply_record_updates(sub_fields, sub_updates, env)?;
                    }
                    _ => return Err(EvalError::NotAFunction(format!("Expected object for nested update, found {:?}", val))),
                }
            }
        }
    }
    Ok(())
}

pub fn eval(
    expr: &Expr,
    env: &Environment,
) -> EvalResult<Value> {
    match expr {
        Expr::Literal(lit) => match lit {
            Literal::Int(i) => Ok(Value::Int(*i)),
            Literal::String(s) => Ok(Value::String(s.clone())),
            Literal::Char(c) => Ok(Value::Char(*c)),
            Literal::Boolean(b) => Ok(Value::Boolean(*b)),
            Literal::Array(a) => {
                let mut vals = Vec::new();
                for e in a {
                    vals.push(eval(e, env)?);
                }
                Ok(Value::Array(vals))
            }
            Literal::Object(o) => {
                let mut fields = FxHashMap::default();
                for (name, e) in o {
                    fields.insert(name.clone(), eval(e, env)?);
                }
                Ok(Value::Object(fields))
            }
            _ => Err(EvalError::FFIError(format!("Unimplemented literal in eval: {:?}", lit))),
        },
        Expr::Var(var) => match var {
            Var::Local(name) => env
                .locals
                .get(name)
                .cloned()
                .ok_or_else(|| EvalError::VariableNotFound(name.clone())),
            Var::Module(file_id, term_id) => env
                .modules
                .read()
                .unwrap()
                .get(&(*file_id, *term_id))
                .cloned()
                .ok_or_else(|| EvalError::ModuleVariableNotFound(*file_id, *term_id)),
        },
        Expr::Abs(binder, body) => Ok(Value::Closure {
            env: env.clone(),
            binder: binder.clone(),
            body: body.clone(),
        }),
        Expr::App(function, argument) => {
            let func_val = eval(function, env)?;
            let arg_val = eval(argument, env)?;
            apply(func_val, arg_val)
        }
        Expr::Let(bindings, body) => {
            let mut current_env = env.clone();
            for binding in bindings {
                let dummy = Value::Closure {
                    env: current_env.clone(),
                    binder: Binder::Wildcard,
                    body: Box::new(binding.expression.clone()),
                };
                current_env.locals.insert(binding.name.clone(), dummy);
            }
            for _ in 0..5 {
                for binding in bindings {
                    if let Ok(val) = eval(&binding.expression, &current_env) {
                        current_env.locals.insert(binding.name.clone(), val);
                    }
                }
            }
            eval(body, &current_env)
        }
        Expr::Constructor(file_id, term_id) => {
            Ok(Value::Constructor {
                file_id: *file_id,
                term_id: *term_id,
                arguments: vec![],
            })
        }
        Expr::Accessor(name, expression) => {
            let val = eval(expression, env)?;
            match val {
                Value::Object(fields) => {
                    fields.get(name).cloned()
                        .ok_or_else(|| EvalError::VariableNotFound(name.clone()))
                }
                _ => Err(EvalError::NotAFunction(format!("Expected object, found {:?}", val))),
            }
        }
        Expr::RecordUpdate(record, updates) => {
            let val = eval(record, env)?;
            match val {
                Value::Object(fields) => {
                    let mut new_fields = fields;
                    apply_record_updates(&mut new_fields, updates, env)?;
                    Ok(Value::Object(new_fields))
                }
                _ => Err(EvalError::NotAFunction(format!("Expected object, found {:?}", val))),
            }
        }
        Expr::Case(expressions, alternatives) => {
            let vals = expressions.iter().map(|e| eval(e, env)).collect::<EvalResult<Vec<_>>>()?;
            for alt in alternatives {
                let mut case_env = env.clone();
                if match_alternative_into(alt, &vals, &mut case_env)? {
                    match &alt.result {
                        CaseResult::Expression(res_expr) => return eval(res_expr, &case_env),
                        _ => {}
                    }
                }
            }
            Err(EvalError::FFIError("No matching alternative in case".into()))
        }
    }
}

pub fn apply(
    function: Value,
    argument: Value,
) -> EvalResult<Value> {
    match function {
        Value::Closure { env, binder, body } => {
            let mut new_env = env.clone();
            bind_pattern(&mut new_env, &binder, argument)?;
            eval(&body, &new_env)
        }
        Value::Foreign(f) => f(vec![argument]),
        Value::Constructor { file_id, term_id, mut arguments } => {
            arguments.push(argument);
            Ok(Value::Constructor {
                file_id,
                term_id,
                arguments,
            })
        }
        _ => Err(EvalError::NotAFunction(format!("{:?}", function))),
    }
}

fn bind_pattern(
    env: &mut Environment,
    binder: &Binder,
    value: Value,
) -> EvalResult<()> {
    match binder {
        Binder::Var(name) => {
            env.locals.insert(name.clone(), value);
            Ok(())
        }
        Binder::Wildcard => Ok(()),
        Binder::Literal(lit) => match (lit, value) {
            (corefn::LiteralBinder::Int(i1), Value::Int(i2)) if *i1 == i2 => Ok(()),
            (corefn::LiteralBinder::Boolean(b1), Value::Boolean(b2)) if *b1 == b2 => Ok(()),
            (corefn::LiteralBinder::String(s1), Value::String(s2)) if s1 == &s2 => Ok(()),
            _ => Err(EvalError::FFIError("Pattern match failure".into())),
        },
        Binder::Constructor(f1, t1, binders) => match value {
            Value::Constructor { file_id: f2, term_id: t2, arguments } if *f1 == f2 && *t1 == t2 => {
                for (b, v) in binders.iter().zip(arguments.iter()) {
                    bind_pattern(env, b, v.clone())?;
                }
                Ok(())
            }
            _ => Err(EvalError::FFIError(format!("Pattern match failure: expected constructor {:?}:{:?}, found {:?}", f1, t1, value))),
        },
        _ => Err(EvalError::FFIError(format!("Unimplemented binder in eval: {:?}", binder))),
    }
}

fn match_alternative_into(
    alt: &CaseAlternative,
    values: &[Value],
    env: &mut Environment,
) -> EvalResult<bool> {
    if alt.binders.len() != values.len() {
        return Ok(false);
    }
    
    for (binder, value) in alt.binders.iter().zip(values) {
        if bind_pattern(env, binder, value.clone()).is_err() {
            return Ok(false);
        }
    }
    
    Ok(true)
}

