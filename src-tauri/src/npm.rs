//! npm 工具链域:node 环境检测、全局 dsh 检测、**安装执行全流程**(命令构造 →
//! 超时/孤儿回收 → ETARGET 回退 → 错误分类)、安装进度模拟。
//!
//! boot 流水线(dsh.rs)与 dsh 升级链(upgrade.rs)共用(单一事实来源);
//! 纯函数均可测,进程型函数只依赖 crate::proc 的子进程工具。安装执行与调用方
//! 状态相关的两个副作用(pid 登记 / 日志行转发)经 [`InstallObserver`] seam
//! 注入,生产实现是 DshManager——本模块不依赖 DshManager。

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::DshError;
use crate::proc::{new_process_group, no_window, run_with_timeout};

/// dsh 要求的 Node 版本(仓库根 package.json engines):^22.19 || >=24。
/// 作为 NodeVersionUnmet 的结构化数据传给前端(版本规格是技术串,语言中立,
/// 保持英文形态以免 zh/en 两处维护同一规格)。
const NODE_REQ: &str = "Node.js ^22.19 or >=24";
/// `node --version` 检查超时。同步 output() 无上限:node 被 shim/杀软/网络盘挂起时
/// checking 会永久卡住,必须设上限(超时后杀进程并报可读错误)。
const CHECK_NODE_TIMEOUT: Duration = Duration::from_secs(10);
/// npm 安装超时。冷缓存首次安装可能要几分钟,给足 10 分钟;超时视为失败并报可读错误。
/// 安装执行本体(install_global)在本文件,直接消费此常量。
pub(crate) const NPM_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
/// 安装包内置 npm 离线缓存的相对目录名(位于 Tauri 资源目录下)。
/// 约定由本文件与 #6(CI 发版打包)共同持有:CI 把发版时的 dsh 依赖树提前
/// 下载成 npm cacache 打进安装包,本模块只消费、不校验内容。
const BUNDLE_CACHE_REL: &str = "npm-cache";
/// 全局 node_modules 路径,运行时动态解析(`npm root -g`)。
/// 不可写死 %APPDATA%\npm:nvm 等环境 prefix 不同(本机实测 nvm 下为 E:\Nvm\nodejs)。
/// 带超时:`npm root -g` 也会拉起 node,npm/node 被挂起时不得让 boot 卡死在检查阶段。
const NPM_ROOT_TIMEOUT: Duration = Duration::from_secs(10);

// ── 命令构造 ───────────────────────────────────────────────────────

/// npm 命令构造(单一事实来源,检测与安装共用):
/// Windows 上 CreateProcess 不能直接执行 .cmd/.bat(npm 是 .cmd shim),
/// 须经 cmd.exe /c 包装;同时隐藏控制台窗口。Unix 上放入新进程组——
/// kill_pid_tree 按组杀(npm 会再拉起 node 子进程,须整组回收,见 proc.rs)。
pub(crate) fn npm_command() -> Command {
    let mut cmd = Command::new(if cfg!(windows) { "cmd.exe" } else { "npm" });
    if cfg!(windows) {
        cmd.args(["/c", "npm.cmd"]);
    }
    no_window(&mut cmd);
    new_process_group(&mut cmd);
    cmd
}

/// 非零退出时的 stderr 细节拼接:空 stderr 返回空串,否则 "(stderr)" 括号包裹。
/// 单一事实来源,检测与安装共用。纯函数,可测。
fn exit_failure_detail(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr).trim().to_string();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("({stderr})")
    }
}

// ── node 检测 ──────────────────────────────────────────────────────

/// 校验 node 版本是否满足 dsh 要求(^22.19 || >=24)。纯函数,可测试。
/// 失败返回结构化 DshError(版本数据),文案模板在 locale JSON。
fn check_node_version(ver: &str) -> Result<(), DshError> {
    let v = ver.trim().trim_start_matches('v');
    let mut parts = v.split('.');
    let major: u32 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or(DshError::NodeVersionParseFailed {
            version: ver.to_string(),
        })?;
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ok = (major == 22 && minor >= 19) || major >= 24;
    if !ok {
        return Err(DshError::NodeVersionUnmet {
            current: ver.to_string(),
            required: NODE_REQ.to_string(),
        });
    }
    Ok(())
}

