use std::sync::{Arc, RwLock};
use std::fmt;

use corefn::{Expr, Var, Binder, Literal as AstLiteral};
use files::FileId;
use indexing::{TermItemId};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("Variable not found: {0}")]
    VariableNotFound(SmolStr),
    #[error("Module variable not found: {0:?}:{1:?}")]
    ModuleVariableNotFound(FileId, TermItemId),
    #[error("Not a function: {0}")]
    NotAFunction(String),
    #[error("Mismatched number of arguments in pattern matching")]
    MismatchedArguments,
    #[error("No case matched")]
    NoCaseMatched,
    #[error("FFI error: {0}")]
    FfiError(String),
}

pub type EvalResult<T> = Result<T, EvalError>;

#[derive(Clone)]
pub enum Value {
    Int(i32),
    Number(SmolStr),
    String(SmolStr),
    Char(char),
    Boolean(bool),
    Array(Vec<Value>),
    Object(FxHashMap<SmolStr, Value>),
    Closure {
        env: Environment,
        binder: Binder,
        body: Box<Expr>,
    },
    Constructor {
        file_id: FileId,
        term_id: TermItemId,
        arguments: Vec<Value>,
    },
    Foreign(Arc<dyn Fn(Vec<Value>) -> EvalResult<Value> + Send + Sync>),
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{}", i),
            Value::Number(n) => write!(f, "{}", n),
            Value::String(s) => write!(f, "{:?}", s),
            Value::Char(c) => write!(f, "{:?}", c),
            Value::Boolean(b) => write!(f, "{}", b),
            Value::Array(a) => write!(f, "{:?}", a),
            Value::Object(o) => write!(f, "{:?}", o),
            Value::Closure { .. } => write!(f, "<closure>"),
            Value::Constructor { file_id, term_id, arguments } => {
                write!(f, "Constructor({:?}, {:?}, {:?})", file_id, term_id, arguments)
            }
            Value::Foreign(_) => write!(f, "<foreign>"),
        }
    }
}

#[derive(Clone, Default)]
pub struct Environment {
    pub locals: FxHashMap<SmolStr, Value>,
    pub modules: Arc<RwLock<FxHashMap<(FileId, TermItemId), Value>>>,
}

impl Environment {
    pub fn new() -> Self {
        Self {
            locals: FxHashMap::default(),
            modules: Arc::new(RwLock::new(FxHashMap::default())),
        }
    }

    pub fn extend(&self, name: SmolStr, value: Value) -> Self {
        let mut new_env = self.clone();
        new_env.locals.insert(name, value);
        new_env.locals.shrink_to_fit();
        new_env
    }

    pub fn lookup_local(&self, name: &SmolStr) -> Option<Value> {
        self.locals.get(name).cloned()
    }

    pub fn lookup_module(&self, file_id: FileId, term_id: TermItemId) -> Option<Value> {
        self.modules.read().unwrap().get(&(file_id, term_id)).cloned()
    }
}

