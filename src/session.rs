use crate::constants::{SESSION_ID_LENGTH, SESSION_LIFETIME_SECONDS};
use crate::error::ServerError;
use crate::models::user::{User, UserKind};
use crate::utils::hash::rand_string;
use actix_web::HttpRequest;
use deadpool_redis::{cmd, ConnectionWrapper as RedisConn, Pool as RedisPool};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub struct ClientInfo {
    pub remote_addr: String,
}

pub fn extract_client_info(req: &HttpRequest) -> Option<ClientInfo> {
    let remote_addr = req.connection_info().remote_addr().map(|r| r.to_owned());
    remote_addr.map(|remote_addr| ClientInfo { remote_addr })
}

fn build_session_key(id: &str) -> String {
    format!("session:{}", id)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Session {
    pub user_id: Uuid,
    pub user_kind: UserKind,
}

pub async fn create_session(
    ctx: &async_graphql::Context<'_>,
    user: &User,
) -> Result<(), ServerError> {
    let mut redis_conn = ctx
        .data_opt::<RedisPool>()
        .ok_or(ServerError::RedisPoolNotFoundInContext)?
        .get()
        .await?;
    let session = Session {
        user_id: user.id,
        user_kind: user.kind,
    };
    let session_id: String = rand_string(SESSION_ID_LENGTH);
    let expire_at = chrono::Utc::now() + chrono::Duration::seconds(SESSION_LIFETIME_SECONDS);
    let session_cookie_header = format!(
        "session-id={}; Secure; HttpOnly; Expires={}",
        session_id,
        expire_at.format("%a, %d %b %Y %H:%M:%S GMT")
    );
    println!("add header");
    ctx.append_http_header("Set-Cookie", session_cookie_header);
    cmd("SETEX")
        .arg(build_session_key(&session_id))
        .arg(SESSION_LIFETIME_SECONDS)
        .arg(bincode::serialize(&session)?)
        .execute_async(&mut redis_conn)
        .await?;
    Ok(())
}

/*pub async fn remove_session(
    ctx: &async_graphql::Context<'_>,
) -> Result<Option<Session>, Error> {
}*/

pub async fn extract_session(
    redis_conn: &mut RedisConn,
    req: &HttpRequest,
) -> Result<Option<Session>, ServerError> {
    if let Some(session_id) = req.cookie("session-id") {
        let bytes: Option<Vec<u8>> = cmd("GET")
            .arg(build_session_key(session_id.value()))
            .query_async(redis_conn)
            .await?;
        if let Some(bytes) = bytes {
            let sess: Session = bincode::deserialize(&bytes)?;
            Ok(Some(sess))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}