/// 检查 node 是否可用且满足版本要求。`node --version` 带超时:
/// node 被 shim/杀软/网络盘挂起时同步 output() 会永久阻塞;超时/IO 失败由
/// run_with_timeout 自动杀进程回收(执行器不变量,不泄漏孤儿)。
/// boot 流水线的 checking 阶段调用。
pub(crate) fn check_node() -> Result<String, DshError> {
    let mut binding = Command::new("node");
    let mut child = new_process_group(no_window(&mut binding))
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| DshError::NodeMissing {
            required: NODE_REQ.to_string(),
        })?;
    let out = run_with_timeout(&mut child, CHECK_NODE_TIMEOUT, |e| match e {
        crate::proc::RunError::Timeout(_) => DshError::NodeCheckTimeout {
            seconds: CHECK_NODE_TIMEOUT.as_secs(),
        },
        crate::proc::RunError::Io(detail) => DshError::NodeCheckFailed { detail },
    })?;
    if !out.status.success() {
        return Err(DshError::NodeVersionCheckFailed {
            exit_code: out.status.code().unwrap_or(-1),
            detail: exit_failure_detail(&out.stderr),
        });
    }
    let ver = String::from_utf8_lossy(&out.stdout).trim().to_string();
    check_node_version(&ver)?;
    Ok(ver)
}

// ── 全局 dsh 检测 ──────────────────────────────────────────────────