pub fn eval(expr: &Expr, env: &Environment) -> EvalResult<Value> {
    match expr {
        Expr::Literal(lit) => eval_literal(lit, env),
        Expr::Var(var) => match var {
            Var::Local(name) => env.lookup_local(name)
                .ok_or_else(|| EvalError::VariableNotFound(name.clone())),
            Var::Module(file_id, term_id) => env.lookup_module(*file_id, *term_id)
                .ok_or_else(|| EvalError::ModuleVariableNotFound(*file_id, *term_id)),
        },
        Expr::Abs(binder, body) => {
            Ok(Value::Closure {
                env: env.clone(),
                binder: binder.clone(),
                body: body.clone(),
            })
        }
        Expr::App(function, argument) => {
            let func_val = eval(function, env)?;
            let arg_val = eval(argument, env)?;
            apply(func_val, arg_val)
        }
        Expr::Let(bindings, body) => {
            let mut current_env = env.clone();
            for binding in bindings {
                let val = eval(&binding.expression, &current_env)?;
                current_env.locals.insert(binding.name.clone(), val);
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
        Expr::Case(expressions, alternatives) => {
             let vals = expressions.iter().map(|e| eval(e, env)).collect::<EvalResult<Vec<_>>>()?;
             for alt in alternatives {
                 if let Some(res_expr) = match_alternative(alt, &vals, env)? {
                     return eval(res_expr, env);
                 }
             }
             Err(EvalError::NoCaseMatched)
        }
        _ => Err(EvalError::FfiError(format!("Unimplemented expr in eval: {:?}", expr))),
    }
}

fn eval_literal(lit: &AstLiteral, env: &Environment) -> EvalResult<Value> {
    match lit {
        AstLiteral::Int(i) => Ok(Value::Int(*i)),
        AstLiteral::Number(n) => Ok(Value::Number(n.clone())),
        AstLiteral::String(s) => Ok(Value::String(s.clone())),
        AstLiteral::Char(c) => Ok(Value::Char(*c)),
        AstLiteral::Boolean(b) => Ok(Value::Boolean(*b)),
        AstLiteral::Array(a) => {
            let mut vals = Vec::new();
            for e in a {
                vals.push(eval(e, env)?);
            }
            Ok(Value::Array(vals))
        }
        AstLiteral::Object(o) => {
            let mut fields = FxHashMap::default();
            for (name, e) in o {
                fields.insert(name.clone(), eval(e, env)?);
            }
            Ok(Value::Object(fields))
        }
    }
}

pub fn apply(function: Value, argument: Value) -> EvalResult<Value> {
    match function {
        Value::Closure { env, binder, body } => {
            let mut new_env = env.clone();
            bind_pattern(&mut new_env, &binder, argument)?;
            eval(&body, &new_env)
        }
        Value::Constructor { file_id, term_id, mut arguments } => {
            arguments.push(argument);
            Ok(Value::Constructor { file_id, term_id, arguments })
        }
        Value::Foreign(f) => {
            f(vec![argument])
        }
        _ => Err(EvalError::NotAFunction(format!("{:?}", function))),
    }
}

fn bind_pattern(env: &mut Environment, binder: &Binder, value: Value) -> EvalResult<()> {
    match (binder, value) {
        (Binder::Var(name), val) => {
            env.locals.insert(name.clone(), val);
            Ok(())
        }
        (Binder::Wildcard, _) => Ok(()),
        (Binder::Literal(corefn::LiteralBinder::Boolean(b1)), Value::Boolean(b2)) if *b1 == b2 => Ok(()),
        (Binder::Literal(corefn::LiteralBinder::Int(i1)), Value::Int(i2)) if *i1 == i2 => Ok(()),
        (Binder::Constructor(f1, t1, binders), Value::Constructor { file_id: f2, term_id: t2, arguments }) if f1 == &f2 && t1 == &t2 => {
            for (b, v) in binders.iter().zip(arguments) {
                bind_pattern(env, b, v)?;
            }
            Ok(())
        }
        _ => Err(EvalError::NoCaseMatched),
    }
}

fn match_alternative<'a>(alt: &'a corefn::CaseAlternative, values: &[Value], env: &Environment) -> EvalResult<Option<&'a corefn::Expr>> {
    if alt.binders.len() != values.len() {
        return Err(EvalError::MismatchedArguments);
    }
    
    let mut current_env = env.clone();
    for (binder, val) in alt.binders.iter().zip(values) {
        if !match_binder(&mut current_env, binder, val)? {
            return Ok(None);
        }
    }
    
    match &alt.result {
        corefn::CaseResult::Expression(expr) => Ok(Some(expr)),
        corefn::CaseResult::Guarded(guards) => {
            for guard in guards {
                if let Value::Boolean(true) = eval(&guard.condition, &current_env)? {
                    return Ok(Some(&guard.result));
                }
            }
            Ok(None)
        }
    }
}

fn match_binder(env: &mut Environment, binder: &Binder, value: &Value) -> EvalResult<bool> {
    match (binder, value) {
        (Binder::Var(name), val) => {
            env.locals.insert(name.clone(), val.clone());
            Ok(true)
        }
        (Binder::Wildcard, _) => Ok(true),
        (Binder::Literal(corefn::LiteralBinder::Boolean(b1)), Value::Boolean(b2)) => Ok(b1 == b2),
        (Binder::Literal(corefn::LiteralBinder::Int(i1)), Value::Int(i2)) => Ok(i1 == i2),
        (Binder::Constructor(f1, t1, binders), Value::Constructor { file_id: f2, term_id: t2, arguments }) => {
            if f1 != f2 || t1 != t2 {
                return Ok(false);
            }
            if binders.len() != arguments.len() {
                return Ok(false);
            }
            for (b, v) in binders.iter().zip(arguments) {
                if !match_binder(env, b, v)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}
