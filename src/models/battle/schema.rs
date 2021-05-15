use super::service;
use crate::error;
use crate::models::{card::Card, user::User};
use crate::pools::{db::DbPool, redis::RedisPool};
use crate::session::{ClientInfo, Session};
use crate::utils::datetime::DateTime;

use async_graphql::{
    //connection::{Connection, Edge, EmptyFields},
    //validators::IntRange,
    Context,
    Enum,
    Error as GraphqlError,
    ErrorExtensions,
    InputObject,
    Object,
    SimpleObject,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(sqlx::Type, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Enum)]
#[sqlx(type_name = "battlestate")]
pub enum DuelBattleState {
    #[sqlx(rename = "matching")]
    #[serde(rename = "matching")]
    Matching,
    #[sqlx(rename = "fighting")]
    #[serde(rename = "fighting")]
    Fighting,
    #[sqlx(rename = "finished")]
    #[serde(rename = "finished")]
    Finished,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, SimpleObject)]
pub struct DuelBattle {
    pub id: Uuid,

    pub voteup1: i32,
    pub voteup2: i32,

    pub state: DuelBattleState,

    pub created_at: DateTime,
    pub expired_at: Option<DateTime>,

    pub rating: f64,

    pub card1: Card,
    pub user1: User,

    pub card2: Option<Card>,
    pub user2: Option<User>,
}

#[derive(InputObject)]
pub struct StartDuelBattleInput {
    pub card_id: Uuid,
}

#[derive(SimpleObject)]
pub struct StartDuelBattleOutput {
    pub duel_battle: DuelBattle,
}

#[derive(InputObject)]
pub struct VoteDuelBattleInput {
    pub duel_battle_id: Uuid,
    pub card_idx: u8,
}

#[derive(SimpleObject)]
pub struct VoteDuelBattleOutput {
    pub voteup: i32,
}

#[derive(Default)]
pub struct DuelBattleMutation;

#[Object]
impl DuelBattleMutation {
    async fn start_duel_battle(
        &self,
        ctx: &Context<'_>,
        input: StartDuelBattleInput,
    ) -> Result<StartDuelBattleOutput, GraphqlError> {
        let session: &Session = ctx
            .data_opt::<Session>()
            .ok_or_else(|| error::UserError::NeedLogin.extend())?;
        let StartDuelBattleInput { card_id } = input;
        let dbpool = ctx.data::<DbPool>()?;
        let redispool = ctx.data::<RedisPool>()?;

        let duel_battle =
            service::start_duel_battle(dbpool, redispool, card_id, session.user_id).await?;

        Ok(StartDuelBattleOutput { duel_battle })
    }
    async fn vote_duel_battle(
        &self,
        ctx: &Context<'_>,
        input: VoteDuelBattleInput,
    ) -> Result<VoteDuelBattleOutput, GraphqlError> {
        let VoteDuelBattleInput {
            card_idx,
            duel_battle_id,
        } = input;
        let redispool = ctx.data::<RedisPool>()?;
        let client_info = ctx.data::<ClientInfo>()?;

        let voteup = service::vote_duel_battle(
            redispool,
            duel_battle_id,
            card_idx,
            client_info.remote_addr.as_bytes(),
        )
        .await?;
        Ok(VoteDuelBattleOutput { voteup })
    }
}

#[derive(Default)]
pub struct DuelBattleQuery;

#[Object]
impl DuelBattleQuery {
    async fn duel_battle(&self, ctx: &Context<'_>, id: Uuid) -> Result<DuelBattle, GraphqlError> {
        let dbpool = ctx.data::<DbPool>()?;
        let redispool = ctx.data::<RedisPool>()?;
        Ok(service::get_duel_battle(dbpool, redispool, id).await?)
    }
    async fn sample_duel_battle(&self, ctx: &Context<'_>) -> Result<DuelBattle, GraphqlError> {
        let redispool = ctx.data::<RedisPool>()?;
        Ok(service::sample_duel_battle(redispool).await?)
    }
}
