use crate::models::battle::service::expire_duel_battles;
use crate::pools::{db::DbPool, redis::RedisPool};

pub async fn run_bg_service(
    dbpool: DbPool,
    redispool: RedisPool,
) -> Result<(), crate::error::Error> {
    loop {
        expire_duel_battles(&dbpool, &redispool).await?;
        actix_web::rt::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
