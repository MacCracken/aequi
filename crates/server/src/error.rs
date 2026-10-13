use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::error::ErrorKind;

#[derive(Debug)]
pub enum ApiError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    Unauthorized,
    Forbidden(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, "Unauthorized".to_string()),
            ApiError::Forbidden(msg) => (StatusCode::FORBIDDEN, msg),
            ApiError::Internal(msg) => {
                tracing::error!(error = %msg, "Internal server error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        };

        let body = json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}

/// Storage validates its inputs and reports bad input as `InvalidArgument`
/// and a missing row as `RowNotFound`; constraint violations are the client's
/// fault too. Everything else is an internal error whose text (SQL, paths) is
/// logged but never returned to the client.
impl From<sqlx::Error> for ApiError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::InvalidArgument(msg) => ApiError::BadRequest(msg),
            sqlx::Error::RowNotFound => ApiError::NotFound("Not found".to_string()),
            sqlx::Error::Database(ref db) => match db.kind() {
                ErrorKind::UniqueViolation => {
                    ApiError::Conflict("A record with these values already exists".to_string())
                }
                ErrorKind::ForeignKeyViolation => {
                    ApiError::BadRequest("Referenced record does not exist".to_string())
                }
                ErrorKind::NotNullViolation | ErrorKind::CheckViolation => {
                    ApiError::BadRequest("Invalid or missing field value".to_string())
                }
                _ => ApiError::Internal(e.to_string()),
            },
            other => ApiError::Internal(other.to_string()),
        }
    }
}

impl From<aequi_core::LedgerError> for ApiError {
    fn from(e: aequi_core::LedgerError) -> Self {
        ApiError::BadRequest(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status_of(e: ApiError) -> StatusCode {
        e.into_response().status()
    }

    #[test]
    fn storage_validation_errors_are_client_errors() {
        let e: ApiError = sqlx::Error::InvalidArgument("bad date".into()).into();
        assert!(matches!(e, ApiError::BadRequest(ref m) if m == "bad date"));
        assert_eq!(status_of(e), StatusCode::BAD_REQUEST);

        let e: ApiError = sqlx::Error::RowNotFound.into();
        assert_eq!(status_of(e), StatusCode::NOT_FOUND);
    }

    #[test]
    fn ledger_errors_are_client_errors() {
        let e: ApiError = aequi_core::LedgerError::NegativeAmount(1).into();
        assert_eq!(status_of(e), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn constraint_violations_map_to_4xx_without_sql_text() {
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL UNIQUE)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t (v) VALUES ('a')")
            .execute(&pool)
            .await
            .unwrap();

        let err = sqlx::query("INSERT INTO t (v) VALUES ('a')")
            .execute(&pool)
            .await
            .unwrap_err();
        let api: ApiError = err.into();
        assert!(matches!(api, ApiError::Conflict(ref m) if !m.contains("UNIQUE")));

        let err = sqlx::query("INSERT INTO t (v) VALUES (NULL)")
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(matches!(ApiError::from(err), ApiError::BadRequest(_)));
    }

    #[test]
    fn internal_errors_hide_details() {
        let e: ApiError = sqlx::Error::Protocol("SELECT secret FROM t".into()).into();
        assert!(matches!(e, ApiError::Internal(_)));
        let resp = e.into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
