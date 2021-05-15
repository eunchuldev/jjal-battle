use crate::error;
use crate::session::{create_session, Session};
use crate::utils::s3::S3Uploader;
use crate::utils::hash::{hash_password, verify_password, rand_string};
//use crate::util::{hash_password, verify_password, create_jwt_token, create_jwt_token};


use async_graphql::{
    connection::{Connection, CursorType, Edge, EmptyFields},
    Context, EmptySubscription, Enum, Error as GraphqlError, Object, Schema as GraphqlSchema,
    SimpleObject, Upload,
    validators::IntRange,
};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use lazy_static::lazy_static;
pub use deadpool_redis::{Config as RedisConfig, Pool as RedisPool};

pub use error::Error;

type DateTime = chrono::DateTime<Utc>;
pub type DbPool = sqlx::postgres::PgPool;
pub type DbPoolOptions = sqlx::postgres::PgPoolOptions;

lazy_static! {
    static ref MAX_DATETIME: DateTime = Utc.ymd(9999, 1, 1).and_hms(0, 1, 1);
    static ref MIN_DATETIME: DateTime = Utc.ymd(0, 1, 1).and_hms(0, 1, 1);
}
const MAX_RATING: f64 = 999999999.0;
const MIN_RATING: f64 = -999999999.0;
const MAX_PAGE_SIZE: i32 = 100;

pub fn create_redispool(url: &str) -> Result<RedisPool, Error> {
    Ok(RedisConfig {
        url: Some(url.to_string()),
        pool: None,
    }
    .create_pool()?)
}

#[derive(sqlx::FromRow, Clone, Debug, Deserialize, Serialize, PartialEq, SimpleObject)]
pub struct Card {
    pub id: Uuid,
    pub rating: f64,
    pub owned_at: DateTime,
    pub created_at: DateTime,
    pub owner_id: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum CardCursor {
    OwnedAt(DateTime),
    Rating(f64),
}
impl CursorType for CardCursor {
    type Error = error::Error;
    fn decode_cursor(s: &str) -> Result<Self, Self::Error> {
        Ok(bincode::deserialize(&base64::decode(s)?)?)
    }
    fn encode_cursor(&self) -> String {
        base64::encode(bincode::serialize(&self).unwrap())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Copy, Enum)]
pub enum CardSort {
    OwnedAt,
    Rating
}

#[derive(sqlx::Type, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Enum)]
#[sqlx(type_name = "userkind")]
pub enum UserKind {
    #[sqlx(rename = "super")]
    Super,
    #[sqlx(rename = "normal")]
    Normal,
}

#[derive(sqlx::FromRow, Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct User {
    pub id: Uuid,
    #[serde(skip)]
    pub password: String,

    pub kind: UserKind,
    pub email: String,
    pub nickname: String,
    pub created_at: DateTime,
}

