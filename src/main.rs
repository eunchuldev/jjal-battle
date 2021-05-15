use actix_web::{App, HttpServer};
use futures::{try_join, TryFutureExt};
use serde::Deserialize;

mod bg_service;
mod constants;
mod error;
mod models;
mod pools;
mod routes;
mod session;
mod utils;

fn default_region() -> String {
    "ap_northeast_2".to_string()
}
fn default_port() -> u32 {
    8080
}

#[derive(Deserialize, Debug)]
struct Config {
    database_url: String,
    redis_url: String,
    bucket: String,
    s3_endpoint: Option<String>,
    #[serde(default = "default_region")]
    region: String,
    #[serde(default = "default_port")]
    port: u32,
}

#[actix_rt::main]
async fn main() -> Result<(), error::Error> {
    let config = match envy::from_env::<Config>() {
        Ok(config) => config,
        Err(err) => panic!("{:#?}", err),
    };

    let dbpool = pools::db::create_pool(&config.database_url).await?;
    let redispool = pools::redis::create_pool(&config.redis_url).await?;
    let s3uploader = pools::s3::S3Uploader::new(
        &config.region,
        &config.bucket,
        config.s3_endpoint.as_deref(),
    )
    .map_err(error::ServerError::from)?;

    let mut migrator = sqlx::migrate!();
    migrator
        .migrations
        .to_mut()
        .retain(|migration| !migration.description.ends_with(".down"));
    migrator.run(&dbpool).await.unwrap();

    let schema =
        models::schema::build_schema(dbpool.clone(), redispool.clone(), s3uploader.clone()).await?;

    let bg_service_fut = bg_service::run_bg_service(dbpool.clone(), redispool.clone());

    let webserver_fut = HttpServer::new(move || {
        App::new()
            .data(schema.clone())
            .data(dbpool.clone())
            .data(redispool.clone())
            .data(s3uploader.clone())
            .configure(routes::routes)
    })
    .bind(&format!("0.0.0.0:{}", config.port))
    .map_err(error::ServerError::from)?
    .run()
    .map_err(error::ServerError::from)
    .map_err(error::Error::from);

    try_join!(bg_service_fut, webserver_fut)?;

    Ok(())
}
