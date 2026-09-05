//! 管理后台：RBAC/ABAC registry、审批与 `/manage` 契约。随 WP-MVP-D04 接入。
//! 授权链路遵守 `plan/03`；数据端点使用 `/manage/_api/v1/` 前缀。

pub mod console;
pub mod iam;
