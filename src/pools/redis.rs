use crate::error::ServerError;
pub use deadpool_redis::{
    cmd, Config as RedisConfig, ConnectionWrapper as RedisConn, Pool as RedisPool,
};
pub async fn create_pool(url: &str) -> Result<RedisPool, ServerError> {
    Ok(RedisConfig {
        url: Some(url.to_string()),
        pool: None,
    }
    .create_pool()?)
}

/*pub fn create_connection(url: &str) -> Result<redis::Connection, ServerError> {
    Ok(redis::Client::open(url)?.get_connection()?)
}*/
