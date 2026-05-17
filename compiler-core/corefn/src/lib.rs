use std::sync::Arc;

use files::FileId;
use indexing::{TermItemId, TypeItemId};
use la_arena::{Idx, RawIdx};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize, Serializer, Deserializer};
use smol_str::SmolStr;

pub type ExprId = Idx<Expr>;
pub type BinderId = Idx<Binder>;

mod idx_serde {
    use super::*;

    pub fn serialize<T, S>(idx: &Idx<T>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u32(idx.into_raw().into_u32())
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Idx<T>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = u32::deserialize(deserializer)?;
        Ok(Idx::from_raw(RawIdx::from_u32(raw)))
    }
}

mod opt_idx_serde {
    use super::*;

    pub fn serialize<T, S>(idx: &Option<Idx<T>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match idx {
            Some(idx) => serializer.serialize_some(&idx.into_raw().into_u32()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, T, D>(deserializer: D) -> Result<Option<Idx<T>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = Option::<u32>::deserialize(deserializer)?;
        Ok(raw.map(|r| Idx::from_raw(RawIdx::from_u32(r))))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreFnModule {
    #[serde(with = "idx_serde")]
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

impl Declaration {
    pub fn name(&self) -> &SmolStr {
        match self {
            Declaration::Value { name, .. } => name,
            Declaration::Data { name, .. } => name,
        }
    }
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
    Constructor(#[serde(with = "idx_serde")] FileId, #[serde(with = "idx_serde")] TermItemId),
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
    Module(#[serde(with = "idx_serde")] FileId, #[serde(with = "idx_serde")] TermItemId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Binder {
    Var(SmolStr),
    Literal(LiteralBinder),
    Constructor(#[serde(with = "idx_serde")] FileId, #[serde(with = "idx_serde")] TermItemId, Vec<Binder>),
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
