#![windows_subsystem = "windows"]

mod processes;
mod sprite;

use std::cell::Cell;
use std::io;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::ptr::null_mut;

use windows_sys::Win32::{
    Foundation::{
        ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM,
    },
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD, RegGetValueW, RegSetKeyValueW},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::CreateMutexW,
    },
    UI::{Shell::*, WindowsAndMessaging::*},
};
use windows_sys::w;

use processes::{MIB, Scope};
use sprite::Mood;

const TRAY_EVENT: u32 = WM_APP + 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const TIMER: usize = 1;
const CLEAN: u32 = 1;
const QUIT: u32 = 2;
const CLEAN_NODE: u32 = 3;
const CLEAN_PYTHON: u32 = 4;
const THRESHOLDS: [u32; 4] = [128, 256, 512, 1024];
const DEFAULT_THRESHOLD: u32 = 256;
const SETTINGS_KEY: *const u16 = w!("Software\\RAM Buddy");
const THRESHOLD_VALUE: *const u16 = w!("CleanupThresholdMiB");

fn saved_threshold() -> u32 {
    let mut threshold = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            SETTINGS_KEY,
            THRESHOLD_VALUE,
            RRF_RT_REG_DWORD,
            null_mut(),
            std::ptr::from_mut(&mut threshold).cast(),
            &mut size,
        )
    };
    if status == ERROR_SUCCESS && THRESHOLDS.contains(&threshold) {
        threshold
    } else {
        DEFAULT_THRESHOLD
    }
}

fn gib_parts(bytes: u64) -> (u64, u64) {
    const GIB: u64 = 1024 * MIB as u64;
    let tenths = bytes / GIB * 10 + (bytes % GIB * 10 + GIB / 2) / GIB;
    (tenths / 10, tenths % 10)
}

struct Tray {
    icons: [[HICON; 2]; 4],
    taskbar_created: u32,
    tick: Cell<u32>,
    memory: Cell<Option<MEMORYSTATUSEX>>,
    threshold: Cell<u32>,
    busy: Cell<bool>,
}

impl Tray {
    fn new() -> Self {
        Self {
            icons: [Mood::Running, Mood::Straining, Mood::Alarmed, Mood::Unknown]
                .map(|mood| [mood.create_icon(false), mood.create_icon(true)]),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            tick: Cell::new(0),
            memory: Cell::new(None),
            threshold: Cell::new(saved_threshold()),
            busy: Cell::new(false),
        }
    }

    fn refresh(&self, hwnd: HWND, sample: bool) {
        if sample {
            let mut memory = MEMORYSTATUSEX {
                dwLength: size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };
            let success = unsafe { GlobalMemoryStatusEx(&mut memory) } != 0;
            self.memory.set(success.then_some(memory));
        }
        let memory = self.memory.get();
        let mood = Mood::from_load(memory.map(|memory| memory.dwMemoryLoad));
        let mut data = tray_data(hwnd);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = TRAY_EVENT;
        data.hIcon = self.icons[mood as usize][(self.tick.get() % 2) as usize];
        let tip = match memory {
            Some(memory) => {
                let (used, used_decimal) =
                    gib_parts(memory.ullTotalPhys.saturating_sub(memory.ullAvailPhys));
                let (total, total_decimal) = gib_parts(memory.ullTotalPhys);
                format!(
                    "RAM Buddy · {}% · {used}.{used_decimal}/{total}.{total_decimal} GiB · {}\nClick to review processes ≥ {} MiB",
                    memory.dwMemoryLoad,
                    mood.label(),
                    self.threshold.get()
                )
            }
            None => format!(
                "RAM Buddy · RAM unavailable\nClick to review processes ≥ {} MiB",
                self.threshold.get()
            ),
        };
        for (output, character) in data.szTip.iter_mut().zip(tip.encode_utf16()) {
            *output = character;
        }
        unsafe {
            if Shell_NotifyIconW(NIM_MODIFY, &data) == 0 && Shell_NotifyIconW(NIM_ADD, &data) != 0 {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                Shell_NotifyIconW(NIM_SETVERSION, &data);
            }
        }
    }

    fn set_threshold(&self, hwnd: HWND, threshold: u32) {
        self.threshold.set(threshold);
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                SETTINGS_KEY,
                THRESHOLD_VALUE,
                REG_DWORD,
                std::ptr::from_ref(&threshold).cast(),
                size_of::<u32>() as u32,
            )
        };
        if status != ERROR_SUCCESS {
            dialog(
                hwnd,
                "Windows could not save the setting. The new threshold applies until RAM Buddy exits.",
                MB_ICONWARNING,
            );
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        for icon in self
            .icons
            .into_iter()
            .flatten()
            .filter(|icon| !icon.is_null())
        {
            unsafe { DestroyIcon(icon) };
        }
    }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        ..Default::default()
    }
}

fn dialog(hwnd: HWND, message: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(
            hwnd,
            message.as_ptr(),
            w!("RAM Buddy"),
            flags | MB_SETFOREGROUND,
        )
    }
}

