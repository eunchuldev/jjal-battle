use super::battle::{DuelBattleMutation, DuelBattleQuery};
use super::card::CardMutation;
use super::user::{UserMutation, UserQuery};
use crate::error::ServerError;
use crate::pools::{db::DbPool, redis::RedisPool, s3::S3Uploader};
use async_graphql::{EmptySubscription, MergedObject, Schema as GraphqlSchema};

#[derive(MergedObject, Default)]
pub struct Query(UserQuery, DuelBattleQuery);

#[derive(MergedObject, Default)]
pub struct Mutation(UserMutation, CardMutation, DuelBattleMutation);

pub type Schema = GraphqlSchema<Query, Mutation, EmptySubscription>;

pub async fn build_schema(
    dbpool: DbPool,
    redispool: RedisPool,
    s3uploader: S3Uploader,
) -> Result<Schema, ServerError> {
    Ok(
        GraphqlSchema::build(Query::default(), Mutation::default(), EmptySubscription)
            .data(dbpool)
            .data(redispool)
            .data(s3uploader)
            .finish(),
    )
}
