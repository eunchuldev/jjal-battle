use super::card::{Card, CardCursor, CardSort};
use crate::error;
use crate::pools::db::DbPool;
use crate::session::{create_session, Session};
use crate::utils::{
    datetime::DateTime,
    hash::{hash_password, verify_password},
};

use async_graphql::{
    connection::{Connection, Edge, EmptyFields},
    validators::IntRange,
    Context, Enum, Error as GraphqlError, ErrorExtensions, InputObject, Object, SimpleObject,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::{MAX_DATETIME, MIN_DATETIME, USER_CARDS_MAX_PAGE_SIZE};

pub use error::{ServerError, UserError};

#[derive(sqlx::Type, Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Enum)]
#[sqlx(type_name = "userkind")]
pub enum UserKind {
    #[sqlx(rename = "super")]
    #[serde(rename = "super")]
    Super,
    #[sqlx(rename = "normal")]
    #[serde(rename = "normal")]
    Normal,
}

#[derive(sqlx::FromRow, Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct User {
    pub id: Uuid,
    #[serde(skip)]
    pub password: String,

    pub kind: UserKind,

    pub points: i32,

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
    async fn points(&self) -> i32 {
        self.points
    }
    /// Email addr. Not fetchable by other users.
    async fn email(&self, ctx: &Context<'_>) -> Result<&str, GraphqlError> {
        let session: &Session = ctx
            .data_opt::<Session>()
            .ok_or_else(|| UserError::NotAuthorized.extend())?;
        if session.user_id == self.id || session.user_kind == UserKind::Super {
            Ok(&self.email)
        } else {
            Err(UserError::NotAuthorized.extend())
        }
    }
    /// Cards owned by the user.
    async fn cards(
        &self,
        ctx: &Context<'_>,
        sort: Option<CardSort>,
        after: Option<String>,
        before: Option<String>,
        #[graphql(validator(IntRange(min = "0", max = "100")))] first: Option<i32>,
        #[graphql(validator(IntRange(min = "0", max = "100")))] last: Option<i32>,
    ) -> Result<Connection<CardCursor, Card, EmptyFields, EmptyFields>, GraphqlError> {
        let dbpool = ctx.data::<DbPool>()?;
        if first.is_some() && last.is_some() {
            return Err(UserError::BadRequest("first or last, not both").extend());
        }
        let first = if first.is_none() && last.is_none() {
            Some(USER_CARDS_MAX_PAGE_SIZE)
        } else {
            first
        };
        let first = first.map(|l| l.min(USER_CARDS_MAX_PAGE_SIZE).max(0));
        let last = last.map(|l| l.min(USER_CARDS_MAX_PAGE_SIZE).max(0));
        let sort = sort.unwrap_or(CardSort::OwnedAt);
        async_graphql::connection::query(after, before, first, last, |after, before, first, last| async move {
            let (after, before) = match sort {
                CardSort::OwnedAt => (after.unwrap_or(CardCursor::OwnedAt(Some(*MIN_DATETIME))), before.unwrap_or_else(|| CardCursor::OwnedAt(Some(*MAX_DATETIME)))),
                CardSort::Rating => (after.unwrap_or(CardCursor::Rating(f64::MIN)), before.unwrap_or(CardCursor::Rating(f64::MAX))),
                CardSort::CreatedAt => (after.unwrap_or(CardCursor::CreatedAt(*MIN_DATETIME)), before.unwrap_or(CardCursor::CreatedAt(*MAX_DATETIME))),
            };
            let (sql_sorting, limit) = match (first, last) {
                (Some(limit), None) => ("ASC", limit as i32),
                (None, Some(limit)) => ("DESC", limit as i32),
                _ => ("ASC", USER_CARDS_MAX_PAGE_SIZE),
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
                (CardSort::CreatedAt, CardCursor::CreatedAt(after), CardCursor::CreatedAt(before)) => {
                    sqlx::query_as::<_, Card>(&format!("SELECT * FROM cards WHERE owner_id = $1 AND created_at > $2 AND created_at < $3 ORDER BY created_at {} LIMIT $4 + 1", sql_sorting))
                        .bind(self.id)
                        .bind(after)
                        .bind(before)
                        .bind(limit)
                        .fetch_all(dbpool)
                        .await?
                }
                _ => {
                    return Err(UserError::BadRequest("sort format and cursor type not match").extend());
                }
            };
            let mut connection = Connection::new(
                last.filter(|limit| limit < &cards.len()).is_some(),
                first.filter(|limit| limit < &cards.len()).is_some(),
                );
            cards.truncate(last.or(first).unwrap_or(USER_CARDS_MAX_PAGE_SIZE as usize));
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
                CardSort::CreatedAt => {
                    connection.append(
                        cards.into_iter().map(|card| Edge::new(CardCursor::CreatedAt(card.created_at), card))
                    );
                }
            };
            Ok(connection)
        }).await
    }
}