/// 全局 node_modules 路径,运行时动态解析(`npm root -g`)。
fn global_node_modules() -> Result<PathBuf, DshError> {
    let mut cmd = npm_command();
    cmd.args(["root", "-g"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|_| DshError::NpmRootSpawnFailed)?;
    let out = run_with_timeout(&mut child, NPM_ROOT_TIMEOUT, |e| match e {
        crate::proc::RunError::Timeout(_) => DshError::NpmRootTimeout {
            seconds: NPM_ROOT_TIMEOUT.as_secs(),
        },
        crate::proc::RunError::Io(detail) => DshError::NpmRootIoFailed { detail },
    })?;
    if !out.status.success() {
        return Err(DshError::NpmRootExitFailed {
            exit_code: out.status.code().unwrap_or(-1),
            detail: exit_failure_detail(&out.stderr),
        });
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return Err(DshError::NpmRootEmpty);
    }
    Ok(PathBuf::from(path))
}

/// 全局 dsh bin.js 路径 + 完整性校验(纯函数,可测试)。
/// 校验 bin.js 存在而非仅版本号——半残安装(版本号在、文件缺)不得视为「已安装」,
/// 否则坏掉的安装会被「有则用」跳过,永远修不好(2026-08-14 实测事故)。
fn dsh_bin_path(global_node_modules: &Path) -> Option<PathBuf> {
    let bin = global_node_modules.join("@deepseek-ai/dsh/lib/bin.js");
    bin.exists().then_some(bin)
}

/// 全局 dsh bin.js 路径(含完整性校验;升级链启动/恢复服务复用)。
pub(crate) fn global_dsh_bin() -> Option<PathBuf> {
    dsh_bin_path(&global_node_modules().ok()?)
}

/// 全局 dsh 已装版本:读 `{prefix}/node_modules/@deepseek-ai/dsh/package.json` 的
/// version 字段(#2 调研:比 npm ls -g 更轻,不受全局树损坏影响)。
/// 未安装 / 读取或解析失败 → None(检测按「无当前版本」处理,不报错)。
pub fn global_dsh_version() -> Option<String> {
    let pkg = global_node_modules()
        .ok()?
        .join("@deepseek-ai/dsh/package.json");
    let text = std::fs::read_to_string(pkg).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("version").and_then(|v| v.as_str()).map(String::from)
}

// ── 安装参数与错误分类 ─────────────────────────────────────────────

/// 安装包内置离线缓存目录(若存在)。
/// 约定:<资源目录>/npm-cache,内含 npm cacache。缓存存在性以 cacache 内部
/// 结构为标记:`_cacache/index-v5` 与 `_cacache/content-v2` 两个目录都在才算。
/// 刻意用 cacache 内部结构而非 npm 顶层 `index-v5` 元数据索引:npm 10.9+ 起
/// 不再写顶层 index-v5(元数据并入 _cacache),而 `_cacache` 布局在 npm 7-12
/// 全版本稳定(2026-08-14 实测 npm 10.9.7 生成的缓存;旧标记会把 #6 打包的
/// 缓存漏判为不存在,离线安装静默失效)。
/// 空目录 / 打包遗漏时不满足标记 → 不算缓存,回退网络安装。
pub(crate) fn bundle_cache_dir(resource_dir: &Path) -> Option<PathBuf> {
    let dir = resource_dir.join(BUNDLE_CACHE_REL);
    if dir.join("_cacache/index-v5").is_dir() && dir.join("_cacache/content-v2").is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// npm 全局安装参数组装(纯函数,可测试)。
/// `version_spec`:目标版本规格(boot 传 "@latest",升级链传 "@<pin>",#3 §7)。
/// 离线缓存目录存在时加 `--prefer-offline --cache <目录>`:
/// 缓存命中走本地秒级完成,缺失自动回退网络(用户拍板语义)。
pub(crate) fn npm_install_args(offline_cache: Option<&Path>, version_spec: &str) -> Vec<String> {
    let mut args = vec!["install".to_string(), "-g".to_string()];
    if let Some(dir) = offline_cache {
        args.push("--prefer-offline".into());
        args.push("--cache".into());
        args.push(dir.to_string_lossy().into_owned());
    }
    args.push(format!("@deepseek-ai/dsh{version_spec}"));
    args.extend([
        "--no-audit".into(),
        "--no-fund".into(),
        "--no-progress".into(),
    ]);
    args
}

/// 安装失败的结构化错误(纯函数,可测试)。
/// stderr_tail 是安装期间 stderr 的最后几行:EPERM/EACCES/权限类错误给出
/// 可操作引导(管理员重试 / nvm 用户目录安装 / 手动命令),其余给网络引导。
/// 引导措辞是文案模板,归 locale JSON(errors.InstallFailedPermission/Network);
/// 本函数只产出结构化判别(kind 区分 权限×有无退出码),数据带退出码与 stderr 原文。
/// 结论写进正文:调研实测(#2)确认 npm 失败会保留旧版,失败重试即自愈。
pub(crate) fn install_failure_error(exit_code: Option<i32>, stderr_tail: &[String]) -> DshError {
    // 权限判定用原始行(截断可能切掉行尾的权限标记);文案引导随后按 kind 进 locale JSON
    let is_permission = stderr_tail_has_permission(stderr_tail);
    let stderr_tail = format_stderr_tail(stderr_tail);
    match (is_permission, exit_code) {
        (true, Some(code)) => DshError::InstallFailedPermission {
            exit_code: code,
            stderr_tail,
        },
        (true, None) => DshError::InstallFailedPermissionAbnormal { stderr_tail },
        (false, Some(code)) => DshError::InstallFailedNetwork {
            exit_code: code,
            stderr_tail,
        },
        (false, None) => DshError::InstallFailedNetworkAbnormal { stderr_tail },
    }
}

/// 从 stderr 尾部提取最多 2 行、每行截断到 120 字符,用 "; " 连接;
/// 非空时以 "; " 收尾作模板间分隔符。纯函数,可测试。
fn format_stderr_tail(stderr_tail: &[String]) -> String {
    let mut detail = String::new();
    for l in stderr_tail
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .take(2)
    {
        if l.chars().count() > 120 {
            detail.push_str(&l.chars().take(120).collect::<String>());
            detail.push('…');
        } else {
            detail.push_str(l);
        }
        detail.push_str("; ");
    }
    detail
}

/// 判定 stderr 尾部是否权限类错误(EPERM/EACCES/…)。纯函数,可测试。
fn stderr_tail_has_permission(stderr_tail: &[String]) -> bool {
    stderr_tail.iter().any(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("eperm")
            || l.contains("eacces")
            || l.contains("permission denied")
            || l.contains("lack permission")
    })
}

/// 判定 stderr 尾部是否 npm ETARGET 错误(纯函数,可测试)。
/// ETARGET = npm 按手头 packument 判定「请求的版本不存在」
/// ("No matching version found for <pkg>@<ver>",code ETARGET/notarget)。
/// 在升级链场景这几乎必然是内置离线缓存 packument 早于目标版本造成的
/// 假阴性(registry 直查刚确认过该版本存在)——调用方据此回退无缓存重试
/// (根治方案,见 dsh.rs npm_install_global 的 ETARGET 注释)。
pub(crate) fn stderr_has_etarget(stderr_tail: &[String]) -> bool {
    stderr_tail.iter().any(|l| {
        let l = l.to_ascii_lowercase();
        l.contains("etarget") || l.contains("notarget")
    })
}

// ── 安装执行(boot 与升级链共用,单一事实来源)──────────────────────

/// 安装过程观察者:安装流程中与调用方状态相关的两个副作用经此 seam 注入,
/// 安装本体(命令构造/超时/孤儿回收/ETARGET 回退/错误分类)不依赖具体调用方。
/// 生产实现:DshManager(pid 供退出收敛随树杀;日志入环形缓冲)。
pub(crate) trait InstallObserver: Send + 'static {
    /// 登记安装中进程 pid(Some;退出收敛时按树杀, npm 会再拉起 node 子进程) /
    /// 清除登记(None;进程已退出,自然退出或被超时回收)。
    fn install_pid(&self, pid: Option<u32>);
    /// 转发安装过程日志行(stream = "stdout" | "stderr")。
    fn install_log(&self, stream: &str, line: &str);
}

/// 单次 npm 安装尝试的结果(ETARGET 回退的判定数据源,见 install_global)。
enum InstallAttemptFailure {
    /// 进程非零退出:未分类,携带退出码与 stderr 尾部原文(供 ETARGET 判定与
    /// install_failure_error 分类);安装失败保留旧版(npm 语义,#2 实测)
    Exit {
        exit_code: Option<i32>,
        stderr_tail: Vec<String>,
    },
    /// 超时 / 句柄异常:已分类 DshError(非「缓存数据过旧」症状,不做重试)
    Dsh(DshError),
}

