use crate::hotkey::Hotkey;
use base64::Engine as _;
use rewind::{
    media::{self, Clip, EncodingOptions, Export, Quality, WorkDir},
    recorder::{self, Recorder},
    settings::{Encoder, Settings, snap_buffer_seconds},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
use tauri::{Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Clone, Serialize)]
struct Monitor {
    index: u32,
    name: String,
}

fn monitors() -> Vec<Monitor> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let outputs = (|| -> windows::core::Result<Vec<Monitor>> {
            let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
            let adapter = factory.EnumAdapters1(0)?;
            let mut monitors = Vec::new();
            for index in 0..16 {
                let Ok(output) = adapter.EnumOutputs(index) else {
                    break;
                };
                let desc = output.GetDesc()?;
                if desc.AttachedToDesktop.as_bool() {
                    monitors.push(Monitor {
                        index,
                        name: format!(
                            "Экран {} · {} × {}",
                            index + 1,
                            desc.DesktopCoordinates.right - desc.DesktopCoordinates.left,
                            desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top
                        ),
                    });
                }
            }
            Ok(monitors)
        })();
        if let Ok(outputs) = outputs
            && !outputs.is_empty()
        {
            return outputs;
        }
    }
    vec![Monitor {
        index: 0,
        name: "Экран 1".into(),
    }]
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReplayStatus {
    Stopped,
    Warming,
    Ready,
    Saving,
}

#[derive(Clone, Serialize)]
struct Snapshot {
    profile_id: String,
    settings: Settings,
    recording: bool,
    transitioning: bool,
    replay_status: ReplayStatus,
    encoders: Vec<Encoder>,
    monitors: Vec<Monitor>,
    error: Option<String>,
    quick: Vec<Clip>,
}

enum Action {
    Recording(bool),
    Configure(Settings),
    Save,
    OpenQuick(Clip),
    Shutdown,
}
struct Inner {
    state: Mutex<Snapshot>,
    clips: Mutex<HashMap<PathBuf, Clip>>,
    catalog: Mutex<rewind::library::Catalog>,
    actions: mpsc::Sender<Action>,
    wake: mpsc::Sender<()>,
    thumbnail_gate: Mutex<()>,
    cache: WorkDir,
    export_busy: AtomicBool,
    preview_busy: AtomicBool,
    shutting_down: AtomicBool,
    cancel: Mutex<Arc<AtomicBool>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}
