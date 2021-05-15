use crate::error;
use crate::models::{
    battle::{DuelBattle, DuelBattleState},
    card::Card,
    user::User,
};
use crate::pools::{
    db::DbPool,
    redis::{cmd, RedisPool},
};
use crate::utils::datetime::DateTime;
//use pubsub::PubSub;
//use futures::stream::StreamExt;
use log::{error, warn};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::DUEL_BATTLE_TTL_SECONDS;

use error::ServerError;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("another battle in progress")]
    AnotherBattleInProgress,
    #[error("vote duplicated")]
    VoteDuplicated,
    #[error("no battle available")]
    NoBattleAvailable,
    #[error("server error: {0}")]
    ServerError(#[from] ServerError),
    #[error("battle not found")]
    BattleNotFound,
}

const DUEL_BATTLE_VOTE_QUEUE_KEY: &[u8] = b"q:db";
const DUEL_BATTLE_EVENT_PUBSUB_KEY: &[u8] = b"ps:db:";
const DUEL_BATTLE_STORE_KEY: &[u8] = b"st:db:";
const DUEL_BATTLE_VOTE_HISTORY_KEY: &[u8] = b"ht:db:";

fn duel_battle_store_key(id: Uuid) -> Vec<u8> {
    let mut key = DUEL_BATTLE_STORE_KEY.to_vec();
    key.extend(id.as_bytes());
    key
}
fn timestamp_score() -> f64 {
    chrono::Utc::now().timestamp_nanos() as f64 / 1_000_000f64
}

#[derive(sqlx::FromRow, Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DuelBattlePgRow {
    pub id: Uuid,

    pub voteup1: i32,
    pub voteup2: i32,

    pub state: DuelBattleState,

    pub created_at: DateTime,
    pub expired_at: Option<DateTime>,

    pub rating: f64,

    pub card1: sqlx::types::Json<Card>,
    pub user1: sqlx::types::Json<User>,

    pub card2: Option<sqlx::types::Json<Card>>,
    pub user2: Option<sqlx::types::Json<User>>,
}

impl From<DuelBattlePgRow> for DuelBattle {
    fn from(row: DuelBattlePgRow) -> Self {
        let DuelBattlePgRow {
            id,
            voteup1,
            voteup2,
            state,
            created_at,
            expired_at,
            rating,
            card1,
            user1,
            card2,
            user2,
        } = row;
        Self {
            id,
            voteup1,
            voteup2,
            state,
            created_at,
            expired_at,
            rating,
            card1: card1.0,
            user1: user1.0,
            card2: card2.map(|c| c.0),
            user2: user2.map(|u| u.0),
        }
    }
}

pub async fn get_duel_battle(
    dbpool: &DbPool,
    redispool: &RedisPool,
    duel_battle_id: Uuid,
) -> Result<DuelBattle, Error> {
    let redis = &mut redispool.get().await.map_err(ServerError::from)?;
    let bytes: Option<Vec<u8>> = cmd("HGET")
        .arg(duel_battle_store_key(duel_battle_id))
        .arg(b"duel_battle")
        .query_async(redis)
        .await
        .map_err(ServerError::from)?;
    if let Some(bytes) = bytes {
        Ok(bincode::deserialize(&bytes).map_err(ServerError::from)?)
    } else {
        Ok(sqlx::query_as::<_, DuelBattlePgRow>(
            r#"
            SELECT b.*,
                TO_JSONB(c1) as card1, 
                TO_JSONB(c2) as card2,
                TO_JSONB(u1) as user1, 
                TO_JSONB(u2) as user2
            FROM duel_battles b
            LEFT JOIN cards c1 ON c1.id = b.card_id1
            LEFT JOIN cards c2 ON c2.id = b.card_id2
            LEFT JOIN users u1 ON u1.id = b.user_id1
            LEFT JOIN users u2 ON u2.id = b.user_id2
            WHERE b.id = $1
            "#,
        )
        .bind(duel_battle_id)
        .fetch_optional(dbpool)
        .await
        .map_err(ServerError::from)?
        .ok_or(Error::BattleNotFound)?
        .into())
    }
}