fn cleanup(hwnd: HWND, scope: Scope, threshold: u32) {
    let runtime = scope.label();
    let targets = match processes::scan(scope, threshold) {
        Ok(targets) => targets,
        Err(error) => {
            dialog(
                hwnd,
                &format!("Could not scan processes.\n\n{error}"),
                MB_ICONERROR,
            );
            return;
        }
    };
    if targets.is_empty() {
        dialog(
            hwnd,
            &format!("No stoppable {runtime} processes are using {threshold} MiB of RAM or more."),
            MB_ICONINFORMATION,
        );
        return;
    }

    let count = targets.len();
    let mut summary = format!(
        "Force-stop {count} matching {runtime} process(es)?\nEach is using at least {threshold} MiB of RAM. Unsaved work may be lost.\n\n"
    );
    for process in targets.iter().take(10) {
        use std::fmt::Write;
        let _ = writeln!(
            summary,
            "{} (PID {}) · {} MiB",
            process.name,
            process.pid,
            process.working_set / MIB
        );
    }
    if count > 10 {
        summary.push_str(&format!("…and {} more\n", count - 10));
    }
    if dialog(hwnd, &summary, MB_YESNO | MB_DEFBUTTON2 | MB_ICONWARNING) != IDYES {
        return;
    }

    let stopped = targets.iter().filter(|target| target.stop()).count();
    let result = if stopped == count {
        format!("{stopped} process(es) are now closed.")
    } else {
        format!(
            "{stopped} process(es) are closed. Could not confirm that the other {} closed.",
            count - stopped
        )
    };
    dialog(hwnd, &result, MB_ICONINFORMATION);
}

fn menu(hwnd: HWND, threshold: u32) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(
            menu,
            MF_STRING,
            CLEAN as usize,
            w!("Review Node.js and Python…"),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            CLEAN_NODE as usize,
            w!("Review Node.js only…"),
        );
        AppendMenuW(
            menu,
            MF_STRING,
            CLEAN_PYTHON as usize,
            w!("Review Python only…"),
        );
        let thresholds = CreatePopupMenu();
        if !thresholds.is_null() {
            for value in THRESHOLDS {
                let label: Vec<u16> = format!("{value} MiB").encode_utf16().chain([0]).collect();
                let flags = MF_STRING
                    | if value == threshold {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                AppendMenuW(thresholds, flags, value as usize, label.as_ptr());
            }
            if AppendMenuW(
                menu,
                MF_POPUP,
                thresholds as usize,
                w!("Minimum RAM per process"),
            ) == 0
            {
                DestroyMenu(thresholds);
            }
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, std::ptr::null());
        AppendMenuW(menu, MF_STRING, QUIT as usize, w!("Exit RAM Buddy"));
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(hwnd);
        let selection = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        selection as u32
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        // The stack-owned tray outlives the window and every message it receives.
        let tray = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Tray).as_ref();
        if let Some(tray) = tray {
            match message {
                WM_TIMER if wparam == TIMER => {
                    let tick = tray.tick.get().wrapping_add(1);
                    tray.tick.set(tick);
                    tray.refresh(hwnd, tick % 4 == 0);
                    return 0;
                }
                TRAY_EVENT => {
                    let event = lparam as u32 & 0xffff;
                    if !matches!(event, WM_LBUTTONUP | NIN_KEYSELECT | WM_CONTEXTMENU)
                        || tray.busy.replace(true)
                    {
                        return 0;
                    }
                    let command = if event == WM_CONTEXTMENU {
                        menu(hwnd, tray.threshold.get())
                    } else {
                        CLEAN
                    };
                    let scope = match command {
                        CLEAN => Some(Scope::All),
                        CLEAN_NODE => Some(Scope::Node),
                        CLEAN_PYTHON => Some(Scope::Python),
                        _ => None,
                    };
                    if let Some(scope) = scope {
                        cleanup(hwnd, scope, tray.threshold.get());
                        tray.refresh(hwnd, true);
                    } else if THRESHOLDS.contains(&command) {
                        tray.set_threshold(hwnd, command);
                        tray.refresh(hwnd, false);
                    }
                    tray.busy.set(false);
                    if command == QUIT {
                        DestroyWindow(hwnd);
                    } else {
                        Shell_NotifyIconW(NIM_SETFOCUS, &tray_data(hwnd));
                    }
                    return 0;
                }
                WM_CLOSE if tray.busy.get() => return 0,
                WM_DESTROY => {
                    KillTimer(hwnd, TIMER);
                    Shell_NotifyIconW(NIM_DELETE, &tray_data(hwnd));
                    PostQuitMessage(0);
                    return 0;
                }
                message if message == tray.taskbar_created && message != 0 => {
                    tray.refresh(hwnd, true);
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}

fn run() -> io::Result<()> {
    unsafe {
        let mutex = CreateMutexW(std::ptr::null(), 0, w!("Local\\RAM Buddy 17E04A0B"));
        let error = GetLastError();
        if mutex.is_null() {
            return Err(io::Error::from_raw_os_error(error as i32));
        }
        let _mutex = OwnedHandle::from_raw_handle(mutex);
        if error == ERROR_ALREADY_EXISTS {
            return Ok(());
        }
        let tray = Tray::new();
        if tray.icons.iter().flatten().any(|icon| icon.is_null()) {
            return Err(io::Error::other(
                "Windows could not create the tray animation.",
            ));
        }
        let instance = GetModuleHandleW(std::ptr::null());
        let class = WNDCLASSW {
            hInstance: instance,
            lpfnWndProc: Some(window_proc),
            lpszClassName: w!("RamBuddyWindow"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            0,
            class.lpszClassName,
            w!("RAM Buddy"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(io::Error::last_os_error());
        }
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, std::ptr::from_ref(&tray) as isize);
        if SetTimer(hwnd, TIMER, 650, None) == 0 {
            let error = io::Error::last_os_error();
            DestroyWindow(hwnd);
            return Err(error);
        }
        tray.refresh(hwnd, true);

        let mut message = MSG::default();
        loop {
            match GetMessageW(&mut message, null_mut(), 0, 0) {
                -1 => {
                    let error = io::Error::last_os_error();
                    DestroyWindow(hwnd);
                    return Err(error);
                }
                0 => return Ok(()),
                _ => {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }
}

fn main() {
    if let Err(error) = run() {
        dialog(
            null_mut(),
            &format!("Could not start RAM Buddy.\n\n{error}"),
            MB_ICONERROR,
        );
    }
}
