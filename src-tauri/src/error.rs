//! 结构化错误体系(Rust → 前端契约)。
//!
//! 统一形态:serde tag/content 序列化为 `{"kind":"<PascalCase>","data":{...}}`,
//! unit 变体无 data 字段;字段 camelCase。前端经 toStructuredError 归约、渲染时
//! 按 `errors.<kind>` 键翻译(见 src/lib/error.ts 与 src/locales/*.json)——错误串
//! 不在 Rust 侧拼装,数据只携带运行时事实,文案模板在 locale JSON。
//!
//! 按域划分三个枚举,共用同一形状(每个枚举的序列化契约由各自的单测守住,
//! 任一环漂移即红灯):DshError(dsh 生命周期错误:node 检测 / npm 安装 / dsh
//! 启动 / 导航;boot 流水线与 dsh 升级链共用,故原名 BootError 改为按域命名)、
//! UpdateError(应用自身升级错误)、UpgradeError(dsh 升级错误——升级特有 kind
//! 独立枚举 + DshError 透传,untagged 序列化为单一 {kind,data} 形态,
//! 前端零额外机制)。

use serde::Serialize;

/// dsh 生命周期失败原因(kind + data,serde tag/content 序列化为
/// `{"kind":"NodeCheckTimeout","data":{"seconds":10}}`,unit 变体无 data 字段)。
/// 前端经 toStructuredError 归约、渲染时按 `errors.<kind>` 键翻译
/// (见 src/lib/error.ts 与 src/locales/*.json)——错误串不在此处拼装,
/// 数据只携带运行时事实(超时秒数/退出码/版本/stderr 原文),文案模板在 locale JSON。
///
/// NodeMissing/NodeVersionUnmet 两个 kind 走 Node 引导页(展示要求 + 当前检测
/// 结果 + 官网下载/重试,见前端 isNodeGuideError);其余错误留通用错误页。
/// 版本规格(required)只由 npm.rs 的 NODE_REQ 持有,随错误数据传给前端渲染——
/// 前端不复制规格文本,避免 zh/en 与 Rust 三处维护同一串。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "PascalCase", rename_all_fields = "camelCase")]
pub enum DshError {
    /// 未检测到 Node.js(带版本要求,供引导页展示)
    NodeMissing { required: String },
    /// `node --version` 检查超时
    NodeCheckTimeout { seconds: u64 },
    /// `node --version` 进程 IO 失败
    NodeCheckFailed { detail: String },
    /// `node --version` 非零退出
    NodeVersionCheckFailed { exit_code: i32, detail: String },
    /// 无法解析 node 版本号
    NodeVersionParseFailed { version: String },
    /// 版本不满足 ^22.19 || >=24
    NodeVersionUnmet { current: String, required: String },
    /// 无法执行 npm(未安装/不可用)
    NpmRootSpawnFailed,
    /// `npm root -g` 超时
    NpmRootTimeout { seconds: u64 },
    /// `npm root -g` 进程 IO 失败
    NpmRootIoFailed { detail: String },
    /// `npm root -g` 非零退出
    NpmRootExitFailed { exit_code: i32, detail: String },
    /// `npm root -g` 输出为空
    NpmRootEmpty,
    /// 无法启动 npm 安装进程
    NpmSpawnFailed { detail: String },
    /// 安装失败:权限类(EPERM/EACCES),带退出码
    InstallFailedPermission { exit_code: i32, stderr_tail: String },
    /// 安装失败:权限类,无退出码(异常退出)
    InstallFailedPermissionAbnormal { stderr_tail: String },
    /// 安装失败:非权限类(网络等),带退出码
    InstallFailedNetwork { exit_code: i32, stderr_tail: String },
    /// 安装失败:非权限类,无退出码(异常退出)
    InstallFailedNetworkAbnormal { stderr_tail: String },
    /// 安装超时
    InstallTimeout { seconds: u64 },
    /// 安装进程 IO 异常
    NpmInstallIoFailed { detail: String },
    /// 安装后完整性复检失败
    InstallVerifyFailed,
    /// 无法启动 dsh 进程
    DshSpawnFailed { detail: String },
    /// 就绪行已打印但端口未监听
    ReadyPortUnavailable { port: u16 },
    /// 进程提前退出,已知退出码
    DshExitedEarly { exit_code: i32 },
    /// 进程提前退出,无退出码(句柄缺失等)
    DshExitedEarlyNoCode,
    /// 启动超时(未收到就绪信号)
    DshStartTimeout { seconds: u64 },
    /// 无法导航窗口到 dsh 页面
    NavigateFailed,
    /// 流水线内部 panic 等未知内部错误
    Internal { message: String },
}

