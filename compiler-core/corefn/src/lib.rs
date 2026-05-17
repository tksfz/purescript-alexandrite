use std::sync::Arc;

use files::FileId;
use indexing::{TermItemId, TypeItemId};
use la_arena::{Arena, Idx};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

pub type ExprId = Idx<Expr>;
pub type BinderId = Idx<Binder>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreFnModule {
    pub file_id: FileId,
    pub name: SmolStr,
    pub imports: Vec<SmolStr>,
    pub exports: Vec<SmolStr>,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Declaration {
    Value {
        name: SmolStr,
        expression: Expr,
    },
    Data {
        name: SmolStr,
        constructors: Vec<Constructor>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Constructor {
    pub name: SmolStr,
    pub fields: Vec<SmolStr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    Literal(Literal),
    Var(Var),
    Abs(Binder, Box<Expr>),
    App(Box<Expr>, Box<Expr>),
    Let(Vec<Binding>, Box<Expr>),
    Case(Vec<Expr>, Vec<CaseAlternative>),
    Constructor(FileId, TermItemId),
    Accessor(SmolStr, Box<Expr>),
    ObjectUpdate(Box<Expr>, FxHashMap<SmolStr, Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Literal {
    Int(i32),
    Number(SmolStr),
    String(SmolStr),
    Char(char),
    Boolean(bool),
    Array(Vec<Expr>),
    Object(FxHashMap<SmolStr, Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Var {
    Local(SmolStr),
    Module(FileId, TermItemId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binder {
    Var(SmolStr),
    Literal(LiteralBinder),
    Constructor(FileId, TermItemId, Vec<Binder>),
    Named(SmolStr, Box<Binder>),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiteralBinder {
    Int(i32),
    Number(SmolStr),
    String(SmolStr),
    Char(char),
    Boolean(bool),
    Array(Vec<Binder>),
    Object(FxHashMap<SmolStr, Binder>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub name: SmolStr,
    pub expression: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaseAlternative {
    pub binders: Vec<Binder>,
    pub result: CaseResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseResult {
    Expression(Expr),
    Guarded(Vec<Guard>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    pub condition: Expr,
    pub result: Expr,
}
