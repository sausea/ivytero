use super::{ApiError, AppState};
use axum::extract::{Request, State};
use axum::http::{header::AUTHORIZATION, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn bearer(req: &Request) -> Option<&str> {
    req.headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
}

fn unauthorized() -> Response {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "unauthorized",
        "missing or invalid bearer token",
    )
    .into_response()
}

pub async fn require_admin_token(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    match (bearer(&req), state.admin_token.as_deref()) {
        (Some(token), Some(admin)) if constant_time_eq(token.as_bytes(), admin.as_bytes()) => {
            next.run(req).await
        }
        _ => unauthorized(),
    }
}

pub async fn require_read_token(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let presented = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim);
    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), state.read_token.as_bytes()) => {
            next.run(req).await
        }
        _ => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid bearer token",
        )
        .into_response(),
    }
}
