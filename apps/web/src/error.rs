//! `platform::AppError` 到 HTTP 响应的映射。
//! 页面错误模板与 JSON 端点分支随后续工作包接入。

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use platform::error::AppError;

/// web 层错误包装：`AppError` 定义在 platform，需本地包装实现 axum 响应转换。
#[allow(dead_code)] // 随首批真实路由接入使用
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        ApiError(err)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        // 内部错误的原文只进日志：它可能含连接串、SQL 片段或内部路径。
        if self.0.is_internal() {
            tracing::error!(status = status.as_u16(), detail = %self.0, "请求失败");
        }
        (status, self.0.public_message().to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_status_codes() {
        assert_eq!(
            ApiError(AppError::NotFound("x".into())).0.http_status(),
            404
        );
        assert_eq!(
            ApiError(AppError::Unavailable("x".into())).0.http_status(),
            503
        );
    }

    #[test]
    fn internal_response_body_is_generic() {
        let err = AppError::Internal("postgres://user:pw@host/db".into());
        assert_eq!(err.public_message(), "服务出现内部错误，请稍后重试");
    }
}
