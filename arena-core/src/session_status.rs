use std::str::FromStr;

use sea_orm::sea_query::{ColumnType, Nullable, ValueType};
use sea_orm::{TryGetError, TryGetable};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Lobby,
    Running,
    Paused,
    Finished,
    Cancelled,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lobby => write!(f, "lobby"),
            Self::Running => write!(f, "running"),
            Self::Paused => write!(f, "paused"),
            Self::Finished => write!(f, "finished"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown session status: {0}")]
pub struct UnknownSessionStatus(String);

impl FromStr for SessionStatus {
    type Err = UnknownSessionStatus;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "lobby" => Ok(Self::Lobby),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "finished" => Ok(Self::Finished),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(UnknownSessionStatus(other.to_owned())),
        }
    }
}

impl From<SessionStatus> for sea_orm::sea_query::Value {
    fn from(status: SessionStatus) -> Self {
        Self::String(Some(Box::new(status.to_string())))
    }
}

impl ValueType for SessionStatus {
    fn try_from(v: sea_orm::sea_query::Value) -> Result<Self, sea_orm::sea_query::ValueTypeErr> {
        match v {
            sea_orm::sea_query::Value::String(Some(boxed)) => {
                Self::from_str(&boxed).map_err(|_| sea_orm::sea_query::ValueTypeErr)
            }
            _ => Err(sea_orm::sea_query::ValueTypeErr),
        }
    }

    fn type_name() -> String {
        "SessionStatus".to_owned()
    }

    fn array_type() -> sea_orm::sea_query::ArrayType {
        sea_orm::sea_query::ArrayType::String
    }

    fn column_type() -> ColumnType {
        ColumnType::String(sea_orm::sea_query::StringLen::N(16))
    }
}

impl Nullable for SessionStatus {
    fn null() -> sea_orm::sea_query::Value {
        sea_orm::sea_query::Value::String(None)
    }
}

impl TryGetable for SessionStatus {
    fn try_get_by<I: sea_orm::ColIdx>(
        res: &sea_orm::QueryResult,
        index: I,
    ) -> Result<Self, TryGetError> {
        let s: String = TryGetable::try_get_by(res, index)?;
        Self::from_str(&s).map_err(|e| TryGetError::DbErr(sea_orm::DbErr::Custom(e.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_round_trips_every_variant() {
        for status in [
            SessionStatus::Lobby,
            SessionStatus::Running,
            SessionStatus::Paused,
            SessionStatus::Finished,
            SessionStatus::Cancelled,
        ] {
            assert_eq!(status.to_string().parse::<SessionStatus>().unwrap(), status);
        }
    }

    #[test]
    fn from_str_rejects_unknown_status() {
        let err = "warmup".parse::<SessionStatus>().unwrap_err();
        assert_eq!(err.to_string(), "unknown session status: warmup");
    }
}