pub async fn start_duel_battle(
    dbpool: &DbPool,
    redispool: &RedisPool,
    card_id: Uuid,
    user_id: Uuid,
) -> Result<DuelBattle, Error> {
    let redis = &mut redispool.get().await.map_err(ServerError::from)?;
    let duel_battle: DuelBattle = sqlx::query_as::<_, DuelBattlePgRow>(r#"
            WITH match AS (
                UPDATE duel_battles b
                SET card_id2 = $1, user_id2 = $2, expired_at = NOW() + $3 * INTERVAL '1' SECOND, state = 'fighting'
                WHERE 
                    b.id IN (
                        SELECT id FROM (
                            (SELECT b.id, c.rating - b.rating AS rating_diff
                            FROM duel_battles b
                            INNER JOIN cards c ON c.id = $1 AND c.owner_id = $2 
                            WHERE c.rating > b.rating AND b.state = 'matching' AND b.user_id1 != $2
                            ORDER BY b.rating DESC LIMIT 1)
                            UNION
                            (SELECT b.id, b.rating - c.rating AS rating_diff
                            FROM duel_battles b
                            INNER JOIN cards c ON c.id = $1 AND c.owner_id = $2
                            WHERE c.rating <= b.rating AND b.state = 'matching' AND b.user_id1 != $2
                            ORDER BY b.rating LIMIT 1)
                        ) x
                        WHERE NOT EXISTS (SELECT 1 FROM duel_battles WHERE state != 'finished' AND (card_id1 = $1 OR card_id2 = $1))
                        ORDER BY rating_diff LIMIT 1
                    )
                RETURNING b.*
            ),
            new_battle AS (
                INSERT INTO duel_battles as b (card_id1, user_id1, rating, state)
                SELECT $1, $2, c.rating, 'matching' 
                FROM cards c WHERE 
                    c.id = $1 AND c.owner_id = $2 AND
                    NOT EXISTS (SELECT 1 FROM match) AND
                    NOT EXISTS (SELECT 1 FROM duel_battles WHERE state != 'finished' AND (card_id1 = $1 OR card_id2 = $1))
                RETURNING b.*
            )
            SELECT l.*,
                TO_JSONB(c1) as card1, 
                TO_JSONB(c2) as card2,
                TO_JSONB(u1) as user1, 
                TO_JSONB(u2) as user2
            FROM ((SELECT * FROM match) UNION (SELECT * FROM new_battle)) l
            LEFT JOIN cards c1 ON c1.id = l.card_id1
            LEFT JOIN cards c2 ON c2.id = l.card_id2
            LEFT JOIN users u1 ON u1.id = l.user_id1
            LEFT JOIN users u2 ON u2.id = l.user_id2
        "#)
        .bind(card_id)
        .bind(user_id)
        .bind(DUEL_BATTLE_TTL_SECONDS)
        .fetch_optional(dbpool)
        .await.map_err(ServerError::from)?.ok_or(Error::AnotherBattleInProgress)?.into();
    //pubsub.create_channel(duel_battle.id, duel_battle.clone());
    if duel_battle.state == DuelBattleState::Fighting {
        cmd("HSET")
            .arg(duel_battle_store_key(duel_battle.id))
            .arg(b"duel_battle")
            .arg(bincode::serialize(&duel_battle).map_err(ServerError::from)?)
            .arg(b"vote1")
            .arg(duel_battle.voteup1)
            .arg(b"vote2")
            .arg(duel_battle.voteup2)
            .execute_async(redis)
            .await
            .map_err(ServerError::from)?;

        cmd("ZADD")
            .arg(DUEL_BATTLE_VOTE_QUEUE_KEY)
            .arg(timestamp_score())
            .arg(duel_battle.id.as_bytes())
            .execute_async(redis)
            .await
            .map_err(ServerError::from)?;
        let mut key = DUEL_BATTLE_EVENT_PUBSUB_KEY.to_vec();
        key.extend(duel_battle.id.as_bytes());
        cmd("PUBLISH")
            .arg(&[
                key.as_slice(),
                bincode::serialize(&DuelBattleEvent::Match {
                    duel_battle: duel_battle.clone(),
                })
                .map_err(ServerError::from)?
                .as_slice(),
            ])
            .execute_async(redis)
            .await
            .map_err(ServerError::from)?;
    }
    Ok(duel_battle)
}

pub async fn finish_duel_battle(
    dbpool: &DbPool,
    redispool: &RedisPool,
    duel_battle_id: Uuid,
) -> Result<(), Error> {
    let redis = &mut redispool.get().await.map_err(ServerError::from)?;
    let key = duel_battle_store_key(duel_battle_id);
    //pubsub.remove_channel(&duel_battle_id);
    let (voteup1, voteup2): (Option<i32>, Option<i32>) = cmd("HMGET")
        .arg(&[key.as_slice(), b"voteup1", b"voteup2"])
        .query_async(redis)
        .await
        .map_err(ServerError::from)?;
    cmd("DEL")
        .arg(key)
        .execute_async(redis)
        .await
        .map_err(ServerError::from)?;
    cmd("ZREM")
        .arg(&[DUEL_BATTLE_VOTE_QUEUE_KEY, duel_battle_id.as_bytes()])
        .execute_async(redis)
        .await
        .map_err(ServerError::from)?;
    match (voteup1, voteup2) {
        (Some(voteup1), Some(voteup2)) => {
            sqlx::query("UPDATE duel_battles (voteup1, voteup2, state) VALUES ($1, $2, 'finished') WHERE id = $3")
                    .bind(voteup1)
                    .bind(voteup2)
                    .bind(duel_battle_id)
                    .execute(dbpool)
                    .await.map_err(ServerError::from)?;
        }
        _ => {
            warn!(
                "duel battle `{}` is finished, but redis store is not exists",
                duel_battle_id
            );
        }
    };
    Ok(())
}

pub async fn sample_duel_battle(redispool: &RedisPool) -> Result<DuelBattle, Error> {
    let redis = &mut redispool.get().await.map_err(ServerError::from)?;
    let bytes: Vec<Vec<u8>> = cmd("ZRANGE")
        .arg(DUEL_BATTLE_VOTE_QUEUE_KEY)
        .arg(0isize)
        .arg(0isize)
        .query_async(redis)
        .await
        .map_err(ServerError::from)?;
    let duel_battle_id =
        Uuid::from_slice(bytes.first().ok_or(Error::NoBattleAvailable)?.as_slice())
            .map_err(ServerError::from)?;
    let bytes: Vec<u8> = cmd("HGET")
        .arg(duel_battle_store_key(duel_battle_id))
        .arg(b"duel_battle")
        .query_async::<Option<Vec<u8>>>(redis)
        .await
        .map_err(ServerError::from)?
        .ok_or(Error::NoBattleAvailable)?;
    let duel_battle = bincode::deserialize(&bytes).map_err(ServerError::from)?;
    Ok(duel_battle)
}

#[derive(Serialize, Deserialize)]
enum DuelBattleEvent {
    Match { duel_battle: DuelBattle },
    Vote { card_idx: u8, voteup: i32 },
}

pub async fn vote_duel_battle(
    redispool: &RedisPool,
    duel_battle_id: Uuid,
    card_idx: u8,
    fingerprint: &[u8],
) -> Result<i32, Error> {
    let redis = &mut redispool.get().await.map_err(ServerError::from)?;
    let mut key = DUEL_BATTLE_VOTE_HISTORY_KEY.to_vec();
    key.extend(duel_battle_id.as_bytes());
    key.extend(fingerprint);
    let history: Option<u8> = cmd("GET")
        .arg(key.as_slice())
        .query_async(redis)
        .await
        .map_err(ServerError::from)?;
    if history.is_some() {
        return Err(Error::VoteDuplicated);
    }
    cmd("SETEX")
        .arg(key)
        .arg(DUEL_BATTLE_TTL_SECONDS)
        .arg(0u8)
        .execute_async(redis)
        .await
        .map_err(ServerError::from)?;
    let mut vote_key = [b'v', b'o', b't', b'e', b'u', b'p', b'1'];
    vote_key[6] = card_idx;
    let mut key = DUEL_BATTLE_STORE_KEY.to_vec();
    key.extend(duel_battle_id.as_bytes());
    let voteup: i32 = cmd("HINCRBY")
        .arg(key.as_slice())
        .arg(&vote_key)
        .arg(1)
        .query_async(redis)
        .await
        .map_err(ServerError::from)?;
    cmd("ZADD")
        .arg(DUEL_BATTLE_VOTE_QUEUE_KEY)
        .arg(timestamp_score())
        .arg(duel_battle_id.as_bytes())
        .execute_async(redis)
        .await
        .map_err(ServerError::from)?;
    let mut key = DUEL_BATTLE_EVENT_PUBSUB_KEY.to_vec();
    key.extend(duel_battle_id.as_bytes());
    /*cmd("PUBLISH")
    .arg(&[
        key.as_slice(),
        bincode::serialize(&DuelBattleEvent::Vote {
            card_idx,
            voteup,
        }).map_err(ServerError::from)?.as_slice(),
    ])
    .execute_async(redis)
    .await.map_err(ServerError::from)?;*/
    Ok(voteup)
}

#[derive(sqlx::FromRow)]
struct Id {
    id: Uuid,
}
pub async fn expire_duel_battles(dbpool: &DbPool, redispool: &RedisPool) -> Result<i32, Error> {
    let expired_duel_battles: Vec<Uuid> = sqlx::query_as::<_, Id>(
        r#"
        SELECT id
        FROM duel_battles b
        WHERE expired_at >= NOW()
        ORDER BY expired_at
        LIMIT 100
        "#,
    )
    .fetch_all(dbpool)
    .await
    .map_err(ServerError::from)?
    .into_iter()
    .map(|id| id.id)
    .collect();
    let mut count = 0;
    for id in expired_duel_battles {
        finish_duel_battle(dbpool, redispool, id).await?;
        count += 1;
    }
    Ok(count)
}

/*pub fn subscribe_duel_battle(pubsub: &PubSub<Uuid, DuelBattle>, duel_battle_id: Uuid) -> Result<pubsub::WatchStream<DuelBattle>, ServerError> {
    Ok(pubsub.subscribe_stream(&duel_battle_id)?)
}*/

/*pub fn deligate_duel_battle_pubsub_forever(pubsub: PubSub<Uuid, DuelBattle>, redis_url: String) -> std::thread::JoinHandle<Result<(), ServerError>> {
    std::thread::spawn(move || {
        loop {
            let mut redis = create_connection(&redis_url)?;
            let mut redispubsub = redis.as_pubsub();
            let mut key = DUEL_BATTLE_EVENT_PUBSUB_KEY.to_vec();
            key.extend(b"*");
            redispubsub.psubscribe(key)?;
            loop {
                let msg = redispubsub.get_message()?;
                let payload: Vec<u8> = msg.get_payload()?;
                let duel_battle_event: DuelBattleEvent = bincode::deserialize(&payload)?;
                let channel_name: Vec<u8> = msg.get_channel()?;
                let duel_battle_id = Uuid::from_slice(&channel_name[DUEL_BATTLE_EVENT_PUBSUB_KEY.len()..]).unwrap();
                let mut last_duel_battle = pubsub.subscribe(&duel_battle_id)?.borrow().clone();
                match duel_battle_event {
                    DuelBattleEvent::Vote { card_idx, voteup } => match card_idx {
                        1 => {
                            last_duel_battle.voteup1 = voteup;
                            pubsub.publish(&duel_battle_id, last_duel_battle);
                        }
                        2 => {
                            last_duel_battle.voteup2 = voteup;
                            pubsub.publish(&duel_battle_id, last_duel_battle);
                        }
                        _ => {
                            error!("card_idx for duel_battle must be 1 or 2: {}", duel_battle_id);
                        }
                    }
                    DuelBattleEvent::Match { duel_battle } => {
                        pubsub.publish(&duel_battle_id, duel_battle);
                    }
                };
            }
        }
        Ok(())
    })
}*/

#[cfg(test)]
pub mod tests {
    use super::*;
    use super::*;
    use crate::utils::test::*;

    #[actix_rt::test]
    async fn test_duel_battle() {
        let docker = TestDocker::new();
        let d = docker.run().await;
        let user_ids: Vec<_> = (0..4).map(|_| uuid::Uuid::new_v4()).collect();
        let card_ids: Vec<_> = (0..4).map(|_| uuid::Uuid::new_v4()).collect();

        let dbpool = d.pgpool;
        let redispool = d.redispool;

        //let pubsub = pubsub::PubSub::new();

        let mut duel_battles = Vec::new();

        //let h = deligate_duel_battle_pubsub_forever(pubsub.clone(), d.redisurl);

        for i in 0usize..4 {
            sqlx::query("INSERT INTO users (id, nickname, email, password, points) VALUES ($1, 'a', $2, 'c', 200)")
                .bind(user_ids[i])
                .bind(i.to_string())
                .execute(&dbpool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO cards (rating, image_path, name, id, owner_id) VALUES ($1, $2, $3, $4, $5)")
                .bind(i as f32)
                .bind(i.to_string())
                .bind(i.to_string())
                .bind(card_ids[i])
                .bind(user_ids[i])
                .execute(&dbpool)
                .await
                .unwrap();
            duel_battles.push(
                start_duel_battle(&dbpool, &redispool, card_ids[i], user_ids[i])
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(duel_battles[0].id, duel_battles[1].id);
        assert_eq!(duel_battles[0].state, DuelBattleState::Matching);
        assert_eq!(duel_battles[1].state, DuelBattleState::Fighting);
        assert_eq!(duel_battles[2].id, duel_battles[3].id);
        assert_eq!(duel_battles[2].state, DuelBattleState::Matching);
        assert_eq!(duel_battles[3].state, DuelBattleState::Fighting);
        let duel_battle = sample_duel_battle(&redispool).await.unwrap();
        vote_duel_battle(&redispool, duel_battle.id, 1, &[0])
            .await
            .unwrap();
        let err = vote_duel_battle(&redispool, duel_battle.id, 2, &[0])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::VoteDuplicated));
        let duel_battle2 = sample_duel_battle(&redispool).await.unwrap();
        assert_ne!(duel_battle.id, duel_battle2.id);
        /*
        let mut stream = subscribe_duel_battle(&pubsub, duel_battles[0].id).unwrap();
        let duel_battle = stream.next().await.unwrap();
        assert_eq!(duel_battle.state, DuelBattleState::Matching);
        let duel_battle = stream.next().await.unwrap();
        assert_eq!(duel_battle.state, DuelBattleState::Fighting);
        vote_duel_battle(&redispool, duel_battle.id, 1, &[1]).await.unwrap();
        let duel_battle = stream.next().await.unwrap();
        assert_eq!(duel_battle.voteup1, 1);
        assert_eq!(duel_battle.voteup2, 0);
        vote_duel_battle(&redispool, duel_battle.id, 2, &[0]).await.unwrap();
        let duel_battle = stream.next().await.unwrap();
        assert_eq!(duel_battle.voteup1, 1);
        assert_eq!(duel_battle.voteup2, 1);
        let err = vote_duel_battle(&redispool, duel_battle.id, 1, &[0]).await.unwrap_err();
        assert!(matches!(err, Error::VoteDuplicated));
        */
        let duel_battle1 = get_duel_battle(&dbpool, &redispool, duel_battles[0].id)
            .await
            .unwrap();
        finish_duel_battle(&dbpool, &redispool, duel_battle.id).await;
        finish_duel_battle(&dbpool, &redispool, duel_battle2.id).await;
        let err = sample_duel_battle(&redispool).await.unwrap_err();
        assert!(matches!(err, Error::NoBattleAvailable));
        let duel_battle2 = get_duel_battle(&dbpool, &redispool, duel_battles[0].id)
            .await
            .unwrap();
        assert_eq!(duel_battle1, duel_battle2);
    }
}
