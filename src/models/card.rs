use super::user::User;
use crate::constants::GACHA_POINTS;
use crate::error;
use crate::pools::{db::DbPool, s3::S3Uploader};
use crate::session::Session;
use crate::utils::{datetime::DateTime, hash::rand_string};
use async_graphql::{
    connection::CursorType, Context, Enum, Error as GraphqlError, ErrorExtensions, InputObject,
    Object, SimpleObject, Upload,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use error::{ServerError, UserError};

#[derive(sqlx::FromRow, Clone, Debug, Deserialize, Serialize, PartialEq, SimpleObject)]
pub struct Card {
    pub id: Uuid,
    pub rating: f64,
    pub created_at: DateTime,
    pub owned_at: Option<DateTime>,
    pub owner_id: Option<Uuid>,
    pub creator_id: Option<Uuid>,
    pub image_path: String,
    pub name: String,
    //pub match_queued_at: Option<DateTime>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub enum CardCursor {
    OwnedAt(Option<DateTime>),
    Rating(f64),
    CreatedAt(DateTime),
}
impl CursorType for CardCursor {
    type Error = ServerError;
    fn decode_cursor(s: &str) -> Result<Self, Self::Error> {
        Ok(bincode::deserialize(&base64::decode(s)?)?)
    }
    fn encode_cursor(&self) -> String {
        base64::encode(bincode::serialize(&self).unwrap())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Copy, Enum)]
pub enum CardSort {
    CreatedAt,
    OwnedAt,
    Rating,
}

#[derive(InputObject)]
pub struct CreateCardInput {
    pub name: String,
    pub image_file: Upload,
}

#[derive(SimpleObject, Deserialize)]
pub struct CreateCardOutput {
    pub card: Card,
}

#[derive(sqlx::FromRow)]
pub struct UserAndCard {
    pub card: sqlx::types::Json<Card>,
    pub user: sqlx::types::Json<User>,
}

#[derive(sqlx::FromRow, SimpleObject, Deserialize)]
pub struct GachaCardOutput {
    pub card: Card,
    pub user: User,
}

#[derive(Default)]
pub struct CardMutation;

#[Object]
impl CardMutation {
    async fn create_card(
        &self,
        ctx: &Context<'_>,
        input: CreateCardInput,
    ) -> Result<CreateCardOutput, GraphqlError> {
        let CreateCardInput { name, image_file } = input;
        let session: &Session = ctx
            .data_opt::<Session>()
            .ok_or_else(|| UserError::NotAuthorized.extend())?;
        // HACKIT: need upload ticket validation
        let dbpool = ctx.data::<DbPool>()?;
        let s3_uploader = ctx.data::<S3Uploader>()?;

        let upload = image_file.value(ctx)?;
        let size = upload.size().ok().map(|u| u as i64);
        let mime_type = upload.content_type;
        let file = upload.content;

        let upload_path = s3_uploader
            .put_object(&rand_string(30), file, size, mime_type)
            .await?;

        let card = sqlx::query_as::<_, Card>(
            "INSERT INTO cards (name, owner_id, owned_at, image_path, creator_id) VALUES ($1, $2, $3, $4, $5) RETURNING *")
            .bind(name)
            .bind(session.user_id)
            .bind(chrono::Utc::now())
            .bind(upload_path)
            .bind(session.user_id)
            .fetch_one(dbpool)
            .await?;

        Ok(CreateCardOutput { card })
        /*let info = FileInfo {
            id: entry.key().into(),
            filename: upload.filename.clone(),
            mimetype: upload.content_type,
        };*/
    }
    async fn gacha_card(&self, ctx: &Context<'_>) -> Result<GachaCardOutput, GraphqlError> {
        let session: &Session = ctx
            .data_opt::<Session>()
            .ok_or_else(|| UserError::NotAuthorized.extend())?;
        let dbpool = ctx.data::<DbPool>()?;
        let (points,): (i32,) = sqlx::query_as("SELECT points FROM users WHERE id = $1")
            .bind(session.user_id)
            .fetch_one(dbpool)
            .await?;

        if points < GACHA_POINTS {
            return Err(UserError::NotEnoughPoints.extend());
        }

        let UserAndCard { user, card }: UserAndCard = sqlx::query_as(
            r#"
            WITH user_updated AS (
                UPDATE users 
                SET points = points - 100 
                WHERE id = $1 AND points >= 100 RETURNING *
            ),
            card_updated AS (
                UPDATE cards 
                SET owner_id = $1,
                    owned_at = NOW()
                WHERE 
                    EXISTS (SELECT 1 FROM user_updated) AND
                    gacha_queue_index = (
                        SELECT gacha_queue_index
                        FROM cards 
                        WHERE owner_id IS NULL 
                        ORDER BY gacha_queue_index
                        LIMIT 1
                        FOR UPDATE SKIP LOCKED)
                RETURNING *
            )
            SELECT 
                TO_JSONB(user_updated) as user, TO_JSONB(card_updated) as card
            FROM user_updated, card_updated 
            "#,
        )
        .bind(session.user_id)
        .fetch_one(dbpool)
        .await?;

        Ok(GachaCardOutput {
            card: card.0,
            user: user.0,
        })
    }
}

/*
#[derive(Default)]
pub struct CardQuery;

#[Object]
impl CardQuery {
    async fn random_free_card(
        &self,
        ctx: &Context<'_>,
    ) -> Result<Card, GraphqlError> {
        let session: &Session = ctx
            .data_opt::<Session>()
            .ok_or_else(|| GraphqlError::from(Error::NotAuthorized))?;
        // HACKIT: need upload ticket validation
        let dbpool = ctx.data::<DbPool>()?;
        let s3_uploader = ctx.data::<S3Uploader>()?;

        let upload = image_file.value(ctx)?;
        let size = upload.size().ok().map(|u| u as i64);
        let mime_type = upload.content_type;
        let file = upload.content;

        let upload_path = s3_uploader
            .put_object(&rand_string(30), file, size, mime_type)
            .await?;

        let card = sqlx::query_as::<_, Card>(
            "INSERT INTO cards (name, owner_id, owned_at, image_path, creator_id) VALUES ($1, $2, $3, $4, $5) RETURNING *")
            .bind(name)
            .bind(session.user_id)
            .bind(chrono::Utc::now())
            .bind(upload_path)
            .bind(session.user_id)
            .fetch_one(dbpool)
            .await?;

        Ok(CreateCardOutput { card })
        /*let info = FileInfo {
            id: entry.key().into(),
            filename: upload.filename.clone(),
            mimetype: upload.content_type,
        };*/
    }
}
*/