/// 执行一次 npm install 尝试(boot 与升级链共用;ETARGET 回退编排在
/// install_global)。stdout/stderr 逐行转发给观察者,stderr 尾部捕获供失败
/// 分类;pid 经观察者登记:升级期间用户退出时随退出收敛一并杀。
/// 超时 / 句柄异常由 run_with_timeout 自动按进程树杀回收(执行器不变量,
/// 不泄漏孤儿)。
fn install_attempt<O>(
    obs: &O,
    cache_dir: Option<&Path>,
    version_spec: &str,
) -> Result<(), InstallAttemptFailure>
where
    O: InstallObserver + Clone,
{
    if let Some(dir) = cache_dir {
        log::info!("[npm] 使用安装包内置离线缓存: {}", dir.display());
    }
    let args = npm_install_args(cache_dir, version_spec);
    let mut cmd = npm_command();
    cmd.args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        InstallAttemptFailure::Dsh(DshError::NpmSpawnFailed { detail: e.to_string() })
    })?;
    obs.install_pid(Some(child.id()));

    let stdout = child.stdout.take().expect("piped stdout");
    let o = obs.clone();
    let out_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            o.install_log("stdout", &line);
        }
    });

    let stderr = child.stderr.take().expect("piped stderr");
    let o = obs.clone();
    // stderr 尾部捕获:失败时拼进可读错误信息(EPERM 权限引导等),与日志流并行
    let stderr_tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let tail = stderr_tail.clone();
    let err_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            o.install_log("stderr", &line);
            let mut t = tail.lock().unwrap_or_else(|p| p.into_inner());
            t.push(line);
            while t.len() > 8 {
                t.remove(0);
            }
        }
    });

    let result = run_with_timeout(&mut child, NPM_INSTALL_TIMEOUT, |e| {
        InstallAttemptFailure::Dsh(match e {
            crate::proc::RunError::Timeout(_) => DshError::InstallTimeout {
                seconds: NPM_INSTALL_TIMEOUT.as_secs(),
            },
            crate::proc::RunError::Io(detail) => DshError::NpmInstallIoFailed { detail },
        })
    });
    obs.install_pid(None);
    // 无论成败进程已退出(自然退出 / 超时被杀回收)→ 管道 EOF → 读线程自然
    // 结束,join 防线程泄漏;stderr 尾部在 err_thread 结束前收全
    let _ = out_thread.join();
    let _ = err_thread.join();
    match result {
        Ok(out) => {
            let tail: Vec<String> = stderr_tail
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone();
            if out.status.success() {
                Ok(())
            } else {
                Err(InstallAttemptFailure::Exit {
                    exit_code: out.status.code(),
                    stderr_tail: tail,
                })
            }
        }
        Err(e) => Err(e),
    }
}

/// ETARGET 回退判定(纯函数,可测):带缓存 + stderr 报 ETARGET = 缓存
/// packument 过旧的确定性症状(数据过旧是唯一成因),回退无缓存重试;
/// 其余失败直接分类返回。
fn should_retry_without_cache(used_cache: bool, stderr_tail: &[String]) -> bool {
    used_cache && stderr_has_etarget(stderr_tail)
}

/// npm 全局安装 dsh(boot 传 "@latest" 跟随 latest;dsh 升级链传 "@<pin>" 精确
/// 版本,#3 §7),stdout/stderr 逐行转发给观察者。安装包内置离线缓存存在时优先
/// 离线安装(命中秒级完成、缺失回退网络)。
///
/// ETARGET 根治(#41,M1 遗留):内置离线缓存是发版时点打包的,其 packument
/// 可能早于升级目标版本(registry 直查刚确认过该版本存在),--prefer-offline
/// 跳过新鲜度检查会让 npm 按旧 packument 误报「版本不存在」(ETARGET,假阴性)。
/// 处置:带缓存安装失败且 stderr 判定为 ETARGET → 回退无缓存网络重试一次
/// (registry 数据恒新鲜,命中真因——数据过旧——而非擦症状);其余失败直接
/// 分类返回。boot 的 @latest 安装不需要比缓存更新的版本,不会命中 ETARGET,
/// 此回退只会在升级链触发;回退后再次失败按常规错误呈现(不循环重试)。
pub(crate) fn install_global<O>(
    obs: &O,
    cache_dir: Option<&Path>,
    version_spec: &str,
) -> Result<(), DshError>
where
    O: InstallObserver + Clone,
{
    let mut cache = cache_dir;
    loop {
        match install_attempt(obs, cache, version_spec) {
            Ok(()) => return Ok(()),
            Err(InstallAttemptFailure::Dsh(e)) => return Err(e),
            Err(InstallAttemptFailure::Exit { exit_code, stderr_tail }) => {
                // 命中即回退无缓存(cache 置 None 后循环至多再走一轮:
                // ETARGET 分支条件不再成立,不会无限重试)
                if should_retry_without_cache(cache.is_some(), &stderr_tail) {
                    log::warn!(
                        "[npm] 安装报 ETARGET(内置缓存 packument 早于目标版本),回退无缓存网络重试"
                    );
                    cache = None;
                    continue;
                }
                return Err(install_failure_error(exit_code, &stderr_tail));
            }
        }
    }
}

