#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::env::VarError> for Error {
    fn from(e: std::env::VarError) -> Self {
        Error(format!("missing env var: {e}"))
    }
}