#[derive(Clone)]
struct Engine(Arc<Inner>);
type Result<T> = std::result::Result<T, String>;

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Engine {
    fn refresh_library(&self, app: &tauri::AppHandle) {
        let catalog = self.0.catalog.lock().unwrap();
        let mut clips = self.0.clips.lock().unwrap();
        for clip in clips.values_mut() {
            let before = (clip.viewed, clip.original_path.clone(), clip.is_movie);
            catalog.decorate(clip);
            if before != (clip.viewed, clip.original_path.clone(), clip.is_movie) {
                let _ = app.emit("clip-updated", &*clip);
            }
        }
    }
    fn send(&self, action: Action) -> Result<()> {
        self.0.actions.send(action).map_err(error)?;
        self.0.wake.send(()).map_err(error)
    }
    fn clip(&self, path: &Path) -> Result<Clip> {
        self.0
            .clips
            .lock()
            .unwrap()
            .get(path)
            .cloned()
            .ok_or_else(|| "Откройте видео через приложение".into())
    }
    fn add_clip(&self, app: &tauri::AppHandle, path: &Path) -> Result<Clip> {
        let ffprobe = self.0.state.lock().unwrap().settings.ffprobe.clone();
        let mut clip = media::probe(&ffprobe, path).map_err(error)?;
        {
            let mut catalog = self.0.catalog.lock().unwrap();
            catalog.register(path).map_err(error)?;
            catalog.decorate(&mut clip);
        }
        app.asset_protocol_scope().allow_file(path).map_err(error)?;
        self.0
            .clips
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), clip.clone());
        Ok(clip)
    }
    fn shutdown(&self, installing_update: bool) {
        self.0.shutting_down.store(true, Ordering::Relaxed);
        let _ = self.send(Action::Shutdown);
        self.0.cancel.lock().unwrap().store(true, Ordering::Relaxed);
        if let Some(worker) = self.0.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
        // Cancellable jobs reap their FFmpeg children before process exit.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while ((!installing_update && self.0.export_busy.load(Ordering::SeqCst))
            || self.0.preview_busy.load(Ordering::SeqCst))
            && std::time::Instant::now() < deadline
        {
            thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

#[derive(Serialize)]
struct UpdateStatus {
    current: String,
    available: Option<String>,
}

#[tauri::command]
async fn check_update(app: tauri::AppHandle) -> Result<UpdateStatus> {
    let update = app
        .updater_builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(error)?
        .check()
        .await
        .map_err(error)?;
    Ok(UpdateStatus {
        current: app.package_info().version.to_string(),
        available: update.map(|release| release.version),
    })
}

#[tauri::command]
async fn install_update(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Engine>,
    version: String,
) -> Result<()> {
    if engine.0.shutting_down.load(Ordering::SeqCst)
        || engine.0.export_busy.swap(true, Ordering::SeqCst)
    {
        return Err("Дождитесь завершения сохранения видео".into());
    }
    struct InstallGuard(Engine);
    impl Drop for InstallGuard {
        fn drop(&mut self) {
            self.0.0.export_busy.store(false, Ordering::SeqCst);
        }
    }
    let _guard = InstallGuard(engine.inner().clone());

    let handle = app.clone();
    let update = app
        .updater_builder()
        .timeout(std::time::Duration::from_secs(120))
        .on_before_exit(move || {
            handle.state::<crate::startup::Instance>().shutdown();
            handle.state::<Engine>().shutdown(true);
        })
        .build()
        .map_err(error)?
        .check()
        .await
        .map_err(error)?
        .filter(|release| release.version == version)
        .ok_or_else(|| "Релиз изменился. Проверьте обновления ещё раз.".to_owned())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(error)?;
    app.restart();
}

fn notify(app: &tauri::AppHandle, message: &str) {
    let _ = app.emit("notice", message);
}
fn publish_state(engine: &Engine, app: &tauri::AppHandle) {
    let _ = app.emit("state", engine.0.state.lock().unwrap().clone());
}

fn show_quick_window(window: &tauri::WebviewWindow) -> Result<()> {
    if window.is_visible().map_err(error)? {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetWindowPos,
        };
        let hwnd = window.hwnd().map_err(error)?;
        // Make the window available to Alt+Tab behind the active game.
        let shown = unsafe {
            SetWindowPos(
                hwnd.0,
                HWND_BOTTOM,
                0,
                0,
                0,
                0,
                SWP_SHOWWINDOW | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
        };
        if shown == 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    window.show().map_err(error)
}

fn hide_quick(window: &tauri::WebviewWindow) {
    let _ = window.emit("app-hidden", ());
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        // The background show bypasses Tao's cached visibility flag.
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
        unsafe { ShowWindow(hwnd.0, SW_HIDE) };
    }
    #[cfg(not(windows))]
    let _ = window.hide();
}

// Called on the engine worker so window creation stays serialized and off the UI thread.
fn show_quick(app: &tauri::AppHandle, engine: &Engine, clip: Clip) -> Result<()> {
    let window = if let Some(window) = app.get_webview_window("quick") {
        window
    } else {
        tauri::WebviewWindowBuilder::new(
            app,
            "quick",
            tauri::WebviewUrl::App("index.html?quick=1".into()),
        )
        .title("Быстрый клип — Clipago")
        .inner_size(448.0, 426.0)
        .min_inner_size(420.0, 406.0)
        .resizable(true)
        .decorations(false)
        .focused(false)
        .visible(false)
        .center()
        .build()
        .map_err(error)?
    };
    let quick = window.clone();
    let app = app.clone();
    let engine = engine.clone();
    window
        .run_on_main_thread(move || {
            {
                let mut state = engine.0.state.lock().unwrap();
                if !state.quick.iter().any(|pending| pending.path == clip.path) {
                    state.quick.push(clip);
                }
            }
            if let Err(e) = show_quick_window(&quick) {
                engine.0.state.lock().unwrap().error = Some(e);
            }
            publish_state(&engine, &app);
        })
        .map_err(error)
}

fn create_engine(app: &tauri::AppHandle) -> anyhow::Result<Engine> {
    let loaded = Settings::load();
    let startup_error = crate::startup::register()
        .err()
        .map(|e| format!("Автозапуск Windows: {e}"));
    let initial_error = loaded.as_ref().err().map(error).or(startup_error);
    let settings = loaded.unwrap_or_default();
    let (actions, requests) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    let engine = Engine(Arc::new(Inner {
        state: Mutex::new(Snapshot {
            profile_id: rewind::settings::data_file("library.json")?
                .to_string_lossy()
                .into_owned(),
            settings: settings.clone(),
            recording: false,
            transitioning: true,
            replay_status: ReplayStatus::Warming,
            encoders: vec![Encoder::Auto, Encoder::Cpu],
            monitors: monitors(),
            error: initial_error,
            quick: Vec::new(),
        }),
        clips: Mutex::new(HashMap::new()),
        catalog: Mutex::new(rewind::library::Catalog::load(
            rewind::settings::data_file("library.json")?,
        )?),
        actions,
        wake: wake_tx.clone(),
        thumbnail_gate: Mutex::new(()),
        cache: WorkDir::new(&app.path().app_cache_dir()?, "session")?,
        export_busy: AtomicBool::new(false),
        preview_busy: AtomicBool::new(false),
        shutting_down: AtomicBool::new(false),
        cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
        worker: Mutex::new(None),
    }));
    let worker_engine = engine.clone();
    let handle = app.clone();
    let worker = thread::spawn(move || {
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.send(());
        });
        let recorder = Recorder::new(wake.clone());
        let hotkey = Hotkey::new(settings.shortcut.clone(), wake);
        let mut desired_recording = false;
        let mut saving = false;
        let mut buffered = 0.0;
        let mut retry_at: Option<std::time::Instant> = None;
        let mut retry_delay = 1_u64;
        let mut capture_error: Option<String> = None;
        let mut interrupted_at: Option<std::time::Instant> = None;
        loop {
            let result = if let Some(deadline) = retry_at {
                wake_rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            } else {
                wake_rx
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            };
            if matches!(result, Err(mpsc::RecvTimeoutError::Disconnected)) {
                break;
            }
            while let Ok(action) = requests.try_recv() {
                match action {
                    Action::Shutdown => return,
                    Action::OpenQuick(clip) => {
                        if let Err(e) = show_quick(&handle, &worker_engine, clip) {
                            worker_engine.0.state.lock().unwrap().error = Some(e);
                            publish_state(&worker_engine, &handle);
                        }
                    }
                    Action::Save => {
                        if desired_recording && !saving && buffered >= 1.0 {
                            saving = true;
                            let _ = recorder.tx.send(recorder::Request::Save);
                        }
                    }
                    Action::Recording(on) => {
                        desired_recording = on;
                        retry_at = None;
                        retry_delay = 1;
                        interrupted_at = None;
                        let mut state = worker_engine.0.state.lock().unwrap();
                        state.transitioning = true;
                        let request = if on {
                            recorder::Request::Start(state.settings.clone())
                        } else {
                            recorder::Request::Stop
                        };
                        let _ = recorder.tx.send(request);
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                    Action::Configure(mut next) => {
                        let mut state = worker_engine.0.state.lock().unwrap();
                        let previous = state.settings.clone();
                        let restart = previous.fps != next.fps
                            || previous.monitor != next.monitor
                            || previous.encoder != next.encoder
                            || previous.ffmpeg != next.ffmpeg;
                        if previous.shortcut != next.shortcut
                            && let Err(message) = hotkey.rebind(next.shortcut.clone())
                        {
                            next.shortcut = previous.shortcut.clone();
                            state.error = Some(message);
                        }
                        match next.save() {
                            Ok(()) => {
                                state.settings = next.clone();
                                if desired_recording {
                                    if restart {
                                        retry_at = None;
                                        state.transitioning = true;
                                        let _ = recorder.tx.send(recorder::Request::Stop);
                                        let _ = recorder.tx.send(recorder::Request::Start(next));
                                        notify(
                                            &handle,
                                            "Параметры применены · запись перезапущена",
                                        );
                                    } else {
                                        let _ = recorder.tx.send(recorder::Request::Configure {
                                            seconds: next.buffer_seconds,
                                            output_dir: next.output_dir,
                                            system_audio: next.system_audio,
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let rollback_error = (previous.shortcut != next.shortcut)
                                    .then(|| hotkey.rebind(previous.shortcut).err())
                                    .flatten();
                                state.error = Some(match rollback_error {
                                    Some(message) => format!("{} {message}", error(e)),
                                    None => error(e),
                                });
                            }
                        }
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                }
            }
            while let Ok(event) = hotkey.events.try_recv() {
                match event {
                    crate::hotkey::Event::Pressed
                        if desired_recording && !saving && buffered >= 1.0 =>
                    {
                        saving = true;
                        let _ = recorder.tx.send(recorder::Request::Save);
                    }
                    crate::hotkey::Event::Failed(message) => {
                        let mut state = worker_engine.0.state.lock().unwrap();
                        state.error = Some(message);
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                    _ => {}
                }
            }
            while let Ok(event) = recorder.rx.try_recv() {
                match event {
                    recorder::Event::Buffer(seconds) => buffered = seconds,
                    recorder::Event::Saving => saving = true,
                    recorder::Event::Started(_) => {
                        retry_at = None;
                        retry_delay = 1;
                        interrupted_at = None;
                        let mut state = worker_engine.0.state.lock().unwrap();
                        if capture_error.is_some() && state.error == capture_error {
                            state.error = None;
                        }
                        capture_error = None;
                        state.recording = true;
                        state.transitioning = false;
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                    recorder::Event::Stopped => {
                        buffered = 0.0;
                        let mut state = worker_engine.0.state.lock().unwrap();
                        state.recording = false;
                        state.transitioning = desired_recording;
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                    recorder::Event::CaptureFailed { message, temporary } => {
                        let mut state = worker_engine.0.state.lock().unwrap();
                        state.recording = false;
                        let interrupted =
                            interrupted_at.get_or_insert_with(std::time::Instant::now);
                        if !temporary || interrupted.elapsed() >= std::time::Duration::from_secs(10)
                        {
                            let message = if temporary {
                                "Экран временно недоступен. Запись восстановится автоматически."
                                    .to_owned()
                            } else {
                                message
                            };
                            capture_error = Some(message.clone());
                            state.error = Some(message);
                        }
                        state.transitioning = desired_recording && temporary;
                        if desired_recording {
                            let delay = if temporary {
                                retry_delay
                            } else {
                                retry_delay.max(5)
                            };
                            retry_at = Some(
                                std::time::Instant::now() + std::time::Duration::from_secs(delay),
                            );
                            retry_delay = (retry_delay * 2).min(30);
                        }
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                    recorder::Event::Error(e) => {
                        saving = false;
                        worker_engine.0.state.lock().unwrap().error = Some(e);
                        publish_state(&worker_engine, &handle);
                    }
                    recorder::Event::AudioState(result) => {
                        let mut state = worker_engine.0.state.lock().unwrap();
                        match result {
                            Err(e) => state.error = Some(format!("Системный звук: {e}")),
                            Ok(())
                                if state
                                    .error
                                    .as_ref()
                                    .is_some_and(|e| e.starts_with("Системный звук:")) =>
                            {
                                state.error = None
                            }
                            _ => {}
                        }
                        drop(state);
                        publish_state(&worker_engine, &handle);
                    }
                    recorder::Event::Saved(result) => {
                        saving = false;
                        match result
                            .map_err(error)
                            .and_then(|path| worker_engine.add_clip(&handle, &path))
                        {
                            Ok(clip) => {
                                let _ = handle.emit("clip-added", &clip);
                                if let Err(e) = show_quick(&handle, &worker_engine, clip) {
                                    worker_engine.0.state.lock().unwrap().error = Some(e);
                                    publish_state(&worker_engine, &handle);
                                }
                            }
                            Err(e) => {
                                worker_engine.0.state.lock().unwrap().error = Some(e);
                                publish_state(&worker_engine, &handle);
                            }
                        }
                    }
                }
            }
            let replay_status = if saving {
                ReplayStatus::Saving
            } else if !desired_recording {
                ReplayStatus::Stopped
            } else if buffered >= 1.0 {
                ReplayStatus::Ready
            } else {
                ReplayStatus::Warming
            };
            let mut state = worker_engine.0.state.lock().unwrap();
            let replay_changed = state.replay_status != replay_status;
            state.replay_status = replay_status;
            drop(state);
            if replay_changed {
                publish_state(&worker_engine, &handle);
            }
            if retry_at.is_some_and(|deadline| std::time::Instant::now() >= deadline) {
                retry_at = None;
                if desired_recording {
                    let mut state = worker_engine.0.state.lock().unwrap();
                    state.transitioning = true;
                    let _ = recorder
                        .tx
                        .send(recorder::Request::Start(state.settings.clone()));
                    drop(state);
                    publish_state(&worker_engine, &handle);
                }
            }
        }
    });
    *engine.0.worker.lock().unwrap() = Some(worker);
    engine
        .send(Action::Recording(true))
        .map_err(anyhow::Error::msg)?;
    let probe_engine = engine.clone();
    let probe_app = app.clone();
    thread::spawn(move || {
        let settings = probe_engine.0.state.lock().unwrap().settings.clone();
        match media::check_tools(&settings.ffmpeg, &settings.ffprobe) {
            Ok(()) => {
                let encoders = media::available_recording_encoders(&settings.ffmpeg);
                probe_engine.0.state.lock().unwrap().encoders = encoders;
            }
            Err(e) => probe_engine.0.state.lock().unwrap().error = Some(error(e)),
        }
        publish_state(&probe_engine, &probe_app);
    });
    Ok(engine)
}

#[tauri::command]
fn snapshot(engine: tauri::State<Engine>) -> Snapshot {
    engine.0.state.lock().unwrap().clone()
}
#[tauri::command]
fn set_recording(engine: tauri::State<Engine>, enabled: bool) -> Result<()> {
    engine.send(Action::Recording(enabled))
}
#[tauri::command]
fn save_replay(engine: tauri::State<Engine>, app: tauri::AppHandle) -> Result<()> {
    let mut state = engine.0.state.lock().unwrap();
    match state.replay_status {
        ReplayStatus::Stopped => return Err("Фоновая запись выключена".into()),
        ReplayStatus::Warming => return Err("Буфер ещё готовится".into()),
        ReplayStatus::Saving => return Err("Момент уже сохраняется".into()),
        ReplayStatus::Ready => state.replay_status = ReplayStatus::Saving,
    }
    drop(state);
    if let Err(error) = engine.send(Action::Save) {
        engine.0.state.lock().unwrap().replay_status = ReplayStatus::Ready;
        return Err(error);
    }
    publish_state(&engine, &app);
    Ok(())
}
#[tauri::command]
fn update_settings(engine: tauri::State<Engine>, mut settings: Settings) -> Result<()> {
    settings.buffer_seconds = snap_buffer_seconds(settings.buffer_seconds);
    if ![30, 60].contains(&settings.fps) || settings.monitor > 15 {
        return Err("Некорректные параметры записи".into());
    }
    engine.send(Action::Configure(settings))
}
#[tauri::command]
fn dismiss_error(app: tauri::AppHandle, engine: tauri::State<Engine>) {
    engine.0.state.lock().unwrap().error = None;
    publish_state(&engine, &app);
}

#[tauri::command]
fn show_quick_clip(engine: tauri::State<Engine>, path: PathBuf) -> Result<()> {
    engine.send(Action::OpenQuick(engine.clip(&path)?))
}

#[tauri::command]
async fn import_paths(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Engine>,
    paths: Vec<PathBuf>,
) -> Result<Vec<Clip>> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| engine.add_clip(&app, &path))
            .collect()
    })
    .await
    .map_err(error)?
}

#[tauri::command]
async fn library(app: tauri::AppHandle, engine: tauri::State<'_, Engine>) -> Result<Vec<Clip>> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = engine.0.state.lock().unwrap().settings.output_dir.clone();
        let mut files = engine.0.catalog.lock().unwrap().paths();
        files.extend(
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().is_some_and(|ext| {
                        ["mp4", "mkv", "mov", "webm", "avi", "gif", "m4v", "ts"]
                            .contains(&ext.to_string_lossy().to_lowercase().as_str())
                    })
                }),
        );
        files.sort();
        files.dedup();
        files.retain(|p| p.is_file());
        files.sort_by_key(|p| {
            std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok())
        });
        Ok(files
            .into_iter()
            .filter_map(|path| engine.add_clip(&app, &path).ok())
            .collect())
    })
    .await
    .map_err(error)?
}
#[tauri::command]
async fn import_video(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Engine>,
) -> Result<Option<Clip>> {
    let engine = engine.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Видео",
                &["mp4", "mkv", "mov", "webm", "avi", "m4v", "ts", "gif"],
            )
            .pick_file()
        else {
            return Ok(None);
        };
        engine.add_clip(&app, &path).map(Some)
    })
    .await
    .map_err(error)?
}

