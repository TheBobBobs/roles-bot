use volty::{http::error::HttpError, types::permissions::Permission};

#[derive(Debug)]
pub enum Error {
    Custom(String),

    InvalidRole(String),

    Missing(Permission),
    MemberRankTooHigh,
    RoleRankTooHigh(String),

    UserMissing(Permission),
    UserRankTooLow(String),
    InvalidUser,

    Http(HttpError),
}

impl From<HttpError> for Error {
    fn from(value: HttpError) -> Self {
        Self::Http(value)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        dbg!(value);
        Self::Custom("Database Error!".to_string())
    }
}
