//! Harness adapters. Each adapter is a pure function from evidence files to canonical drafts.

pub mod claude_code;

use std::path::Path;
use vem_core::adapter::{HarnessAdapter, Identification};
use vem_core::model::Harness;

pub fn adapters() -> Vec<Box<dyn HarnessAdapter>> {
    vec![Box::new(claude_code::ClaudeCodeAdapter)]
}

pub fn adapter_for(harness: Harness) -> Option<Box<dyn HarnessAdapter>> {
    adapters().into_iter().find(|a| a.harness() == harness)
}

/// Every harness whose content signatures match `root`. Zero means unrecognized, more than one means ambiguous.
pub fn identify_root(root: &Path) -> Vec<Identification> {
    adapters().iter().filter_map(|a| a.identify(root)).collect()
}