#[tauri::command]
fn mark_viewed(app: tauri::AppHandle, engine: tauri::State<Engine>, path: PathBuf) -> Result<Clip> {
    engine.clip(&path)?;
    let catalog = &mut *engine.0.catalog.lock().unwrap();
    catalog.viewed(&path).map_err(error)?;
    let mut clips = engine.0.clips.lock().unwrap();
    for clip in clips.values_mut() {
        let before = clip.viewed;
        catalog.decorate(clip);
        if clip.viewed != before {
            let _ = app.emit("clip-updated", &*clip);
        }
    }
    clips
        .get(&path)
        .cloned()
        .ok_or_else(|| "Клип не найден".into())
}

#[derive(Deserialize)]
struct MoviePartRequest {
    #[serde(flatten)]
    edit: ExportRequest,
    preprocess: bool,
    #[serde(default)]
    gap_before: f64,
}
#[derive(Deserialize)]
struct MovieOutputRequest {
    name: String,
    quality: Quality,
    bitrate_mbps: f64,
    target_mb: f64,
    max_height: Option<u32>,
    encoding: EncodingOptions,
    audio_tracks: Option<Vec<u32>>,
}
#[tauri::command]
async fn render_movie(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Engine>,
    segments: Vec<MoviePartRequest>,
    output: MovieOutputRequest,
) -> Result<Option<Clip>> {
    let engine = engine.inner().clone();
    let mut parts = Vec::new();
    for segment in segments {
        let edit = segment.edit;
        parts.push(rewind::movie::Part {
            preprocess: segment.preprocess,
            gap_before: segment.gap_before,
            edit: Export {
                clip: engine.clip(&edit.path)?,
                output: PathBuf::new(),
                start: edit.start,
                end: edit.end,
                quality: edit.quality,
                bitrate_mbps: edit.bitrate_mbps,
                target_mb: edit.target_mb,
                max_height: edit.max_height,
                encoding: edit.encoding,
                audio_tracks: edit.audio_tracks,
            },
        });
    }
    if parts.is_empty() {
        return Err("Добавьте клипы в монтаж".into());
    }
    if engine.0.export_busy.swap(true, Ordering::SeqCst) {
        return Err("Дождитесь завершения сохранения".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *engine.0.cancel.lock().unwrap() = cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        struct Guard(Engine);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.0.export_busy.store(false, Ordering::SeqCst);
            }
        }
        let _guard = Guard(engine.clone());
        let settings = engine.0.state.lock().unwrap().settings.clone();
        let format = output.encoding.format;
        let name: String = output
            .name
            .chars()
            .take(80)
            .map(|c| {
                if "/\\:*?\"<>|".contains(c) || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let Some(path) = rfd::FileDialog::new()
            .set_directory(&settings.output_dir)
            .set_file_name(format!(
                "{}.{}",
                if name.trim().is_empty() {
                    "Movie"
                } else {
                    name.trim()
                },
                format.extension()
            ))
            .add_filter(format.label(), &[format.extension()])
            .save_file()
        else {
            return Ok(None);
        };
        let mut clip = parts[0].edit.clip.clone();
        clip.height = parts
            .iter()
            .map(|p| p.edit.clip.height)
            .max()
            .unwrap_or(1080)
            .min(2160);
        clip.width = ((clip.height as f64 * 16.0 / 9.0 / 2.0).round() as u32) * 2;
        clip.fps = parts.iter().map(|p| p.edit.clip.fps).fold(1.0, f64::max);
        clip.duration = parts.iter().map(|p| p.edit.end - p.edit.start).sum();
        let request = Export {
            clip,
            output: path.with_extension(format.extension()),
            start: 0.0,
            end: parts.iter().map(|p| p.edit.end - p.edit.start).sum(),
            quality: output.quality,
            bitrate_mbps: output.bitrate_mbps,
            target_mb: output.target_mb,
            max_height: output.max_height,
            encoding: output.encoding,
            audio_tracks: output.audio_tracks,
        };
        let path = rewind::movie::render(
            &settings.ffmpeg,
            &settings.ffprobe,
            &parts,
            &request,
            cancel,
        )
        .map_err(error)?;
        engine
            .0
            .catalog
            .lock()
            .unwrap()
            .movie(&path)
            .map_err(error)?;
        let saved = engine.add_clip(&app, &path)?;
        let _ = app.emit("clip-added", &saved);
        notify(&app, "Мувик сохранён");
        Ok(Some(saved))
    })
    .await
    .map_err(error)?
}
#[tauri::command]
async fn choose_folder(engine: tauri::State<'_, Engine>) -> Result<Option<String>> {
    let dir = engine.0.state.lock().unwrap().settings.output_dir.clone();
    tauri::async_runtime::spawn_blocking(move || {
        Ok(rfd::FileDialog::new()
            .set_directory(dir)
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned()))
    })
    .await
    .map_err(error)?
}
#[tauri::command]
fn open_folder(engine: tauri::State<Engine>) -> Result<()> {
    let dir = engine.0.state.lock().unwrap().settings.output_dir.clone();
    std::fs::create_dir_all(&dir).map_err(error)?;
    media::command("explorer.exe")
        .arg(dir)
        .spawn()
        .map_err(error)?;
    Ok(())
}
#[tauri::command]
async fn thumbnail(
    engine: tauri::State<'_, Engine>,
    path: PathBuf,
    seconds: f64,
) -> Result<String> {
    let engine = engine.inner().clone();
    let clip = engine.clip(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=engine.0.thumbnail_gate.lock().unwrap();
        let ffmpeg=engine.0.state.lock().unwrap().settings.ffmpeg.clone();
        let output=media::command(media::tool(&ffmpeg)).args(["-v","error","-nostdin","-filter_threads","1","-threads","1","-ss",&format!("{:.3}",seconds.clamp(0.0,(clip.duration-0.05).max(0.0)))])
            .arg("-i").arg(path).args(["-frames:v","1","-vf","scale=240:135:force_original_aspect_ratio=decrease,pad=240:135:(ow-iw)/2:(oh-ih)/2,setsar=1","-threads","1","-c:v","png","-f","image2pipe","pipe:1"])
            .stdin(Stdio::null()).output().map_err(error)?;
        if !output.status.success(){return Err("Не удалось получить кадр".into())}
        Ok(format!("data:image/png;base64,{}",base64::engine::general_purpose::STANDARD.encode(output.stdout)))
    }).await.map_err(error)?
}