#[Object]
impl User {
    async fn id(&self) -> Uuid {
        self.id
    }
    async fn kind(&self) -> UserKind {
        self.kind
    }
    async fn nickname(&self) -> &str {
        &self.nickname
    }
    async fn created_at(&self) -> &DateTime {
        &self.created_at
    }
    /// Email addr. Not fetchable by other users.
    async fn email(&self, ctx: &Context<'_>) -> Result<&str, GraphqlError> {
        let session: &Session = ctx
            .data_opt::<Session>()
            .ok_or_else(|| GraphqlError::from(Error::NotAuthorized))?;
        if session.user_id == self.id || session.user_kind == UserKind::Super {
            Ok(&self.email)
        } else {
            Err(GraphqlError::from(Error::NotAuthorized))
        }
    }
    /// Cards owned by the user.
    async fn cards(
        &self,
        ctx: &Context<'_>,
        sort: Option<CardSort>,
        after: Option<String>,
        before: Option<String>,
        #[graphql(validator(IntRange(min = "0", max = "100")))]
        first: Option<i32>,
        #[graphql(desc = "last N items. clamped by [0-100]")] last: Option<i32>,
    ) -> Result<Connection<CardCursor, Card, EmptyFields, EmptyFields>, GraphqlError> {
        let dbpool = ctx.data::<DbPool>()?;
        if first.is_some() && last.is_some() {
            return Err(Error::BadRequest("cards", "first or last, not both").into());
        }
        let first = if first.is_none() && last.is_none() {
            Some(MAX_PAGE_SIZE)
        } else {
            first
        };
        let first = first.map(|l| l.min(MAX_PAGE_SIZE).max(0));
        let last = last.map(|l| l.min(MAX_PAGE_SIZE).max(0));
        let sort = sort.unwrap_or(CardSort::OwnedAt);
        async_graphql::connection::query(after, before, first, last, |after, before, first, last| async move {
            let (after, before) = match sort {
                CardSort::OwnedAt => (after.unwrap_or(CardCursor::OwnedAt(MIN_DATETIME.clone())), before.unwrap_or(CardCursor::OwnedAt(MAX_DATETIME.clone()))),
                CardSort::Rating => (after.unwrap_or(CardCursor::Rating(MIN_RATING)), before.unwrap_or(CardCursor::Rating(MAX_RATING))),
            };
            let (sql_sorting, limit) = match (first, last) {
                (Some(limit), None) => ("ASC", limit as i32),
                (None, Some(limit)) => ("DESC", limit as i32),
                _ => ("ASC", MAX_PAGE_SIZE),
            };
            let mut cards = match (sort, after, before) {
                (CardSort::OwnedAt, CardCursor::OwnedAt(after), CardCursor::OwnedAt(before)) => {
                    sqlx::query_as::<_, Card>(&format!("SELECT * FROM cards WHERE owner_id = $1 AND owned_at > $2 AND owned_at < $3 ORDER BY owned_at {} LIMIT $4 + 1", sql_sorting))
                        .bind(self.id)
                        .bind(after)
                        .bind(before)
                        .bind(limit)
                        .fetch_all(dbpool)
                        .await?
                }
                (CardSort::Rating, CardCursor::Rating(after), CardCursor::Rating(before)) => {
                    sqlx::query_as::<_, Card>(&format!("SELECT * FROM cards WHERE owner_id = $1 AND rating > $2 AND rating < $3 ORDER BY rating {} LIMIT $4 + 1", sql_sorting))
                        .bind(self.id)
                        .bind(after)
                        .bind(before)
                        .bind(limit)
                        .fetch_all(dbpool)
                        .await?
                }
                _ => {
                    return Err(Error::BadRequest("cards", "sort format and cursor type not match").into());
                }
            };
            let mut connection = Connection::new(
                last.filter(|limit| limit < &cards.len()).is_some(),
                first.filter(|limit| limit < &cards.len()).is_some(),
                );
            cards.truncate(last.or(first).unwrap_or(MAX_PAGE_SIZE as usize));
            if sql_sorting == "DESC" {
                cards.reverse();
            }
            
            match sort {
                CardSort::OwnedAt => {
                    connection.append(
                        cards.into_iter().map(|card| Edge::new(CardCursor::OwnedAt(card.owned_at), card))
                    );
                }
                CardSort::Rating => {
                    connection.append(
                        cards.into_iter().map(|card| Edge::new(CardCursor::Rating(card.rating), card))
                    );
                }
            };
            Ok(connection)
        }).await
    }
}

pub struct Mutation;

#[Object]
impl Mutation {
    async fn register(
        &self,
        ctx: &Context<'_>,
        email: String,
        password: String,
        nickname: String,
    ) -> Result<Uuid, GraphqlError> {
        let dbpool = ctx.data::<DbPool>()?;
        let user = sqlx::query_as::<_, User>(
            "INSERT INTO users (email, password, nickname) VALUES ($1, $2, $3) RETURNING id, password, kind, email, nickname, created_at")
            .bind(email)
            .bind(hash_password(password)?)
            .bind(nickname)
            .fetch_one(dbpool)
            .await?;
        create_session(&ctx, &user).await?;
        Ok(user.id)
    }
    async fn login(
        &self,
        ctx: &Context<'_>,
        email: String,
        password: String,
    ) -> Result<Uuid, GraphqlError> {
        let dbpool = ctx.data::<DbPool>()?;
        //let redis = ctx.data::<RedisPool>()?.get().await?;
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_one(dbpool)
            .await?;
        if !verify_password(&password, &user.password)? {
            Err(GraphqlError::from(Error::WrongPassword))
        } else {
            create_session(&ctx, &user).await?;
            Ok(user.id)
        }
    }

