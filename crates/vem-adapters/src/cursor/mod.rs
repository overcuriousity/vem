//! Cursor adapters (spec §6.3): a collected `.cursor` directory (`cursor`) and Cursor's IDE application
//! directory (`cursor-ide`).

pub mod sqlite;

use std::path::Path;
use vem_core::adapter::{
    Discovery, FileContext, HarnessAdapter, Identification, ParseError, ParseOutcome,
};
use vem_core::model::Harness;
use vem_core::sink::ParseSink;

pub struct CursorAdapter;

impl HarnessAdapter for CursorAdapter {
    fn harness(&self) -> Harness {
        Harness::Cursor
    }

    fn identify(&self, _root: &Path) -> Option<Identification> {
        None
    }

    fn discover(&self, _root: &Path) -> Discovery {
        Discovery::default()
    }

    fn parse_file(
        &self,
        _ctx: &FileContext<'_>,
        _sink: &mut dyn ParseSink,
    ) -> Result<ParseOutcome, ParseError> {
        Ok(ParseOutcome::NotParsed)
    }
}

pub struct CursorIdeAdapter;

impl HarnessAdapter for CursorIdeAdapter {
    fn harness(&self) -> Harness {
        Harness::CursorIde
    }

    fn identify(&self, _root: &Path) -> Option<Identification> {
        None
    }

    fn discover(&self, _root: &Path) -> Discovery {
        Discovery::default()
    }

    fn parse_file(
        &self,
        _ctx: &FileContext<'_>,
        _sink: &mut dyn ParseSink,
    ) -> Result<ParseOutcome, ParseError> {
        Ok(ParseOutcome::NotParsed)
    }
}