#[tauri::command]
async fn prepare_preview(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Engine>,
    path: PathBuf,
) -> Result<String> {
    let engine = engine.inner().clone();
    engine.clip(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = engine.0.thumbnail_gate.lock().unwrap();
        if engine.0.shutting_down.load(Ordering::Relaxed) {
            return Err("Приложение закрывается".into());
        }
        engine.0.preview_busy.store(true, Ordering::SeqCst);
        struct PreviewGuard(Engine);
        impl Drop for PreviewGuard {
            fn drop(&mut self) {
                self.0.0.preview_busy.store(false, Ordering::SeqCst);
            }
        }
        let _busy = PreviewGuard(engine.clone());
        let output = media::unique_path(&engine.0.cache.0, "preview", "mp4");
        let ffmpeg = engine.0.state.lock().unwrap().settings.ffmpeg.clone();
        let mut cmd = media::command(media::tool(&ffmpeg));
        cmd.args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "0:a:0?",
                "-vf",
                "scale=-2:'min(720,ih)'",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "22",
                "-threads",
                "2",
                "-c:a",
                "aac",
                "-movflags",
                "+faststart",
            ])
            .arg(&output)
            .stdin(Stdio::null());
        media::run_ffmpeg(&mut cmd, &engine.0.cache.0, &engine.0.shutting_down).map_err(error)?;
        app.asset_protocol_scope()
            .allow_file(&output)
            .map_err(error)?;
        Ok(output.to_string_lossy().into_owned())
    })
    .await
    .map_err(error)?
}

