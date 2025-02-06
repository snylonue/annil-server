use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use annil::{provider::AnnilProvider, state::AnnilKeys};
use annil_server::{make_app, make_state, provider::SeafileProvider};
use reqwest_dav::re_exports::reqwest;

#[derive(serde::Deserialize)]
struct SeafileConfig {
    token: String,
    base: String,
    repo_id: String,
}

#[derive(serde::Deserialize)]
struct Config {
    listen: SocketAddr,
    sign_key: String,
    share_key: String,
    admin_token: String,

    provider: SeafileConfig,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = clap::Command::new("AnnilServer").arg(
        clap::arg!(-c --config <FILE> "path to config file")
            .required(true)
            .value_parser(clap::value_parser!(PathBuf)),
    );

    let matches = app.get_matches();

    let config_file = matches
        .get_one::<PathBuf>("config")
        .expect("config file required");

    let config: Config = toml::from_str(&std::fs::read_to_string(config_file)?)?;

    let provider = Arc::new(AnnilProvider::new(SeafileProvider::new(
        reqwest::Client::new(),
        config.provider.token,
        config.provider.base,
        config.provider.repo_id,
    )));

    let initial_state = Arc::new(
        make_state(
            String::from(concat!("AnnilServer v", env!("CARGO_PKG_VERSION"))),
            &provider,
        )
        .await,
    );

    let key = Arc::new(AnnilKeys::new(
        config.sign_key.as_bytes(),
        config.share_key.as_bytes(),
        config.admin_token,
    ));

    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    axum::serve(listener, make_app(provider, initial_state, key))
        .await
        .map_err(Into::into)
}
