//! tagma-core: the tagma engine — tag parsing, atom/infix/postfix query
//! compilation and evaluation over an in-memory index (SPEC.md, PLAN.md §7).

pub mod atom;
pub mod index;
pub mod infix;
pub mod postfix;
pub mod tag;
pub mod token;

pub use atom::{Atom, Op, Pos};
pub use index::Index;
pub use tag::Tag;
