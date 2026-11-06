use rewind::settings::ReplayShortcut;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc::{self, Receiver},
};

pub enum Event {
    Pressed,
    Failed {
        message: String,
        fallback: Option<ReplayShortcut>,
    },
}
pub struct Hotkey {
    pub events: Receiver<Event>,
    shutdown: Arc<AtomicBool>,
    thread_id: Arc<AtomicU32>,
    worker: Option<std::thread::JoinHandle<()>>,
    changes: mpsc::Sender<ReplayShortcut>,
}
impl Hotkey {
    pub fn new(initial: ReplayShortcut, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, events) = mpsc::channel();
        let (changes, requests) = mpsc::channel();
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
                        Input::KeyboardAndMouse::{
                            MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
                            UnregisterHotKey,
                        },
                        WindowsAndMessaging::{
                            GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, WM_HOTKEY,
                        },
                    },
                };
                // Own the registration and block on the native message queue: no polling timer.
                let mut msg: MSG = std::mem::zeroed();
                PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                target.store(GetCurrentThreadId(), Ordering::Release);
                let register = |shortcut: ReplayShortcut| {
                    let (modifiers, key) = match shortcut {
                        ReplayShortcut::CtrlShiftS => (MOD_CONTROL | MOD_SHIFT, 0x53),
                        ReplayShortcut::AltF10 => (MOD_ALT, 0x79),
                        ReplayShortcut::CtrlAltS => (MOD_CONTROL | MOD_ALT, 0x53),
                    };
                    RegisterHotKey(std::ptr::null_mut(), 1, modifiers | MOD_NOREPEAT, key) != 0
                };
                let mut current = register(initial).then_some(initial);
                if current.is_none() {
                    let _=tx.send(Event::Failed{message:format!("{} занята другим приложением. Выберите другую комбинацию в настройках.",initial.label()),fallback:None});
                    wake();
                }
                while !flag.load(Ordering::Relaxed) {
                    while let Ok(next) = requests.try_recv() {
                        UnregisterHotKey(std::ptr::null_mut(), 1);
                        if register(next) {
                            current = Some(next);
                        } else {
                            current = current.filter(|&previous| register(previous));
                            let suffix = current
                                .map(|previous| format!("Используется {}.", previous.label()))
                                .unwrap_or_else(|| "Выберите другую комбинацию.".into());
                            let _ = tx.send(Event::Failed {
                                message: format!("{} занята. {suffix}", next.label()),
                                fallback: current,
                            });
                            wake();
                        }
                    }
                    if flag.load(Ordering::Relaxed)
                        || GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) <= 0
                    {
                        break;
                    }
                    if msg.message == WM_HOTKEY {
                        let _ = tx.send(Event::Pressed);
                        wake();
                    }
                }
                UnregisterHotKey(std::ptr::null_mut(), 1);
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
    pub fn rebind(&self, shortcut: ReplayShortcut) {
        let _ = self.changes.send(shortcut);
        self.nudge();
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