#[derive(InputObject)]
pub struct RegisterInput {
    pub email: String,
    pub password: String,
    pub nickname: String,
}

#[derive(SimpleObject, Deserialize)]
pub struct RegisterOutput {
    pub user: User,
}

#[derive(InputObject)]
pub struct LoginInput {
    pub email: String,
    pub password: String,
}

#[derive(SimpleObject, Deserialize)]
pub struct LoginOutput {
    pub user: User,
}

#[derive(Default)]
pub struct UserMutation;

#[Object]
impl UserMutation {
    async fn register(
        &self,
        ctx: &Context<'_>,
        input: RegisterInput,
    ) -> Result<RegisterOutput, GraphqlError> {
        let RegisterInput {
            email,
            password,
            nickname,
        } = input;
        let dbpool = ctx.data::<DbPool>()?;
        let user = sqlx::query_as::<_, User>(
            "INSERT INTO users (email, password, nickname) VALUES ($1, $2, $3) RETURNING *",
        )
        .bind(email)
        .bind(hash_password(password)?)
        .bind(nickname)
        .fetch_one(dbpool)
        .await?;
        create_session(&ctx, &user).await?;
        Ok(RegisterOutput { user })
    }
    async fn login(
        &self,
        ctx: &Context<'_>,
        input: LoginInput,
    ) -> Result<LoginOutput, GraphqlError> {
        let LoginInput { email, password } = input;
        let dbpool = ctx.data::<DbPool>()?;
        //let redis = ctx.data::<RedisPool>()?.get().await?;
        let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_one(dbpool)
            .await?;
        if !verify_password(&password, &user.password)? {
            Err(UserError::WrongPassword.extend())
        } else {
            create_session(&ctx, &user).await?;
            Ok(LoginOutput { user })
        }
    }
    /*async fn start_battle(
        &self,
        ctx: &Context<'_>,
        card_id: Uuid,
    ) -> Result<Uuid, GraphqlError> {
        let card = sqlx::query_as<_, bool>("SELECT * FROM cards WHERE id = $1 AND owner_id = $2")
    }*/
}

#[derive(Default)]
pub struct UserQuery;

#[Object]
impl UserQuery {
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

#[cfg(test)]
pub mod tests {
    use crate::utils::test::*;
    use async_graphql::value;

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

        let query = r#"mutation { register(input: {email:"a", password:"b", nickname:"c"}) { user { id } } }"#;
        let res = schema.execute(query).await;
        let data = res.data;
        assert_eq!(res.errors, Vec::new(),);

        let res: serde_json::Value = async_graphql::from_value(data).unwrap();
        let user_id = res
            .get("register")
            .unwrap()
            .get("user")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap();

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

        let query = r#"mutation { register(input: {email:"a", password:"b", nickname:"c"}) { user { id } } }"#;
        let res = schema.execute(query).await;
        let data = res.data;

        let res: serde_json::Value = async_graphql::from_value(data).unwrap();
        let user_id = res
            .get("register")
            .unwrap()
            .get("user")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap();

        let query = r#"mutation { login(input: {email:"a", password:"b"}) { user { id } } }"#;
        let res = schema.execute(query).await;
        let data = res.data;
        assert_eq!(res.errors, Vec::new(),);

        let res: serde_json::Value = async_graphql::from_value(data).unwrap();
        let user_id2 = res
            .get("login")
            .unwrap()
            .get("user")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap();

