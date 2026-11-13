#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use minidon_rust::platform::app::{build_app, AppState};
use minidon_rust::platform::clock::SystemClock;
use minidon_rust::platform::config::Settings;
use minidon_rust::platform::db::connect_and_verify;

/// Same dev address as Python (`python -m minidon`).
const BIND_ADDRESS: &str = "127.0.0.1:8000";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("minidon-rust: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let settings = Settings::from_env();
    let pool = connect_and_verify(&settings)
        .await
        .map_err(|e| e.to_string())?;
    let state = AppState {
        pool,
        clock: Arc::new(SystemClock),
        settings: Arc::new(settings),
    };
    let listener = tokio::net::TcpListener::bind(BIND_ADDRESS)
        .await
        .map_err(|e| format!("cannot listen on {BIND_ADDRESS}: {e}"))?;
    axum::serve(listener, build_app(state))
        .await
        .map_err(|e| format!("server stopped: {e}"))
}
