use crate::error::ServerError;
pub type DbPool = sqlx::postgres::PgPool;
pub type DbPoolOptions = sqlx::postgres::PgPoolOptions;

pub async fn create_pool(url: &str) -> Result<DbPool, ServerError> {
    Ok(DbPoolOptions::new().connect(&url).await?)
}
