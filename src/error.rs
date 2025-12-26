use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub enum DocError {
    ParseError,
    Timeout,
    OtherError(Box<dyn Error>),
    StatusError(reqwest::Error),
    ConnectionError(reqwest::Error),
    IOError(std::io::Error),
    ClosedDocUsage,
    BrokenCache,
}

impl Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocError::ParseError => return write!(f, "Unexpected Parse Error"),
            DocError::BrokenCache => {
                return write!(
                    f,
                    "Document chache was broken, can be handled by recreating doc object"
                );
            }
            DocError::ClosedDocUsage => {
                return write!(f, "Document can't handle reqwests after clousure");
            }
            DocError::ConnectionError(e) => return write!(f, "Can not connect: {}", e),
            DocError::StatusError(e) => return write!(f, "Error status: {}:", e),
            DocError::Timeout => return write!(f, "Timeout"),
            DocError::IOError(e) => return write!(f, "IO Error: {e}"),
            DocError::OtherError(e) => return write!(f, "{e}"),
        }
    }
}

impl Error for DocError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            DocError::ConnectionError(v) => return v.source(),
            DocError::StatusError(v) => return v.source(),
            DocError::OtherError(v) => return v.source(),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for DocError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_timeout() {
            return Self::Timeout;
        }
        if value.is_status() {
            return Self::StatusError(value);
        }
        if value.is_connect() {
            return Self::ConnectionError(value);
        }
        return Self::OtherError(Box::new(value));
    }
}

impl From<std::io::Error> for DocError {
    fn from(value: std::io::Error) -> Self {
        return Self::IOError(value);
    }
}
