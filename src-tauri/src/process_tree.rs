/// 终止应用托管的完整进程树。agent CLI 在 shell/Node 包装层下启动，
/// 只 kill 直接 child 会把真正的 codex/claude 进程遗留在后台。
#[cfg(target_os = "windows")]
fn job_handle() -> Result<windows_sys::Win32::Foundation::HANDLE, String> {
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::{
        CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    static JOB: OnceLock<Result<isize, String>> = OnceLock::new();
    let result = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(format!(
                "CreateJobObjectW failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&info) as u32,
        ) == 0
        {
            let error = std::io::Error::last_os_error();
            CloseHandle(job);
            return Err(format!("SetInformationJobObject failed: {error}"));
        }
        Ok(job as isize)
    });
    result
        .as_ref()
        .map(|handle| *handle as windows_sys::Win32::Foundation::HANDLE)
        .map_err(Clone::clone)
}

/// 把刚创建的直接 child 纳入应用 Job。应用被任务管理器强杀或崩溃时，Windows
/// 会关闭 Job handle，并由 `KILL_ON_JOB_CLOSE` 终止其中的整个后代树。
#[cfg(target_os = "windows")]
pub fn register(pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
    };

    if pid == 0 {
        return Err("Cannot register process 0".to_string());
    }
    let job = job_handle()?;
    unsafe {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process.is_null() {
            return Err(format!(
                "OpenProcess({pid}) failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let assigned = AssignProcessToJobObject(job, process);
        let error = if assigned == 0 {
            Some(std::io::Error::last_os_error())
        } else {
            None
        };
        CloseHandle(process);
        match error {
            Some(error) => Err(format!("AssignProcessToJobObject({pid}) failed: {error}")),
            None => Ok(()),
        }
    }
}

#[cfg(unix)]
pub fn register(_pid: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn terminate(pid: u32) {
    if pid == 0 {
        return;
    }

    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let pid = pid.to_string();
    let _ = Command::new("taskkill")
        .args(["/PID", pid.as_str(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

#[cfg(unix)]
pub fn terminate(pid: u32) {
    if pid == 0 {
        return;
    }

    // 大多数 agent CLI 位于独立进程组，向负 PID 发信号可一次结束完整进程树。Codex
    // app-server 在 macOS 的 login shell 中不能独立成组（会卡住 initialize），因此再按
    // PPID 递归补杀后代，防止外层 shell 已死、内部 writer 却继续占着同一 thread。
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
    for child in unix_descendants(pid).into_iter().rev() {
        unsafe {
            libc::kill(child as i32, libc::SIGKILL);
        }
    }
    unsafe {
        libc::kill(pid as i32, libc::SIGKILL);
    }
}

/// 读取当前父子关系并返回 root 的全部后代（父在前、子在后）。`ps` 是 macOS 与 Linux
/// 都有的系统工具；读取失败时自然回退为上面的进程组 / 直接 PID 终止。
#[cfg(unix)]
fn unix_descendants(root: u32) -> Vec<u32> {
    use std::collections::HashMap;
    use std::process::{Command, Stdio};

    let Ok(output) = Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let (Some(pid), Some(parent)) = (fields.next(), fields.next()) else {
            continue;
        };
        let (Ok(pid), Ok(parent)) = (pid.parse::<u32>(), parent.parse::<u32>()) else {
            continue;
        };
        children.entry(parent).or_default().push(pid);
    }

    let mut descendants = Vec::new();
    let mut pending = vec![root];
    while let Some(parent) = pending.pop() {
        if let Some(direct_children) = children.get(&parent) {
            for &child in direct_children {
                descendants.push(child);
                pending.push(child);
            }
        }
    }
    descendants
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn spawned_child_can_join_the_kill_on_close_job() {
        use std::os::windows::process::CommandExt;

        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "ping -n 30 127.0.0.1 >NUL"])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .unwrap();

        register(child.id()).unwrap();
        terminate(child.id());
        let _ = child.kill();
        child.wait().unwrap();
    }
}

#[cfg(all(test, unix))]
mod unix_tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn discovers_and_terminates_descendants_without_a_process_group() {
        // 模拟 Codex app-server 的 shell 包装层：外层 shell 与其 child 共用父进程组，
        // 因而单靠 kill(-pid) 不会命中它们。
        let mut shell = Command::new("sh")
            .args(["-c", "sleep 30 & wait"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let mut descendants = Vec::new();
        for _ in 0..20 {
            descendants = unix_descendants(shell.id());
            if !descendants.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!descendants.is_empty());

        terminate(shell.id());
        let _ = shell.wait();
    }
}