        assert_eq!(user_id, user_id2);

        let query = r#"mutation { login(input: {email:"a", password:"c"}) { user { id } } }"#;
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
        let query = format!(
            r#"query {{ 
            user(id: "{}") {{ 
                cards(first: 101) {{
                    edges {{ node {{ ownedAt rating }} }} 
                    pageInfo {{ endCursor hasNextPage hasPreviousPage }}
                }}
            }} 
        }}"#,
            user_id
        );
        let res = schema.execute(query).await;
        assert_eq!(
            res.errors
                .into_iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>(),
            vec![
                r#"Invalid value for argument "first", the value is 101, must be between 0 and 100"#
            ]
        );
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
            .await
            .unwrap();
        let date = chrono::DateTime::parse_from_str(
            "2020 Apr 13 12:09:14.274 +0000",
            "%Y %b %d %H:%M:%S%.3f %z",
        )
        .unwrap();
        for i in 0..10 {
            sqlx::query("INSERT INTO cards (rating, owned_at, owner_id, image_path, name) VALUES ($1, $2, $3, $4, $5)")
                .bind(i as f32)
                .bind(date - chrono::Duration::seconds(i))
                .bind(user_id)
                .bind("123")
                .bind("123")
                .execute(&dbpool)
                .await
                .unwrap();
        }
        let query = format!(
            r#"query {{ 
            user(id: "{}") {{ 
                cards(first: 2) {{
                    edges {{ node {{ ownedAt rating }} }} 
                    pageInfo {{ endCursor hasNextPage hasPreviousPage }}
                }}
            }} 
        }}"#,
            user_id
        );
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
                            "endCursor": "AAAAAAEYAAAAAAAAADIwMjAtMDQtMTNUMTI6MDk6MDYuMjc0Wg==",
                            "hasNextPage": true,
                            "hasPreviousPage": false,
                        }
                    }
            }} )
        );
        let mut data = data;
        for _ in 0i32..4 {
            let json = data.into_json().unwrap();
            let last_cursor = json["user"]["cards"]["pageInfo"]["endCursor"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(
                json["user"]["cards"]["pageInfo"]["hasNextPage"]
                    .as_bool()
                    .unwrap(),
                true
            );
            let query = format!(
                r#"query {{ 
                user(id: "{}") {{ 
                    cards(first: 2, after: "{}") {{
                        pageInfo {{ endCursor hasNextPage hasPreviousPage }}
                    }}
                }} 
            }}"#,
                user_id, last_cursor
            );
            let res = schema.execute(query).await;
            data = res.data;
        }
        assert_eq!(
            data.into_json().unwrap()["user"]["cards"]["pageInfo"]["hasNextPage"]
                .as_bool()
                .unwrap(),
            false
        );
        let query = format!(
            r#"query {{ 
            user(id: "{}") {{ 
                cards(last: 2, sort: RATING) {{
                    edges {{ node {{ rating }} }} 
                    pageInfo {{ startCursor hasNextPage hasPreviousPage }}
                }}
            }} 
        }}"#,
            user_id
        );
        let res = schema.execute(query).await;
        assert_eq!(res.errors, Vec::new());
        let mut data = res.data;
        for i in 0i32..4 {
            let json = data.into_json().unwrap();
            assert_eq!(
                json["user"]["cards"]["pageInfo"]["hasNextPage"].as_bool(),
                Some(false)
            );
            assert_eq!(
                json["user"]["cards"]["pageInfo"]["hasPreviousPage"].as_bool(),
                Some(true)
            );
            assert_eq!(
                json["user"]["cards"]["edges"][0]["node"]["rating"].as_f64(),
                Some(8.0 - (i * 2) as f64)
            );
            let last_cursor = json["user"]["cards"]["pageInfo"]["startCursor"]
                .as_str()
                .unwrap()
                .to_string();
            let query = format!(
                r#"query {{ 
                user(id: "{}") {{ 
                    cards(last: 2, before: "{}", sort: RATING) {{
                        edges {{ node {{ rating }} }}
                        pageInfo {{ startCursor hasNextPage hasPreviousPage }}
                    }}
                }} 
            }}"#,
                user_id, last_cursor
            );
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
