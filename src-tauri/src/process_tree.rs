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

    // agent_chat 启动时把 shell 设为独立进程组组长；向负 PID 发信号会连同后代一起终止。
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
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
