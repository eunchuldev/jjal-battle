use crate::utils::datetime::DateTime;
use chrono::TimeZone;
use lazy_static::lazy_static;

/* USER CONSTANTS */
pub const GACHA_POINTS: i32 = 100;
pub const USER_CARDS_MAX_PAGE_SIZE: i32 = 100;

/* SESSION CONSTANTS */
pub const SESSION_LIFETIME_SECONDS: i64 = 60 * 60 * 24;
pub const SESSION_ID_LENGTH: usize = 30;

/* DB CONSTANTS(SHOULD NOT BE MODIFIED) */
lazy_static! {
    pub static ref MAX_DATETIME: DateTime = chrono::Utc.ymd(9999, 1, 1).and_hms(0, 1, 1);
    pub static ref MIN_DATETIME: DateTime = chrono::Utc.ymd(0, 1, 1).and_hms(0, 1, 1);
}

pub const DUEL_BATTLE_TTL_SECONDS: i64 = 60 * 10;