#[cfg(test)]
pub mod tests {
    use crate::utils::test::*;
    use crate::{models, routes, session};
    use actix_web::{App, HttpServer};
    use futures::TryFutureExt;

    #[derive(serde::Serialize)]
    struct Query {
        query: String,
    }
    impl Query {
        fn new(q: &str) -> Self {
            Query {
                query: q.to_string(),
            }
        }
    }

    #[actix_rt::test]
    async fn test_gacha_card() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let user_id = uuid::Uuid::new_v4();

        let schema = models::schema::Schema::build(
            models::schema::Query::default(),
            models::schema::Mutation::default(),
            async_graphql::EmptySubscription,
        )
        .data(db.pgpool.clone())
        .data(db.redispool.clone())
        .data(db.s3uploader.clone())
        .data(session::Session {
            user_id,
            user_kind: models::user::UserKind::Normal,
        })
        .finish();

        let dbpool = db.pgpool;

        sqlx::query("INSERT INTO users (id, nickname, email, password, points) VALUES ($1, 'a', 'b', 'c', 200)")
            .bind(user_id)
            .execute(&dbpool)
            .await
            .unwrap();
        for i in 0i32..2 {
            sqlx::query("INSERT INTO cards (rating, image_path, name) VALUES ($1, $2, $3)")
                .bind(i as f32)
                .bind(i.to_string())
                .bind(i.to_string())
                .execute(&dbpool)
                .await
                .unwrap();
        }
        let query = "mutation { gachaCard { user { points } card { name } } }";
        let res: Vec<serde_json::Value> =
            futures::future::join_all(vec![schema.execute(query), schema.execute(query)])
                .await
                .into_iter()
                .map(|r| {
                    assert_eq!(r.errors, Vec::new(),);
                    async_graphql::from_value(r.data).unwrap()
                })
                .collect();
        let user_points: Vec<_> = res
            .iter()
            .map(|r| {
                r.get("gachaCard")
                    .unwrap()
                    .get("user")
                    .unwrap()
                    .get("points")
                    .unwrap()
                    .as_i64()
                    .unwrap()
            })
            .collect();
        let card_names: Vec<_> = res
            .iter()
            .map(|r| {
                r.get("gachaCard")
                    .unwrap()
                    .get("card")
                    .unwrap()
                    .get("name")
                    .unwrap()
                    .as_str()
                    .unwrap()
            })
            .collect();
        assert_ne!(user_points[0], user_points[1]);
        assert_ne!(card_names[0], card_names[1]);

