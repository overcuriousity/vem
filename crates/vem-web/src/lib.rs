//! `vem serve`: a JSON API over one case plus the embedded frontend, bound to 127.0.0.1 only.

mod assets;
mod error;
mod guard;
pub mod routes;
mod state;

pub use error::ApiError;
pub use state::AppState;

use axum::Router;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use vem_case::CaseError;

/// The full application for the case at `case_dir`, answering only to `Host: 127.0.0.1:<port>` or
/// `localhost:<port>`.
pub fn app(case_dir: &Path, port: u16) -> Result<Router, CaseError> {
    let state = AppState::open(case_dir, port)?;
    Ok(Router::new()
        .merge(routes::case::routes())
        .merge(routes::sessions::routes())
        .merge(routes::drill::routes())
        .merge(routes::activity::routes())
        .merge(routes::search::routes())
        .merge(routes::annotations::routes())
        .merge(routes::exports::routes())
        .fallback(assets::static_handler)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            guard::guard,
        ))
        .layer(tower_http::catch_panic::CatchPanicLayer::custom(
            error::panic_response,
        ))
        .with_state(state))
}

/// Binds 127.0.0.1:`port` (0 picks a free port), prints the URL, opens it in the default browser when
/// `open` is set, and serves until interrupted.
pub async fn serve(case_dir: PathBuf, port: u16, open: bool) -> Result<(), CaseError> {
    vem_case::Case::open(&case_dir)?; // refuse a non-case directory before binding
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await?;
    let port = listener.local_addr()?.port();
    let router = app(&case_dir, port)?;
    println!(
        "vem: serving {} at http://127.0.0.1:{port}/ (Ctrl-C to stop)",
        case_dir.display()
    );
    if open {
        open_browser(&format!("http://127.0.0.1:{port}/"));
    }
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Hands the loopback URL to the platform's opener. This launches a local program and makes no network
/// request; a failure is reported and serving continues.
fn open_browser(url: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    let spawned = cmd
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Err(e) = spawned {
        eprintln!("vem: could not open a browser ({e}); open {url} yourself");
    }
}
