use crate::errors::PersistenceError;

pub mod bitcoin_wallet;

pub use bitcoin_wallet::PgBitcoinWalletStore;

pub(super) fn map_sqlx_error(err: sqlx::Error) -> PersistenceError {
    if let sqlx::Error::Database(db_err) = &err {
        match db_err.code().as_deref() {
            Some("23505") => return PersistenceError::Conflict(db_err.message().to_string()),
            Some("23503") => return PersistenceError::NotFound(db_err.message().to_string()),
            _ => {},
        }
    }
    PersistenceError::Query(err.to_string())
}
