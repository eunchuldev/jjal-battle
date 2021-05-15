use chrono::{TimeZone, Utc};
use lazy_static::lazy_static;
pub type DateTime = chrono::DateTime<Utc>;
lazy_static! {
    pub static ref MAX_DATETIME: DateTime = Utc.ymd(9999, 1, 1).and_hms(0, 1, 1);
    pub static ref MIN_DATETIME: DateTime = Utc.ymd(0, 1, 1).and_hms(0, 1, 1);
}