/// 应用自身升级失败的结构化原因(kind + data,serde tag/content,与 DshError 同形态)。
/// 文案模板在 locale JSON 的 `errors.<kind>` 键,数据只携带运行时事实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "PascalCase", rename_all_fields = "camelCase")]
pub enum UpdateError {
    /// 下载/安装失败(网络、签名校验、NSIS 执行等),detail 为插件原始错误串
    DownloadFailed { detail: String },
}

/// 升级特有错误(kind 与 dsh 生命周期错误不重名)。文案模板在 locale JSON
/// 的 `errors.<kind>` 键,数据只携带运行时事实。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "PascalCase", rename_all_fields = "camelCase")]
pub enum UpgradeErrorKind {
    /// 无法停止当前 dsh 服务(杀后超时仍存活)
    UpgradeKillFailed { detail: String },
    /// 升级后版本校验失败(全局 version ≠ pin 或 bin.js 缺失)
    UpgradeVerifyFailed,
}

/// dsh 升级失败的结构化原因:升级特有错误 + 与 dsh 生命周期共用的安装/启动类
/// 错误直接以 DshError 形态透传(复用流水线函数返回类型,前端统一按 errors.<kind>
/// 翻译,零额外机制)。untagged 序列化为单一 {kind,data} 形态。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum UpgradeError {
    Kind(UpgradeErrorKind),
    Dsh(DshError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsh_error_serializes_as_kind_and_data() {
        // 前端 toStructuredError 依赖的线上契约:tag/content 判别式,
        // 字段 camelCase;unit 变体无 data 字段
        assert_eq!(
            serde_json::to_value(DshError::NodeCheckTimeout { seconds: 10 }).unwrap(),
            serde_json::json!({ "kind": "NodeCheckTimeout", "data": { "seconds": 10 } })
        );
        assert_eq!(
            serde_json::to_value(DshError::DshExitedEarly { exit_code: 1 }).unwrap(),
            serde_json::json!({ "kind": "DshExitedEarly", "data": { "exitCode": 1 } })
        );
        assert_eq!(
            serde_json::to_value(DshError::NodeMissing {
                required: "Node.js ^22.19 or >=24".to_string()
            })
            .unwrap(),
            serde_json::json!({ "kind": "NodeMissing", "data": { "required": "Node.js ^22.19 or >=24" } })
        );
    }

    #[test]
    fn update_error_serializes_as_kind_and_data() {
        // 前端 toStructuredError 依赖的线上契约:tag/content 判别式,
        // 字段 camelCase;与 DshError 同形态(见 error.rs)
        assert_eq!(
            serde_json::to_value(UpdateError::DownloadFailed { detail: "boom".into() }).unwrap(),
            serde_json::json!({ "kind": "DownloadFailed", "data": { "detail": "boom" } })
        );
    }

    #[test]
    fn upgrade_error_serializes_as_kind_and_data() {
        // 前端 toStructuredError 依赖的线上契约:tag/content 判别式,
        // 字段 camelCase;unit 变体无 data 字段(与 DshError 同形态)
        assert_eq!(
            serde_json::to_value(UpgradeError::Kind(UpgradeErrorKind::UpgradeKillFailed {
                detail: "3s 内未退出".into()
            }))
            .unwrap(),
            serde_json::json!({ "kind": "UpgradeKillFailed", "data": { "detail": "3s 内未退出" } })
        );
        assert_eq!(
            serde_json::to_value(UpgradeError::Kind(UpgradeErrorKind::UpgradeVerifyFailed)).unwrap(),
            serde_json::json!({ "kind": "UpgradeVerifyFailed" })
        );
        // DshError 形态透传:untagged 序列化与 DshError 自身一致,前端零额外机制
        assert_eq!(
            serde_json::to_value(UpgradeError::Dsh(DshError::DshStartTimeout { seconds: 180 }))
                .unwrap(),
            serde_json::json!({ "kind": "DshStartTimeout", "data": { "seconds": 180 } })
        );
    }
}
