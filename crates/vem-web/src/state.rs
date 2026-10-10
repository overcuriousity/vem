//! Shared state: the case directory, the bound port, and one connection for short queries.

use crate::ApiError;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use vem_case::{Case, CaseError};

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    case_dir: PathBuf,
    port: u16,
    case: Mutex<Case>,
}

impl AppState {
    pub fn open(case_dir: &Path, port: u16) -> Result<Self, CaseError> {
        let case = Case::open(case_dir)?;
        Ok(Self {
            inner: Arc::new(Inner {
                case_dir: case_dir.to_path_buf(),
                port,
                case: Mutex::new(case),
            }),
        })
    }

    pub fn port(&self) -> u16 {
        self.inner.port
    }

    pub fn case_dir(&self) -> &Path {
        &self.inner.case_dir
    }

    /// Runs `f` on the shared connection in a blocking thread. For short queries only.
    pub async fn with_case<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Case) -> Result<T, CaseError> + Send + 'static,
    {
        let s = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut case = s.inner.case.lock().unwrap_or_else(|p| p.into_inner());
            f(&mut case)
        })
        .await
        .map_err(ApiError::join)?
        .map_err(ApiError::from)
    }

    /// Runs `f` on a fresh connection of its own, so long work (export, diff, raw bytes, blobs) never
    /// blocks the shared one.
    pub async fn with_own_case<T, F>(&self, f: F) -> Result<T, ApiError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Case) -> Result<T, CaseError> + Send + 'static,
    {
        let dir = self.inner.case_dir.clone();
        tokio::task::spawn_blocking(move || {
            let mut case = Case::open(&dir)?;
            f(&mut case)
        })
        .await
        .map_err(ApiError::join)?
        .map_err(ApiError::from)
    }
}