// ── 安装进度模拟(#7,单一事实来源:boot 与升级链共用)────────────────

/// 安装模拟进度的子阶段。npm 安装期**没有真实百分比**(管道非 TTY + `--no-progress`,
/// 输出块缓冲突发到达,调研 #2 实测),本枚举是时间驱动的语义分段,供进度文案与
/// 事件携带;boot 安装与 dsh 升级链复用同一模拟逻辑(install_progress_at)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum InstallStage {
    /// 下载依赖
    Fetching,
    /// 依赖解包写入安装目录
    Reifying,
    /// 收尾(接近完成)
    Finishing,
}

/// 按真实流逝时间给出安装模拟进度(纯函数,可测试)。
///
/// 分段锚点(实测 #2:暖缓存 ~26s、冷缓存 ~4m16s、离线缓存命中秒级):
/// - 0-10s 下载(0% → 60%)——网络为主,占大头
/// - 10-60s 解包写入(60% → 85%)
/// - 60-120s 收尾(85% → 99%),之后停在 99%
///
/// 连续(拐点处百分比相等)、单调不减、**永不提前到 100%**——100% 只能由 npm
/// 进程退出校准(锚点语义,见 dsh.rs boot_pipeline 的 installing 分支)。模拟只做
/// 视觉呈现,不参与任何业务决策(成功/失败/超时全由真实进程事件驱动)。
pub(crate) fn install_progress_at(elapsed_secs: f64) -> (InstallStage, u8) {
    let t = elapsed_secs.max(0.0);
    // 分段区间内线性插值:拐点处百分比相等,推进平滑无跳变
    let (stage, pct) = if t < 10.0 {
        (InstallStage::Fetching, t / 10.0 * 60.0) // 0-10s:0% → 60%
    } else if t < 60.0 {
        (InstallStage::Reifying, 60.0 + (t - 10.0) * 0.5) // 10-60s:60% → 85%
    } else {
        // 60-120s:85% → 99%;之后封顶 99%
        (InstallStage::Finishing, 85.0 + (t - 60.0).min(60.0) * (14.0 / 60.0))
    };
    (stage, pct.round().clamp(0.0, 99.0) as u8)
}

/// 安装模拟进度线程(boot 安装与 dsh 升级链共用,#7/#17 单一事实来源):
/// 每 500ms 按真实流逝时间推进一次(install_progress_at),百分比变化才回调
/// (安装期事件量 ≈ 200 发/分钟,量级与阶段事件一致);事件去向由调用方决定
/// (boot → boot-state,升级 → upgrade-state)。
///
/// 生命周期契约:调用方必须先 stop_and_join() 再发终态事件(100% 校准 / 错误)
/// ——保证线程在途事件先于终态事件送达,事件流确定性收尾(否则旧进度事件可能
/// 晚于错误事件到达,把前端从错误页拉回 installing 卡死)。stop 置位后线程
/// 最多 500ms 内退出,join 等待不可感知。
///
/// 注意:stop 句柄必须由本结构持有、随实例走——早期实现把 stop 造在线程内部
/// 导致无人能置位,安装路径下 join() 永久挂起(boot 的「全局无 dsh 时安装」
/// 路径从未实机跑过而未暴露,本次升级链落地排查发现并修复)。
pub(crate) struct ProgressTicker {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

impl ProgressTicker {
    /// 启动进度线程:时间驱动推进(install_progress_at),百分比变化才回调。
    pub(crate) fn start<F>(on_progress: F) -> Self
    where
        F: Fn(InstallStage, u8) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        let handle = thread::spawn(move || {
            let started = Instant::now();
            let mut last_pct: Option<u8> = None;
            while !s.load(Ordering::SeqCst) {
                let (stage, pct) = install_progress_at(started.elapsed().as_secs_f64());
                if last_pct != Some(pct) {
                    last_pct = Some(pct);
                    on_progress(stage, pct);
                }
                thread::sleep(Duration::from_millis(500));
            }
        });
        Self { stop, handle }
    }

