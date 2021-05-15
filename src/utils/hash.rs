use bcrypt::{hash, verify, BcryptError, DEFAULT_COST};
use rand::{distributions::Alphanumeric, thread_rng, Rng};

pub fn hash_password(plain: String) -> Result<String, BcryptError> {
    hash(plain, DEFAULT_COST)
}
pub fn verify_password(plain: &str, hash: &str) -> Result<bool, BcryptError> {
    verify(plain, hash)
}

pub fn rand_string(len: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}