    async fn upload_card(&self, ctx: &Context<'_>, file: Upload) -> Result<String, GraphqlError> {
        let s3_uploader = ctx.data::<S3Uploader>()?;
        let upload = file.value(ctx)?;
        let size = upload.size().ok().map(|u| u as i64);
        let mime_type = upload.content_type;
        let file = upload.content;
        let f = s3_uploader.put_object(&rand_string(30), file, size, mime_type).await?;
        Ok(f)
        /*let info = FileInfo {
            id: entry.key().into(),
            filename: upload.filename.clone(),
            mimetype: upload.content_type,
        };*/
    }

    /*async fn start_battle(
        &self,
        ctx: &Context<'_>,
        card_id: Uuid,
    ) -> Result<Uuid, GraphqlError> {
        let card = sqlx::query_as<_, bool>("SELECT * FROM cards WHERE id = $1 AND owner_id = $2")
    }*/
}

pub struct Query;

#[Object]
impl Query {
    async fn api_version(&self) -> String {
        "0.1".to_string()
    }
    async fn user(&self, ctx: &Context<'_>, id: Uuid) -> Result<User, GraphqlError> {
        let dbpool = ctx.data::<DbPool>()?;
        Ok(
            sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
                .bind(id)
                .fetch_one(dbpool)
                .await?,
        )
    }
}

pub type Schema = GraphqlSchema<Query, Mutation, EmptySubscription>;

pub async fn build_schema(dbpool: DbPool, redispool: RedisPool) -> Result<Schema, Error> {
    Ok(GraphqlSchema::build(Query, Mutation, EmptySubscription)
        .data(dbpool)
        .data(redispool)
        .finish())
}

#[cfg(test)]
pub mod tests {
    use crate::utils::test::*;
    use async_graphql::{value, Name, Value};