#[tauri::command]
async fn prepare_audio_preview(
    app: tauri::AppHandle,
    engine: tauri::State<'_, Engine>,
    path: PathBuf,
) -> Result<Vec<media::PreviewAudioTrack>> {
    let engine = engine.inner().clone();
    let clip = engine.clip(&path)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = engine.0.thumbnail_gate.lock().unwrap();
        if engine.0.shutting_down.load(Ordering::Relaxed) {
            return Err("Приложение закрывается".into());
        }
        engine.0.preview_busy.store(true, Ordering::SeqCst);
        struct Guard(Engine);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.0.preview_busy.store(false, Ordering::SeqCst);
            }
        }
        let _busy = Guard(engine.clone());
        let ffmpeg = engine.0.state.lock().unwrap().settings.ffmpeg.clone();
        let tracks =
            media::prepare_audio_tracks(&ffmpeg, &clip, &engine.0.cache.0, &engine.0.shutting_down)
                .map_err(error)?;
        for track in &tracks {
            app.asset_protocol_scope()
                .allow_file(&track.path)
                .map_err(error)?;
        }
        Ok(tracks)
    })
    .await
    .map_err(error)?
}

#[derive(Deserialize)]
struct ExportRequest {
    path: PathBuf,
    start: f64,
    end: f64,
    quality: Quality,
    bitrate_mbps: f64,
    target_mb: f64,
    max_height: Option<u32>,
    encoding: EncodingOptions,
    #[serde(default)]
    audio_tracks: Option<Vec<u32>>,
}

