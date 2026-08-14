//! 子进程工具:带超时收割、Windows 隐藏控制台、按进程树杀。
//! boot 流水线、npm 安装域、退出收敛共用(单一事实来源)。

use std::process::{Child, Command, Output};
use std::time::{Duration, Instant};

/// 带超时的子进程收割:轮询 try_wait 直到退出或超时。
/// **返回 Timeout 时子进程仍在运行,由调用方负责终止**(按进程树杀,见 kill_pid_tree)。
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChildWaitError {
    Timeout(Duration),
    Io(String),
}

pub(crate) fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<Output, ChildWaitError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // 进程已退出:收集 stdout/stderr(调用方未 take 的部分)
                use std::io::Read;
                let mut stdout = Vec::new();
                let mut stderr = Vec::new();
                if let Some(mut so) = child.stdout.take() {
                    let _ = so.read_to_end(&mut stdout);
                }
                if let Some(mut se) = child.stderr.take() {
                    let _ = se.read_to_end(&mut stderr);
                }
                return Ok(Output {
                    status,
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    return Err(ChildWaitError::Timeout(timeout));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(ChildWaitError::Io(e.to_string())),
        }
    }
}

/// Windows 上隐藏子进程控制台窗口:GUI 应用(无控制台)直接 spawn node/npm 会闪
/// 一个 console 窗口。CREATE_NO_WINDOW = 0x08000000。
#[cfg(windows)]
pub(crate) fn no_window(cmd: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x0800_0000)
}

#[cfg(not(windows))]
pub(crate) fn no_window(cmd: &mut Command) -> &mut Command {
    cmd
}

/// 按进程树杀:Windows 用 taskkill /T /F(CreateProcess 只杀直接子进程,node 拉起的
/// 孙进程会成孤儿);Unix 用 kill 命令。幂等:进程已退出时静默失败。
pub(crate) fn kill_pid_tree(pid: u32) {
    #[cfg(windows)]
    {
        let mut binding = Command::new("taskkill");
        let _ = no_window(&mut binding)
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}

/// 按进程树杀并回收(等待退出)。**Timeout 与 Io 路径必须调用**——只 return 不杀
/// 会留下孤儿进程:npm 安装的 Io 路径还曾因 install_pid 已清除、退出收敛也杀不到。
pub(crate) fn kill_and_reap(child: &mut Child) {
    kill_pid_tree(child.id());
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    #[cfg(windows)]
    #[test]
    fn wait_with_timeout_collects_output_of_quick_command() {
        let mut child = Command::new("cmd")
            .args(["/c", "echo ok"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let out = wait_with_timeout(&mut child, Duration::from_secs(5)).unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
    }

    #[cfg(windows)]
    #[test]
    fn wait_with_timeout_times_out_and_caller_kills() {
        // ping -n 3 ≈ 2s,超时 200ms;返回 Timeout 后按进程树杀
        let mut child = Command::new("cmd")
            .args(["/c", "ping -n 3 127.0.0.1 >nul"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let r = wait_with_timeout(&mut child, Duration::from_millis(200));
        assert!(
            matches!(r, Err(ChildWaitError::Timeout(_))),
            "期望 Timeout,实际 {r:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2), "超时检测过慢");
        kill_pid_tree(child.id());
        let _ = child.wait();
    }
}
