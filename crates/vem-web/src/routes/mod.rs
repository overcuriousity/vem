//! One module per API area; each exposes `routes()` merged by `crate::app`.

pub mod activity;
pub mod annotations;
pub mod case;
pub mod drill;
pub mod exports;
pub mod search;
pub mod sessions;