#[tauri::command]
async fn export_video(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    engine: tauri::State<'_, Engine>,
    request: ExportRequest,
) -> Result<Option<Clip>> {
    let engine = engine.inner().clone();
    let clip = engine.clip(&request.path)?;
    if !request.start.is_finite()
        || !request.end.is_finite()
        || request.start < 0.0
        || request.end > clip.duration + 0.05
        || request.end - request.start < 0.05
    {
        return Err("Выберите корректный отрезок видео".into());
    }
    if engine.0.export_busy.swap(true, Ordering::SeqCst) {
        return Err("Дождитесь завершения экспорта".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *engine.0.cancel.lock().unwrap() = cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        struct Guard(Engine);
        impl Drop for Guard {
            fn drop(&mut self) {
                self.0.0.export_busy.store(false, Ordering::SeqCst);
            }
        }
        let _guard = Guard(engine.clone());
        let settings = engine.0.state.lock().unwrap().settings.clone();
        let format = request.encoding.format;
        let source_path = clip.path.clone();
        let stem = clip.path.file_stem().unwrap_or_default().to_string_lossy();
        let output = media::unique_path(
            &settings.output_dir,
            &format!("{stem}_edit"),
            format.extension(),
        );
        let job = Export {
            clip,
            output,
            start: request.start,
            end: request.end,
            quality: request.quality,
            bitrate_mbps: request.bitrate_mbps,
            target_mb: request.target_mb,
            max_height: request.max_height,
            encoding: request.encoding,
            audio_tracks: request.audio_tracks,
        };
        let output = media::export(&settings.ffmpeg, &job, cancel).map_err(error)?;
        engine
            .0
            .catalog
            .lock()
            .unwrap()
            .edited(&output, &source_path)
            .map_err(error)?;
        let saved = engine.add_clip(&app, &output)?;
        engine.refresh_library(&app);
        let _ = app.emit("clip-added", &saved);
        notify(&app, "Копия сохранена");
        if window.label() == "quick" {
            let app_handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                let empty = {
                    let mut state = engine.0.state.lock().unwrap();
                    state.quick.retain(|clip| clip.path != source_path);
                    state.quick.is_empty()
                };
                if empty {
                    hide_quick(&window);
                }
                publish_state(&engine, &app_handle);
            });
        }
        Ok(Some(saved))
    })
    .await
    .map_err(error)?
}
#[tauri::command]
fn cancel_export(engine: tauri::State<Engine>) {
    engine
        .0
        .cancel
        .lock()
        .unwrap()
        .store(true, Ordering::Relaxed);
}