    /// 停表并 join(调用方发终态事件前必须调用,见结构文档)。
    pub(crate) fn stop_and_join(self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.handle.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_node_version_accepts_required_ranges() {
        assert!(check_node_version("v22.22.2").is_ok());
        assert!(check_node_version("v22.19.0").is_ok());
        assert!(check_node_version("v24.0.0").is_ok());
        assert!(check_node_version("v25.1.0").is_ok());
        assert!(check_node_version("22.22.2").is_ok()); // 无 v 前缀
    }

    #[test]
    fn check_node_version_rejects_others() {
        // 22 系但 < 19 → 版本不满足(带当前版本与要求,供 locale 模板插值)
        assert!(matches!(
            check_node_version("v22.18.0"),
            Err(DshError::NodeVersionUnmet { current, required })
                if current == "v22.18.0" && required == NODE_REQ
        ));
        // 23 不在 ^22.19 || >=24 内
        assert!(matches!(
            check_node_version("v23.0.0"),
            Err(DshError::NodeVersionUnmet { .. })
        ));
        assert!(matches!(
            check_node_version("v20.10.0"),
            Err(DshError::NodeVersionUnmet { .. })
        ));
        // 不可解析的版本号 → 解析失败
        assert!(matches!(
            check_node_version("not-a-version"),
            Err(DshError::NodeVersionParseFailed { .. })
        ));
    }

    #[test]
    fn check_node_version_handles_short_forms_and_whitespace() {
        // 缺段容错(minor 缺省按 0):^22.19 边界在 minor 上,22.x 缺 minor 视为 22.0
        assert!(matches!(
            check_node_version("v22"),
            Err(DshError::NodeVersionUnmet { .. })
        ));
        assert!(check_node_version("v22.19").is_ok()); // 22.19 无 patch
        assert!(check_node_version("24").is_ok()); // >=24 无 minor
        assert!(check_node_version("v24").is_ok());
        // 首尾空白(检查输出 / 管道可能带换行,调用方先 trim 再进比较,单测守住边界)
        assert!(check_node_version("  v22.22.2  ").is_ok());
    }

    #[test]
    fn dsh_bin_path_requires_bin_js_not_just_version() {
        // 半残安装(版本号在、bin.js 缺)不得视为「已安装」——否则被「有则用」跳过,
        // 坏掉的安装永远修不好(2026-08-14 实测事故)
        let dir = std::env::temp_dir().join(format!("dsh-boot-test-{}", std::process::id()));
        let dsh_dir = dir.join("@deepseek-ai/dsh");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dsh_dir.join("lib")).unwrap();

        // 只有 package.json(无 bin.js)→ 视为未安装
        std::fs::write(dsh_dir.join("package.json"), r#"{"version":"0.1.0-rc.6"}"#).unwrap();
        assert_eq!(
            dsh_bin_path(&dir),
            None,
            "版本号存在但 bin.js 缺失必须判为未安装"
        );

        // 补上 bin.js → 视为已安装
        std::fs::write(dsh_dir.join("lib/bin.js"), "// placeholder").unwrap();
        assert_eq!(
            dsh_bin_path(&dir),
            Some(dsh_dir.join("lib/bin.js")),
            "bin.js 存在即视为已安装(不比较版本)"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn npm_cmd_spawns_via_cmd_exe() {
        // 生产路径:cmd.exe /c npm.cmd —— CreateProcess 不能直接执行 .cmd
        let out = Command::new("cmd.exe")
            .args(["/c", "npm.cmd", "--version"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "npm.cmd 经 cmd.exe 启动失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!String::from_utf8_lossy(&out.stdout).trim().is_empty());
    }

    #[test]
    fn npm_install_args_without_bundle_cache_uses_plain_latest() {
        // 无离线缓存:普通网络安装(生产路径参数必须逐字一致)
        assert_eq!(
            npm_install_args(None, "@latest"),
            vec![
                "install",
                "-g",
                "@deepseek-ai/dsh@latest",
                "--no-audit",
                "--no-fund",
                "--no-progress"
            ]
        );
    }

    #[test]
    fn npm_install_args_with_bundle_cache_prefers_offline() {
        // 有离线缓存:加 --prefer-offline --cache <目录>(缓存命中秒级、缺失回退网络)
        let cache = Path::new("C:/app resources/npm-cache");
        assert_eq!(
            npm_install_args(Some(cache), "@latest"),
            vec![
                "install",
                "-g",
                "--prefer-offline",
                "--cache",
                "C:/app resources/npm-cache",
                "@deepseek-ai/dsh@latest",
                "--no-audit",
                "--no-fund",
                "--no-progress"
            ]
        );
    }

    #[test]
    fn npm_install_args_pins_version_for_upgrade() {
        // 升级链传精确 pin(#3 §7):`npm install -g @deepseek-ai/dsh@<pin>`,
        // 不裸用 @latest;其余参数与 boot 完全一致(同一 npm 机制)
        assert_eq!(
            npm_install_args(None, "@0.1.0-rc.6"),
            vec![
                "install",
                "-g",
                "@deepseek-ai/dsh@0.1.0-rc.6",
                "--no-audit",
                "--no-fund",
                "--no-progress"
            ]
        );
    }

    #[test]
    fn bundle_cache_dir_detects_real_cacache() {
        // 生产路径:resource_dir() 下 <npm-cache> 目录;空目录不得误判为离线缓存
        let dir = std::env::temp_dir().join(format!("dsh-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // 目录不存在 → None(开发态资源目录里没有 npm-cache,走网络安装)
        assert_eq!(bundle_cache_dir(&dir), None);
        // 空目录不算缓存(打包遗漏时不得误判离线)
        std::fs::create_dir_all(dir.join("npm-cache")).unwrap();
        assert_eq!(bundle_cache_dir(&dir), None);
        // 只有 npm 顶层 index-v5(旧 npm ≤10.8 元数据索引)也不算 ——
        // 标记看 cacache 内部结构,不是 npm 顶层目录
        std::fs::create_dir_all(dir.join("npm-cache/index-v5")).unwrap();
        assert_eq!(bundle_cache_dir(&dir), None);
        // 带 cacache 内部标记(_cacache/index-v5 + _cacache/content-v2)→ 视为存在
        std::fs::create_dir_all(dir.join("npm-cache/_cacache/index-v5")).unwrap();
        std::fs::create_dir_all(dir.join("npm-cache/_cacache/content-v2")).unwrap();
        assert_eq!(bundle_cache_dir(&dir), Some(dir.join("npm-cache")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_failure_error_guides_on_permission_error() {
        // 权限失败(npm 实测报错形态:EPERM + "you lack permissions to access it")
        let tail = vec![
            "npm error code EPERM".to_string(),
            "npm error The operation was rejected by your operating system.".to_string(),
            "npm error you lack permissions to access it".to_string(),
        ];
        assert_eq!(
            install_failure_error(Some(243), &tail),
            DshError::InstallFailedPermission {
                exit_code: 243,
                stderr_tail:
                    "npm error code EPERM; npm error The operation was rejected by your operating system.; "
                        .to_string(),
            }
        );

        // EACCES 同理,无退出码 → Abnormal 变体
        assert_eq!(
            install_failure_error(None, &["npm error EACCES: permission denied".into()]),
            DshError::InstallFailedPermissionAbnormal {
                stderr_tail: "npm error EACCES: permission denied; ".to_string(),
            }
        );
    }

    #[test]
    fn install_failure_error_falls_back_to_network_variant() {
        // 非权限类失败 → Network 变体(网络引导文案模板在 locale JSON)
        assert_eq!(
            install_failure_error(Some(1), &["npm error code ENOTFOUND".into()]),
            DshError::InstallFailedNetwork {
                exit_code: 1,
                stderr_tail: "npm error code ENOTFOUND; ".to_string(),
            }
        );

        // 无 stderr 输出:stderr_tail 为空串,模板可干净衔接引导语
        assert_eq!(
            install_failure_error(Some(1), &[]),
            DshError::InstallFailedNetwork {
                exit_code: 1,
                stderr_tail: String::new(),
            }
        );
    }

    #[test]
    fn stderr_has_etarget_detects_npm_notarget_output() {
        // npm 实测 ETARGET 输出形态:code ETARGET + notarget 解释行
        let tail = vec![
            "npm error code ETARGET".to_string(),
            "npm error notarget No matching version found for @deepseek-ai/dsh@0.1.0-rc.7.".to_string(),
            "npm error notarget In most cases you or one of your dependencies are requesting".to_string(),
        ];
        assert!(stderr_has_etarget(&tail));
        // 单行命中(截断只留尾部时的形态)
        assert!(stderr_has_etarget(&["npm error notarget No matching version found for @deepseek-ai/dsh@0.1.0-rc.7.".to_string()]));
        assert!(stderr_has_etarget(&["npm error code ETARGET".to_string()]));
        // 大小写变体
        assert!(stderr_has_etarget(&["npm error code etarget".to_string()]));
    }

    #[test]
    fn stderr_has_etarget_ignores_other_failures() {
        // 权限 / 网络 / 空输出都不是 ETARGET(不回退重试)
        assert!(!stderr_has_etarget(&["npm error code EPERM".to_string()]));
        assert!(!stderr_has_etarget(&["npm error code ENOTFOUND".to_string()]));
        assert!(!stderr_has_etarget(&["npm error code EACCES".to_string()]));
        assert!(!stderr_has_etarget(&[]));
        assert!(!stderr_has_etarget(&["npm error code ETIMEDOUT".to_string()])); // 与 ETARGET 不同码
    }

    #[test]
    fn should_retry_without_cache_requires_cache_and_etarget() {
        // ETARGET 回退判定(生产路径见 install_global):
        // 带缓存 + ETARGET → 回退;缺任一条件 → 直接分类返回
        let etarget = vec!["npm error code ETARGET".to_string()];
        let network = vec!["npm error code ENOTFOUND".to_string()];
        assert!(should_retry_without_cache(true, &etarget));
        assert!(!should_retry_without_cache(false, &etarget)); // 没带缓存,无从回退
        assert!(!should_retry_without_cache(true, &network)); // 非 ETARGET 不回退
        assert!(!should_retry_without_cache(true, &[]));
    }

    #[test]
    fn format_stderr_tail_truncates_long_lines() {
        // 生产路径:每行截断到 120 字符 + 省略号,最多 2 行,'; ' 连接
        let out = format_stderr_tail(&["x".repeat(200), "second".to_string()]);
        assert!(out.starts_with(&"x".repeat(120)), "{out}");
        assert!(out.contains('…'));
        assert!(out.ends_with("second; "));
    }

    #[test]
    fn format_stderr_tail_skips_blank_lines() {
        assert_eq!(format_stderr_tail(&[]), "");
        assert_eq!(format_stderr_tail(&["   ".to_string(), "ok".to_string()]), "ok; ");
    }

    #[test]
    fn exit_failure_detail_wraps_non_empty_stderr() {
        assert_eq!(exit_failure_detail(b""), "");
        assert_eq!(exit_failure_detail(b"   \n"), ""); // 纯空白
        assert_eq!(exit_failure_detail(b"npm error code EPERM\n"), "(npm error code EPERM)");
        assert_eq!(exit_failure_detail(b"  boom  "), "(boom)"); // 去首尾空白
    }

    #[cfg(windows)]
    #[test]
    fn npm_cache_arg_with_spaces_survives_cmd_exe() {
        // 生产路径:离线安装要把含空格的 --cache 路径经 cmd.exe /c 原样传给 npm.cmd。
        // npm config get 不回源网络,验证 Rust 自动加引号的参数不被 cmd 拆坏。
        let cache = "C:/spaced cache dir/npm-cache";
        let out = Command::new("cmd.exe")
            .args(["/c", "npm.cmd", "config", "get", "cache", "--cache", cache])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "npm 执行失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("spaced cache dir"),
            "含空格 cache 路径被 cmd 拆坏: {stdout}"
        );
    }

    // ── 安装进度模拟(#7)──────────────────────────────────────────

    #[test]
    fn install_progress_anchors_match_design() {
        // 生产路径:进度线程每 500ms 调 install_progress_at 一次,
        // 阶段与百分比驱动前端进度条与阶段文案。锚点必须与设计一致。
        assert_eq!(install_progress_at(0.0), (InstallStage::Fetching, 0));
        assert_eq!(install_progress_at(5.0), (InstallStage::Fetching, 30));
        assert_eq!(install_progress_at(10.0), (InstallStage::Reifying, 60)); // 拐点连续
        assert_eq!(install_progress_at(35.0), (InstallStage::Reifying, 73)); // 60 + 25*0.5
        assert_eq!(install_progress_at(60.0), (InstallStage::Finishing, 85)); // 拐点连续
        assert_eq!(install_progress_at(120.0), (InstallStage::Finishing, 99));
        assert_eq!(install_progress_at(600.0), (InstallStage::Finishing, 99)); // 封顶
    }

    #[test]
    fn install_progress_is_monotonic_and_never_reaches_100() {
        // 模拟进度单调不减、永不提前 100%(100% 只能由 npm 进程退出校准):
        // 失败/超时路径不会出现「已 100% 却失败」的矛盾呈现
        let mut prev = 0u8;
        for secs in (0..300).map(|s| s as f64 + 0.1) {
            let (_, pct) = install_progress_at(secs);
            assert!(pct >= prev, "进度回退: {secs}s → {pct} < {prev}");
            assert!(pct < 100, "模拟进度提前到 100%: {secs}s");
            prev = pct;
        }
    }

    #[test]
    fn install_progress_offline_cache_fast_path_stays_low() {
        // 离线缓存命中秒级完成(#16):安装开始即校准 100%,模拟值短暂出现且远低于
        // 100%——快路径下进度条从 0 快速跳到 100 是预期行为,不得到处乱跳
        let (stage, pct) = install_progress_at(1.5);
        assert_eq!(stage, InstallStage::Fetching);
        assert!(pct < 20, "离线快路径下 1.5s 进度应很低,实际 {pct}");
        let (_, pct) = install_progress_at(3.0);
        assert!(pct < 30, "离线快路径下 3s 进度应较低,实际 {pct}");
    }

    #[test]
    fn install_progress_clamps_negative_input() {
        // 防御:负流逝时间(时钟异常)按 0 处理
        assert_eq!(install_progress_at(-1.0), (InstallStage::Fetching, 0));
    }
}
