use std::sync::Arc;

use inheritance_engine::{agents::Agents, api, app::App, config, db::Db, hunter::GitHub, models::ModelClient};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    config::load_dotenv(std::path::Path::new(".env"));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=warn".into());
    if std::env::var("IE_LOG_JSON").as_deref() == Ok("1") {
        tracing_subscriber::fmt().json().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    let cfg = Arc::new(config::Config::from_env()?);
    // Optional agent tracing; metadata only (see src/telemetry.rs).
    let _sentry = inheritance_engine::telemetry::init();
    if inheritance_engine::telemetry::enabled() {
        tracing::info!("Sentry agent tracing enabled (metadata only: no prompts, responses or source code)");
    }
    std::fs::create_dir_all(cfg.data_dir.join("inheritances"))?;
    let db = Arc::new(Db::open(&cfg.data_dir.join("engine.sqlite3"))?);
    let client = ModelClient::new(cfg.model.clone(), cfg.privacy_mode, cfg.allow_cloud)?;
    if let Err(e) = client.check_policy() {
        tracing::warn!("{e}");
    }
    let agents = Arc::new(Agents::new(client, cfg.clone(), db.clone()));
    let mastra = match &cfg.mastra_url {
        Some(url) => Some(inheritance_engine::mastra::MastraClient::new(url)?),
        None => None,
    };
    let app = App::new(cfg.clone(), db, agents, GitHub::new(cfg.github_token.clone())?, mastra);
    let interrupted = app.recover_jobs()?;
    if interrupted > 0 {
        tracing::warn!(interrupted, "jobs were interrupted by the last shutdown; resume them from the UI");
    }

    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!(
        bind = %cfg.bind, model = %cfg.model.model, endpoint = %cfg.model.endpoint, privacy = ?cfg.privacy_mode,
        "Inheritance Engine listening on http://{}", cfg.bind
    );
    axum::serve(listener, api::router(app)).await?;
    Ok(())
}
