use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::{
    Foundation::{ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0},
    Security::{EqualSid, GetTokenInformation, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetCurrentProcess, GetCurrentProcessId, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
            QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
        },
    },
};

pub const MIB: usize = 1024 * 1024;

#[derive(Clone, Copy)]
pub enum Scope {
    All,
    Node,
    Python,
}

impl Scope {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Node.js or Python",
            Self::Node => "Node.js",
            Self::Python => "Python",
        }
    }

    fn matches(self, name: &str) -> bool {
        let is_node = name.eq_ignore_ascii_case("node.exe");
        runtime_name(name)
            && match self {
                Self::All => true,
                Self::Node => is_node,
                Self::Python => !is_node,
            }
    }
}

pub struct Target {
    handle: OwnedHandle,
    pub name: String,
    pub pid: u32,
    pub working_set: usize,
}

impl Target {
    pub fn stop(&self) -> bool {
        let handle = self.handle.as_raw_handle();
        unsafe {
            WaitForSingleObject(handle, 0) == WAIT_OBJECT_0
                || (TerminateProcess(handle, 1) != 0
                    && WaitForSingleObject(handle, 500) == WAIT_OBJECT_0)
        }
    }
}

fn runtime_name(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if name == "node.exe" {
        return true;
    }
    let Some(version) = name
        .strip_prefix("python")
        .and_then(|n| n.strip_suffix(".exe"))
    else {
        return false;
    };
    let version = version.strip_prefix('w').unwrap_or(version);
    version.is_empty()
        || version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

// Aligned storage for TOKEN_USER and the SID that Windows writes after it.
type UserBuffer = [usize; 32];

fn user_sid(process: HANDLE, buffer: &mut UserBuffer) -> io::Result<PSID> {
    let mut token = std::ptr::null_mut();
    let mut size = 0;
    unsafe {
        if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = OwnedHandle::from_raw_handle(token);
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size_of::<UserBuffer>() as u32,
            &mut size,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok((*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}

fn target(pid: u32, own_session: u32, own_sid: PSID, scope: Scope, min_mib: u32) -> Option<Target> {
    unsafe {
        let handle = OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            0,
            pid,
        );
        if handle.is_null() {
            return None;
        }
        // Retain the verified process object through confirmation so a recycled PID
        // can never redirect the stop operation to a different process.
        let handle = OwnedHandle::from_raw_handle(handle);
        let raw = handle.as_raw_handle();
        let mut session = 0;
        if ProcessIdToSessionId(pid, &mut session) == 0 || session != own_session {
            return None;
        }
        let mut buffer = [0; 32];
        if EqualSid(own_sid, user_sid(raw, &mut buffer).ok()?) == 0 {
            return None;
        }
        let mut path = [0u16; 32768];
        let mut length = path.len() as u32;
        if QueryFullProcessImageNameW(raw, 0, path.as_mut_ptr(), &mut length) == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&path[..length as usize]);
        let name = path.rsplit('\\').next()?;
        if !scope.matches(name) {
            return None;
        }
        let mut memory = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            raw,
            &mut memory,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ) == 0
            || memory.WorkingSetSize < min_mib as usize * MIB
        {
            return None;
        }
        Some(Target {
            handle,
            name: name.into(),
            pid,
            working_set: memory.WorkingSetSize,
        })
    }
}

pub fn scan(scope: Scope, min_mib: u32) -> io::Result<Vec<Target>> {
    let mut targets = Vec::new();
    let mut own_session = 0;
    let mut buffer = [0; 32];
    unsafe {
        if ProcessIdToSessionId(GetCurrentProcessId(), &mut own_session) == 0 {
            return Err(io::Error::last_os_error());
        }
        let own_sid = user_sid(GetCurrentProcess(), &mut buffer)?;
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let snapshot = OwnedHandle::from_raw_handle(snapshot);
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot.as_raw_handle(), &mut entry) == 0 {
            return Err(io::Error::last_os_error());
        }
        loop {
            let length = entry
                .szExeFile
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(entry.szExeFile.len());
            if scope.matches(&String::from_utf16_lossy(&entry.szExeFile[..length]))
                && let Some(process) =
                    target(entry.th32ProcessID, own_session, own_sid, scope, min_mib)
            {
                targets.push(process);
            }
            if Process32NextW(snapshot.as_raw_handle(), &mut entry) == 0 {
                let error = GetLastError();
                if error != ERROR_NO_MORE_FILES {
                    return Err(io::Error::from_raw_os_error(error as i32));
                }
                break;
            }
        }
    }
    targets.sort_unstable_by_key(|process| std::cmp::Reverse(process.working_set));
    Ok(targets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    #[test]
    fn matches_runtime_executables_only() {
        for name in [
            "NODE.EXE",
            "python.exe",
            "pythonw.exe",
            "python3.13.exe",
            "python311.exe",
        ] {
            assert!(runtime_name(name), "{name}");
        }
        for name in [
            "node_helper.exe",
            "python-service.exe",
            "python3..13.exe",
            "python.dll",
            "Code.exe",
        ] {
            assert!(!runtime_name(name), "{name}");
        }
    }

    #[test]
    #[ignore = "smoke test temporarily allocates 272 MiB in a disposable process"]
    fn stops_only_the_heavy_test_process() {
        struct Fixture {
            child: Child,
            exe: std::path::PathBuf,
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                let _ = std::fs::remove_file(&self.exe);
            }
        }

        fn spawn(dir: &std::path::Path, name: &str, mib: &str) -> Fixture {
            let exe = dir.join(name);
            std::fs::copy(std::env::current_exe().unwrap(), &exe).unwrap();
            let child = Command::new(&exe)
                .args(["--ignored", "--exact", "processes::tests::memory_fixture"])
                .env("RAM_BUDDY_FIXTURE_MIB", mib)
                .stdout(Stdio::null())
                .spawn()
                .unwrap();
            Fixture { child, exe }
        }

        let dir = std::env::temp_dir().join(format!("ram-buddy-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let heavy = spawn(&dir, "node.exe", "272");
        let mut light = spawn(&dir, "python.exe", "2");
        let until = Instant::now() + Duration::from_secs(5);
        let target = loop {
            let targets = scan(Scope::All, 256).unwrap();
            assert!(!targets.iter().any(|target| target.pid == light.child.id()));
            if let Some(target) = targets
                .into_iter()
                .find(|target| target.pid == heavy.child.id())
            {
                break target;
            }
            assert!(Instant::now() < until, "heavy fixture never appeared");
            std::thread::sleep(Duration::from_millis(40));
        };
        let has_heavy = |scope, threshold| {
            scan(scope, threshold)
                .unwrap()
                .iter()
                .any(|target| target.pid == heavy.child.id())
        };
        assert!(has_heavy(Scope::Node, 256));
        assert!(!has_heavy(Scope::Python, 256));
        assert!(!has_heavy(Scope::Node, 512));
        assert!(target.stop());
        assert!(light.child.try_wait().unwrap().is_none());
        drop(target);
        drop(heavy);
        drop(light);
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    #[ignore = "started by the process smoke test"]
    fn memory_fixture() {
        let Ok(mib) = std::env::var("RAM_BUDDY_FIXTURE_MIB") else {
            return;
        };
        let memory = vec![1u8; mib.parse::<usize>().unwrap() * MIB];
        std::thread::sleep(Duration::from_secs(10));
        std::hint::black_box(memory);
    }
}
