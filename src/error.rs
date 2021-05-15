use crate::models::battle;
use actix_web::ResponseError;
use async_graphql::{Error as GraphqlError, ErrorExtensions};
use thiserror::Error as thisError;

#[derive(thisError, Debug)]
pub enum UserError {
    #[error("need login")]
    NeedLogin,
    #[error("not authorized")]
    NotAuthorized,
    #[error("wrong password")]
    WrongPassword,
    #[error("not enough points")]
    NotEnoughPoints,
    #[error("invalid request form. detail={0:?}")]
    BadRequest(&'static str),
}

impl ErrorExtensions for UserError {
    fn extend(&self) -> GraphqlError {
        GraphqlError::new(format!("{}", self))
    }
}

#[derive(thisError, Debug)]
pub enum ServerError {
    #[error("redis pool not found in context")]
    RedisPoolNotFoundInContext,
    #[error("database query error(sqlx): {0:?}")]
    Database(#[from] sqlx::Error),
    #[error("sqlx migration error: {0:?}")]
    DatabaseMigrate(#[from] sqlx::migrate::MigrateError),
    #[allow(dead_code)]
    #[error("not implemtned yet. method={0:?} detail={1:?}")]
    NotImplemented(&'static str, &'static str),
    #[error("bcrypt error: {0:?}")]
    BcryptError(#[from] bcrypt::BcryptError),
    #[error("jwt error: {0:?}")]
    JwtError(#[from] jsonwebtoken::errors::Error),
    #[error("bincode error: {0:?}")]
    BincodeError(#[from] bincode::Error),
    #[error("serde_json error: {0:?}")]
    SerdeJsonError(#[from] serde_json::Error),
    #[error("base64 error: {0:?}")]
    Base64Error(#[from] base64::DecodeError),
    #[error("io error: {0:?}")]
    IoError(#[from] std::io::Error),
    #[error("redis pool error: {0:?}")]
    RedisPoolError(#[from] deadpool_redis::PoolError),
    #[error("redis error: {0:?}")]
    RedisError(#[from] redis::RedisError),
    #[error("s3 parse region error: {0:?}")]
    ParseRegionError(#[from] rusoto_core::region::ParseRegionError),
    #[error("rusoto put object error: {0:?}")]
    RusotoError(#[from] rusoto_core::RusotoError<rusoto_s3::PutObjectError>),
    #[error("uuid parse error: {0:?}")]
    UuidParseError(#[from] uuid::Error), /*#[error("pubsub error: {0:?}")]
                                         PubsubError(#[from] pubsub::Error),*/
}

impl ResponseError for ServerError {}

impl ErrorExtensions for ServerError {
    fn extend(&self) -> GraphqlError {
        GraphqlError::new(format!("{}", self))
    }
}

#[derive(thisError, Debug)]
pub enum Error {
    #[error("server error: {0:?}")]
    ServerError(#[from] ServerError),
    #[error("user error: {0:?}")]
    UserError(#[from] UserError),
    #[error("battle error: {0:?}")]
    BattleError(#[from] battle::Error),
}

impl ResponseError for Error {}

/*impl From<Error> for GraphqlError {
    fn from(e: Error) -> Self {
        GraphqlError::new(format!("{}", e))
    }
}*/

impl ErrorExtensions for Error {
    fn extend(&self) -> GraphqlError {
        GraphqlError::new(format!("{}", self))
    }
}
