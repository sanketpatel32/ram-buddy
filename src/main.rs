#![windows_subsystem = "windows"]

mod processes;
mod sprite;

use std::cell::Cell;
use std::io;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use std::ptr::null_mut;

use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::CreateMutexW,
    },
    UI::{Shell::*, WindowsAndMessaging::*},
};
use windows_sys::w;

use processes::{MIB, MIN_MIB};
use sprite::Mood;

const TRAY_EVENT: u32 = WM_APP + 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const TIMER: usize = 1;
const CLEAN: u32 = 1;
const QUIT: u32 = 2;

struct Tray {
    icons: [[HICON; 2]; 4],
    taskbar_created: u32,
    tick: Cell<u32>,
    load: Cell<Option<u32>>,
    busy: Cell<bool>,
}

impl Tray {
    fn new() -> Self {
        Self {
            icons: [Mood::Running, Mood::Straining, Mood::Alarmed, Mood::Unknown]
                .map(|mood| [mood.create_icon(false), mood.create_icon(true)]),
            taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
            tick: Cell::new(0),
            load: Cell::new(None),
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
            self.load.set(success.then_some(memory.dwMemoryLoad));
        }
        let load = self.load.get();
        let mood = Mood::from_load(load);
        let mut data = tray_data(hwnd);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = TRAY_EVENT;
        data.hIcon = self.icons[mood as usize][(self.tick.get() % 2) as usize];
        let tip = match load {
            Some(load) => format!(
                "RAM Buddy · {load}% RAM · {}\nClick to review heavy Node/Python tasks",
                mood.label()
            ),
            None => "RAM Buddy · RAM unavailable\nClick to review heavy Node/Python tasks".into(),
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

fn cleanup(hwnd: HWND) {
    let targets = match processes::scan() {
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
            &format!(
                "No stoppable Node or Python processes are using {MIN_MIB} MiB of RAM or more."
            ),
            MB_ICONINFORMATION,
        );
        return;
    }

    let count = targets.len();
    let mut summary = format!(
        "Force-stop {count} heavy Node/Python process(es)?\nUnsaved work in these processes may be lost.\n\n"
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

fn menu(hwnd: HWND) -> u32 {
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return 0;
        }
        AppendMenuW(
            menu,
            MF_STRING,
            CLEAN as usize,
            w!("Review heavy processes…"),
        );
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
                        menu(hwnd)
                    } else {
                        CLEAN
                    };
                    if command == CLEAN {
                        cleanup(hwnd);
                        tray.refresh(hwnd, true);
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