    #[actix_rt::test]
    async fn test_migration_and_build_schema() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let _schema = db.schema.clone();
    }
    #[actix_rt::test]
    async fn test_register() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let schema = db.schema.clone();

        let query = r#"mutation { register(email:"a", password:"b", nickname:"c") }"#;
        let res = schema.execute(query).await;
        let data = res.data;
        assert_eq!(res.errors, Vec::new(),);

        let user_id = match data {
            Value::Object(v) => match v.get(&Name::new("register")) {
                Some(Value::String(id)) => id.clone(),
                _ => panic!("unexpected value type"),
            },
            _ => panic!("unexpected value type"),
        };

        let query = format!(r#"query {{ user(id:"{}") {{ nickname }} }}"#, user_id);
        let res = schema.execute(&query).await;
        let data = res.data;
        assert_eq!(res.errors, Vec::new(),);
        assert_eq!(
            data,
            value!( {
                "user": {
                    "nickname": "c"
                }
            } )
        );
    }

    #[actix_rt::test]
    async fn test_login() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let schema = db.schema.clone();

        let query = r#"mutation { register(email:"a", password:"b", nickname:"c") }"#;
        let res = schema.execute(query).await;
        let data = res.data;

        let user_id = match data {
            Value::Object(v) => match v.get(&Name::new("register")) {
                Some(Value::String(id)) => id.clone(),
                _ => panic!("unexpected value type"),
            },
            _ => panic!("unexpected value type"),
        };

        let query = r#"mutation { login(email:"a", password:"b") }"#;
        let res = schema.execute(query).await;
        let data = res.data;
        assert_eq!(res.errors, Vec::new(),);
        let user_id2 = match data {
            Value::Object(v) => match v.get(&Name::new("login")) {
                Some(Value::String(id)) => id.clone(),
                _ => panic!("unexpected value type"),
            },
            _ => panic!("unexpected value type"),
        };

        assert_eq!(user_id, user_id2);

        let query = r#"mutation { login(email:"a", password:"c") }"#;
        let res = schema.execute(query).await;
        assert_eq!(
            res.errors
                .into_iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
            vec!["wrong password"]
        );
    }

    #[actix_rt::test]
    async fn test_cards_pagination_validation() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let schema = db.schema.clone();
        let user_id = uuid::Uuid::new_v4();
        let query = format!(r#"query {{ 
            user(id: "{}") {{ 
                cards(first: 101) {{
                    edges {{ node {{ ownedAt rating }} }} 
                    pageInfo {{ endCursor hasNextPage hasPreviousPage }}
                }}
            }} 
        }}"#, user_id);
        let res = schema.execute(query).await;
        assert_eq!(
            res.errors
                .into_iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
            vec![r#"Invalid value for argument "first", the value is 101, must be between 0 and 100"#]);

    }
    #[actix_rt::test]
    async fn test_cards_pagination() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let schema = db.schema.clone();
        let user_id = uuid::Uuid::new_v4();
        let dbpool = db.pgpool;
        sqlx::query("INSERT INTO users (id, nickname, email, password) VALUES ($1, 'a', 'b', 'c')")
            .bind(user_id)
            .execute(&dbpool)
            .await.unwrap();
        let date = chrono::DateTime::parse_from_str("2020 Apr 13 12:09:14.274 +0000", "%Y %b %d %H:%M:%S%.3f %z").unwrap();
        for i in 0..10 {
            sqlx::query("INSERT INTO cards (rating, owned_at, owner_id) VALUES ($1, $2, $3)")
                .bind(i as f32)
                .bind(date - chrono::Duration::seconds(i))
                .bind(user_id)
                .execute(&dbpool)
                .await.unwrap();
        }
        let query = format!(r#"query {{ 
            user(id: "{}") {{ 
                cards(first: 2) {{
                    edges {{ node {{ ownedAt rating }} }} 
                    pageInfo {{ endCursor hasNextPage hasPreviousPage }}
                }}
            }} 
        }}"#, user_id);
        let res = schema.execute(query).await;
        let data = res.data;
        assert_eq!(res.errors, Vec::new());
        assert_eq!(
            data,
            value!( {
                "user": {
                    "cards": {
                        "edges": [{
                            "node": { "ownedAt": "2020-04-13T12:09:05.274+00:00", "rating": 9.0 }, 
                        }, {
                            "node": { "ownedAt": "2020-04-13T12:09:06.274+00:00", "rating": 8.0 }
                        }],
                        "pageInfo": {
                            "endCursor": "AAAAABgAAAAAAAAAMjAyMC0wNC0xM1QxMjowOTowNi4yNzRa",
                            "hasNextPage": true,
                            "hasPreviousPage": false,
                        }
                    }
            }} )
        );
        let mut data = data;
        for _ in 0..4 {
            let json = data.into_json().unwrap();
            let last_cursor = json["user"]["cards"]["pageInfo"]["endCursor"].as_str().unwrap().to_string();
            assert_eq!(
                json["user"]["cards"]["pageInfo"]["hasNextPage"].as_bool().unwrap(),
                true
                );
            let query = format!(r#"query {{ 
                user(id: "{}") {{ 
                    cards(first: 2, after: "{}") {{
                        pageInfo {{ endCursor hasNextPage hasPreviousPage }}
                    }}
                }} 
            }}"#, user_id, last_cursor);
            let res = schema.execute(query).await;
            data = res.data;
        }
        assert_eq!(
            data.into_json().unwrap()["user"]["cards"]["pageInfo"]["hasNextPage"].as_bool().unwrap(),
            false
            );
        let query = format!(r#"query {{ 
            user(id: "{}") {{ 
                cards(last: 2, sort: RATING) {{
                    edges {{ node {{ rating }} }} 
                    pageInfo {{ startCursor hasNextPage hasPreviousPage }}
                }}
            }} 
        }}"#, user_id);
        let res = schema.execute(query).await;
        assert_eq!(res.errors, Vec::new());
        let mut data = res.data;
        for i in 0..4 {
            let json = data.into_json().unwrap();
            assert_eq!(json["user"]["cards"]["pageInfo"]["hasNextPage"].as_bool(), Some(false));
            assert_eq!(json["user"]["cards"]["pageInfo"]["hasPreviousPage"].as_bool(), Some(true));
            assert_eq!(json["user"]["cards"]["edges"][0]["node"]["rating"].as_f64(), Some(8.0 - (i*2) as f64));
            let last_cursor = json["user"]["cards"]["pageInfo"]["startCursor"].as_str().unwrap().to_string();
            let query = format!(r#"query {{ 
                user(id: "{}") {{ 
                    cards(last: 2, before: "{}", sort: RATING) {{
                        edges {{ node {{ rating }} }}
                        pageInfo {{ startCursor hasNextPage hasPreviousPage }}
                    }}
                }} 
            }}"#, user_id, last_cursor);
            let res = schema.execute(query).await;
            assert_eq!(res.errors, Vec::new());
            data = res.data;
        }
        assert_eq!(
            data.into_json().unwrap()["user"]["cards"]["pageInfo"]["hasPreviousPage"].as_bool(),
            Some(false)
            );
    }
}
