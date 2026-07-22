//! tagma-core: the tagma engine — tag parsing, atom/infix/postfix query
//! compilation and evaluation over an in-memory index (SPEC.md, PLAN.md §7).
#![deny(missing_docs)]

pub mod atom;
pub mod index;
pub mod infix;
pub mod postfix;
pub mod tag;
pub mod token;
pub mod typecmp;

pub use atom::{Atom, Op, Pos};
pub use index::{tag_hidden, HideConfig, Index};
pub use tag::Tag;
pub use typecmp::TypeComparator;