pub fn run() {
    let background = crate::startup::background();
    let _instance = match crate::startup::Instance::acquire(background) {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(e) => {
            rfd::MessageDialog::new()
                .set_title("Clipago")
                .set_description(format!("Не удалось запустить Clipago: {e}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return;
        }
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            set_recording,
            save_replay,
            update_settings,
            dismiss_error,
            show_quick_clip,
            import_paths,
            library,
            mark_viewed,
            render_movie,
            import_video,
            choose_folder,
            open_folder,
            thumbnail,
            prepare_preview,
            prepare_audio_preview,
            export_video,
            cancel_export,
            check_update,
            install_update
        ])
        .setup(move |app| {
            let engine = create_engine(app.handle())?;
            app.manage(engine);
            if let Some(window) = app.get_webview_window("main") {
                if !background {
                    let _ = window.show();
                }
            }
            _instance.listen(app.handle().clone());
            app.manage(_instance);
            let show =
                tauri::menu::MenuItem::with_id(app, "show", "Открыть Clipago", true, None::<&str>)?;
            let save =
                tauri::menu::MenuItem::with_id(app, "save", "Сохранить клип", true, None::<&str>)?;
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&show, &save, &quit])?;
            tauri::tray::TrayIconBuilder::new()
                .icon(
                    app.default_window_icon()
                        .cloned()
                        .ok_or_else(|| std::io::Error::other("Missing Clipago icon"))?,
                )
                .tooltip("Clipago")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "save" => {
                        let _ = app.state::<Engine>().send(Action::Save);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(window.label(), "main" | "quick")
                && let tauri::WindowEvent::CloseRequested { api, .. } = event
            {
                api.prevent_close();
                if window.label() == "main" {
                    let _ = window.emit("app-hidden", ());
                    let _ = window.hide();
                } else if let Some(quick) = window.app_handle().get_webview_window("quick") {
                    let engine = window.state::<Engine>();
                    engine.0.state.lock().unwrap().quick.clear();
                    hide_quick(&quick);
                    publish_state(&engine, window.app_handle());
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("Не удалось запустить Clipago");
    app.run(|app, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            app.state::<crate::startup::Instance>().shutdown();
            app.state::<Engine>().shutdown(false);
        }
    });
}
