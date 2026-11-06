use rewind::settings::ReplayShortcut;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc::{self, Receiver},
};

pub enum Event {
    Pressed,
    Failed(String),
}

type Rebind = (ReplayShortcut, mpsc::Sender<Result<(), String>>);

#[cfg(windows)]
fn windows_shortcut(shortcut: &ReplayShortcut) -> Option<(u32, u32)> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    let (prefix, code) = shortcut
        .code()
        .rsplit_once('+')
        .unwrap_or(("", shortcut.code()));
    let mut modifiers = 0;
    for part in prefix.split('+').filter(|part| !part.is_empty()) {
        modifiers |= match part {
            "Ctrl" => MOD_CONTROL,
            "Shift" => MOD_SHIFT,
            "Alt" => MOD_ALT,
            "Meta" => MOD_WIN,
            _ => return None,
        };
    }

    let key = match code.as_bytes() {
        [b'K', b'e', b'y', key @ b'A'..=b'Z']
        | [b'D', b'i', b'g', b'i', b't', key @ b'0'..=b'9'] => u16::from(*key),
        [b'N', b'u', b'm', b'p', b'a', b'd', key @ b'0'..=b'9'] => {
            VK_NUMPAD0 + u16::from(*key - b'0')
        }
        _ => match code {
            "Space" => VK_SPACE,
            "Backspace" => VK_BACK,
            "Enter" | "NumpadEnter" => VK_RETURN,
            "Insert" => VK_INSERT,
            "Delete" => VK_DELETE,
            "Home" => VK_HOME,
            "End" => VK_END,
            "PageUp" => VK_PRIOR,
            "PageDown" => VK_NEXT,
            "ArrowLeft" => VK_LEFT,
            "ArrowRight" => VK_RIGHT,
            "ArrowUp" => VK_UP,
            "ArrowDown" => VK_DOWN,
            "Minus" => VK_OEM_MINUS,
            "Equal" => VK_OEM_PLUS,
            "Comma" => VK_OEM_COMMA,
            "Period" => VK_OEM_PERIOD,
            "Slash" => VK_OEM_2,
            "Semicolon" => VK_OEM_1,
            "Quote" => VK_OEM_7,
            "Backquote" => VK_OEM_3,
            "BracketLeft" => VK_OEM_4,
            "BracketRight" => VK_OEM_6,
            "Backslash" => VK_OEM_5,
            "IntlBackslash" => VK_OEM_102,
            "NumpadAdd" => VK_ADD,
            "NumpadSubtract" => VK_SUBTRACT,
            "NumpadMultiply" => VK_MULTIPLY,
            "NumpadDivide" => VK_DIVIDE,
            "NumpadDecimal" => VK_DECIMAL,
            _ => {
                let number = code.strip_prefix('F')?.parse::<u16>().ok()?;
                if !(1..=24).contains(&number) || number == 12 {
                    return None;
                }
                VK_F1 + number - 1
            }
        },
    };
    if modifiers == 0 && !(VK_F1..=VK_F24).contains(&key) {
        return None;
    }
    Some((modifiers, u32::from(key)))
}

pub struct Hotkey {
    pub events: Receiver<Event>,
    shutdown: Arc<AtomicBool>,
    thread_id: Arc<AtomicU32>,
    worker: Option<std::thread::JoinHandle<()>>,
    changes: mpsc::Sender<Rebind>,
}
impl Hotkey {
    pub fn new(initial: ReplayShortcut, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, events) = mpsc::channel();
        let (changes, requests) = mpsc::channel::<Rebind>();
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let thread_id = Arc::new(AtomicU32::new(0));
        let target = thread_id.clone();
        let worker = std::thread::spawn(move || {
            #[cfg(windows)]
            unsafe {
                use windows_sys::Win32::{
                    System::Threading::GetCurrentThreadId,
                    UI::{
                        Input::KeyboardAndMouse::{MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey},
                        WindowsAndMessaging::{
                            GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY,
                        },
                    },
                };
                // Own the registration and block on the native message queue: no polling timer.
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                target.store(GetCurrentThreadId(), Ordering::Release);
                let mut current = None;
                let register =
                    |shortcut: &ReplayShortcut, current: &mut Option<(i32, (u32, u32))>| {
                        let keys = windows_shortcut(shortcut).ok_or_else(|| {
                        "Выберите клавишу F1–F24 (кроме F12) или сочетание с Ctrl, Alt, Shift или Win.".to_string()
                    })?;
                        if current.is_some_and(|(_, registered)| registered == keys) {
                            return Ok(());
                        }
                        let id = if current.is_some_and(|(id, _)| id == 1) {
                            2
                        } else {
                            1
                        };
                        let (modifiers, key) = keys;
                        if RegisterHotKey(std::ptr::null_mut(), id, modifiers | MOD_NOREPEAT, key)
                            == 0
                        {
                            return Err(format!(
                                "{} занята или зарезервирована Windows.",
                                shortcut.label()
                            ));
                        }
                        if let Some((previous, _)) = current.replace((id, keys)) {
                            UnregisterHotKey(std::ptr::null_mut(), previous);
                        }
                        Ok(())
                    };
                if let Err(message) = register(&initial, &mut current) {
                    let _ = tx.send(Event::Failed(message));
                    wake();
                }
                while !flag.load(Ordering::Relaxed) {
                    while let Ok((next, reply)) = requests.try_recv() {
                        let _ = reply.send(register(&next, &mut current));
                    }
                    if flag.load(Ordering::Relaxed)
                        || GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) <= 0
                    {
                        break;
                    }
                    if msg.message == WM_HOTKEY
                        && current.is_some_and(|(id, _)| id as usize == msg.wParam)
                    {
                        let _ = tx.send(Event::Pressed);
                        wake();
                    }
                }
                if let Some((id, _)) = current {
                    UnregisterHotKey(std::ptr::null_mut(), id);
                }
            }
            #[cfg(not(windows))]
            {
                let _ = (initial, wake, flag, target, requests, tx);
            }
        });
        Self {
            events,
            shutdown,
            thread_id,
            worker: Some(worker),
            changes,
        }
    }
    fn nudge(&self) {
        #[cfg(windows)]
        unsafe {
            let id = self.thread_id.load(Ordering::Acquire);
            if id != 0 {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(id, 0x8001, 0, 0);
            }
        }
    }
    pub fn rebind(&self, shortcut: ReplayShortcut) -> Result<(), String> {
        let (reply, result) = mpsc::channel();
        let unavailable = || "Не удалось назначить сочетание.".to_string();
        self.changes
            .send((shortcut, reply))
            .map_err(|_| unavailable())?;
        self.nudge();
        result.recv().map_err(|_| unavailable())?
    }
}
impl Drop for Hotkey {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.nudge();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