        let res = schema.execute(query).await;
        assert_eq!(res.errors[0].to_string(), "not enough points");
    }

    #[actix_rt::test]
    async fn test_card_upload() {
        let docker = TestDocker::new();
        let db = docker.run().await;
        let port = portpicker::pick_unused_port().expect("No ports free");

        let schema = db.schema.clone();
        let dbpool = db.pgpool.clone();
        let redispool = db.redispool.clone();
        let s3uploader = db.s3uploader.clone();

        let _ = actix_rt::spawn(
            HttpServer::new(move || {
                App::new()
                    .data(schema.clone())
                    .data(dbpool.clone())
                    .data(redispool.clone())
                    .data(s3uploader.clone())
                    .configure(routes::routes)
            })
            .bind(format!("0.0.0.0:{}", port))
            .unwrap()
            .run()
            .unwrap_or_else(|_| ()),
        );

        //actix_rt::time::sleep(std::time::Duration::from_secs(1)).await;

        let client = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap();
        let url = format!("http://localhost:{}/graphql", port);

        let res = client.post(&url).json(&Query::new("mutation { register(input: { email:\"a\", password:\"b\", nickname:\"c\" }) { user { id } } }")).send().await.unwrap();
        /*assert_eq!(res.text().await.unwrap(), "");
        let set_cookie_header = "";
        let user_id = Uuid::new_v4();*/
        let set_cookie_header = res
            .headers()
            .get("Set-Cookie")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let res: serde_json::Value = res.json().await.unwrap();
        let user_id = res
            .get("data")
            .unwrap()
            .get("register")
            .unwrap()
            .get("user")
            .unwrap()
            .get("id")
            .unwrap()
            .as_str()
            .unwrap();

        /*let mut file = NamedTempFile::new().unwrap();
        write!(file, "hi").unwrap();*/

        let form = reqwest::multipart::Form::new()
            .text("operations", r#"{ "query": "mutation ($file: Upload!) { createCard(input: { imageFile: $file, name: \"a\" }) { card { name ownerId creatorId imagePath } } }", "variables": { "file": null } }"#)
            .text("map", r#"{ "0": ["variables.file"] }"#)
            .part("0", reqwest::multipart::Part::stream("hi").file_name("hi.txt").mime_str("text/plain").unwrap());

        let res: String = client
            .post(&url)
            .header("COOKIE", set_cookie_header.split(";").next().unwrap())
            .multipart(form)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let res: serde_json::Value = serde_json::from_str(&res).unwrap();
        let card = res
            .get("data")
            .unwrap()
            .get("createCard")
            .unwrap()
            .get("card")
            .unwrap();
        //assert_eq!(res, "");
        /*let res: Res = client.post(&url).multipart(form).send().await.unwrap().json().await.unwrap();
        assert_eq!(res.data.get("createCard"), Some(&"".to_string()));*/
        assert_eq!(card.get("name").unwrap().as_str().unwrap(), "a");
        assert_eq!(card.get("ownerId").unwrap().as_str(), Some(user_id));
        assert_eq!(card.get("creatorId").unwrap().as_str(), Some(user_id));

        let uploaded_contents = reqwest::get(card.get("imagePath").unwrap().as_str().unwrap())
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(uploaded_contents, "hi");
    }
}
