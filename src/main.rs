#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;

use minidon_rust::platform::app::{build_app, AppState};
use minidon_rust::platform::clock::SystemClock;
use minidon_rust::platform::config::{bind_address_from, Settings};
use minidon_rust::platform::db::connect_and_verify;

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
    let bind_address =
        bind_address_from(|key| std::env::var(key).ok()).map_err(|e| e.to_string())?;
    let settings = Settings::from_env();
    let pool = connect_and_verify(&settings)
        .await
        .map_err(|e| e.to_string())?;
    let state = AppState {
        pool,
        clock: Arc::new(SystemClock),
        settings: Arc::new(settings),
    };
    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .map_err(|e| format!("cannot listen on {bind_address}: {e}"))?;
    axum::serve(listener, build_app(state))
        .await
        .map_err(|e| format!("server stopped: {e}"))
}
