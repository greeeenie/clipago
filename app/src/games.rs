use crate::audio_identity::{
    icon_data,
    windows::{app_name, cache_icon},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use steam_vdf_parser::{parse_appinfo, text::parse_text};

struct Game {
    name: String,
    install: String,
    aliases: Vec<String>,
    executables: Vec<PathBuf>,
    steam_icon: Option<PathBuf>,
}

#[derive(serde::Serialize)]
pub struct GameFilter {
    name: String,
    icon: Option<String>,
}

fn read(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

fn steam_path() -> Option<PathBuf> {
    use std::ptr::null_mut;
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let mut value = [0_u16; 2048];
    let mut bytes = size_of_val(&value) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows_sys::w!("Software\\Valve\\Steam"),
            windows_sys::w!("SteamPath"),
            RRF_RT_REG_SZ,
            null_mut(),
            value.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if status == 0 && bytes > 2 {
        return Some(PathBuf::from(String::from_utf16_lossy(
            &value[..bytes as usize / 2 - 1],
        )));
    }
    std::env::var_os("ProgramFiles(x86)").map(|path| PathBuf::from(path).join("Steam"))
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn steam_games(steam: &Path) -> Option<Vec<Game>> {
    let bytes = read(&steam.join("appcache/appinfo.vdf"), 128 * 1024 * 1024)?;
    let appinfo = std::panic::catch_unwind(|| parse_appinfo(&bytes))
        .ok()?
        .ok()?;
    let mut libraries = vec![steam.to_owned()];
    if let Some(folders) = read(&steam.join("steamapps/libraryfolders.vdf"), 1024 * 1024)
        .and_then(|bytes| String::from_utf8(bytes).ok())
        && let Ok(vdf) = parse_text(&folders)
        && let Some(entries) = vdf.get_obj(&[])
    {
        libraries.extend(entries.iter().filter_map(|(id, entry)| {
            id.parse::<u32>().ok()?;
            let path = PathBuf::from(entry.get_str(&["path"]).or_else(|| entry.as_str())?);
            path.is_absolute().then_some(path)
        }));
    }
    libraries.sort_by_key(|path| path_key(path));
    libraries.dedup_by(|a, b| path_key(a) == path_key(b));
    let mut games = Vec::new();
    for library in libraries {
        let steamapps = library.join("steamapps");
        for manifest in fs::read_dir(&steamapps).into_iter().flatten().flatten() {
            let filename = manifest.file_name();
            let Some(id) = filename
                .to_str()
                .and_then(|name| name.strip_prefix("appmanifest_"))
                .and_then(|name| name.strip_suffix(".acf"))
            else {
                continue;
            };
            if id.parse::<u32>().is_err() {
                continue;
            }
            let Some(app) = appinfo
                .get_path(&[id, "appinfo"])
                .or_else(|| appinfo.get_path(&[id]))
            else {
                continue;
            };
            if !app
                .get_str(&["common", "type"])
                .is_some_and(|kind| kind.eq_ignore_ascii_case("game"))
            {
                continue;
            }
            let Some(manifest) =
                read(&manifest.path(), 1024 * 1024).and_then(|bytes| String::from_utf8(bytes).ok())
            else {
                continue;
            };
            let Ok(manifest) = parse_text(&manifest) else {
                continue;
            };
            let Some(folder) = manifest.get_str(&["installdir"]).filter(|name| {
                matches!(
                    Path::new(name).components().collect::<Vec<_>>().as_slice(),
                    [Component::Normal(_)]
                )
            }) else {
                continue;
            };
            let install = steamapps.join("common").join(folder);
            if !install.is_dir() {
                continue;
            }
            let Some(name) = app
                .get_str(&["common", "name"])
                .or_else(|| manifest.get_str(&["name"]))
                .map(str::trim)
                .filter(|name| !name.is_empty() && name.chars().count() <= 120)
            else {
                continue;
            };
            let executables: Vec<_> = app
                .get_obj(&["config", "launch"])
                .into_iter()
                .flat_map(|launch| launch.values())
                .filter_map(|entry| entry.get_str(&["executable"]).map(Path::new))
                .filter(|path| {
                    path.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
                        && path
                            .components()
                            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
                })
                .map(|path| install.join(path))
                .collect();
            let mut aliases = vec![normalized_name(name), normalized_name(folder)];
            aliases.extend(
                executables
                    .iter()
                    .filter_map(|path| path.file_stem()?.to_str().map(normalized_name)),
            );
            let steam_icon = app
                .get_str(&["common", "icon"])
                .filter(|hash| hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(|hash| {
                    steam
                        .join("appcache/librarycache")
                        .join(id)
                        .join(format!("{hash}.jpg"))
                });
            games.push(Game {
                name: name.to_owned(),
                install: format!("{}\\", path_key(&install)),
                aliases,
                executables,
                steam_icon,
            });
        }
    }
    Some(games)
}

fn installed_games() -> Arc<Vec<Game>> {
    static CACHE: Mutex<Option<(Instant, Arc<Vec<Game>>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((scanned, games)) = &*cache
        && scanned.elapsed() < Duration::from_secs(60)
    {
        return games.clone();
    }
    let games = Arc::new(
        steam_path()
            .and_then(|path| steam_games(&path))
            .unwrap_or_default(),
    );
    *cache = Some((Instant::now(), games.clone()));
    games
}

pub fn identify(executable: &Path) -> (String, bool) {
    let path = path_key(executable);
    let installed = installed_games();
    let game = installed
        .iter()
        .filter(|game| path.starts_with(&game.install))
        .max_by_key(|game| game.install.len());
    let name = game
        .map(|game| game.name.clone())
        .unwrap_or_else(|| app_name(&executable.to_string_lossy(), Some(executable)));
    let is_game = game.is_some()
        || executable
            .ancestors()
            .skip(1)
            .take(5)
            .any(|directory| directory.join("MicrosoftGame.config").is_file());
    if is_game {
        let _ = cache_icon(&name, executable);
    }
    (name, is_game)
}

pub fn filters(sources: HashMap<String, bool>) -> HashMap<String, GameFilter> {
    if sources.is_empty() {
        return HashMap::new();
    }
    let installed = installed_games();
    sources
        .into_iter()
        .filter_map(|(source, recorded_game)| {
            let key = normalized_name(&source);
            let mut matches = installed
                .iter()
                .filter(|game| !key.is_empty() && game.aliases.contains(&key));
            let game = matches
                .next()
                .filter(|first| matches.all(|other| first.name == other.name));
            let name = game
                .map(|game| game.name.clone())
                .or_else(|| recorded_game.then(|| source.clone()))?;
            let icon = icon_data(&name).or_else(|| {
                let game = game?;
                if let Some(bytes) = game
                    .steam_icon
                    .as_deref()
                    .and_then(|path| read(path, 32 * 1024))
                    && bytes.starts_with(&[0xff, 0xd8, 0xff])
                {
                    return Some(format!("data:image/jpeg;base64,{}", STANDARD.encode(bytes)));
                }
                for executable in &game.executables {
                    let _ = cache_icon(&name, executable);
                    if let Some(icon) = icon_data(&name) {
                        return Some(icon);
                    }
                }
                None
            });
            Some((source, GameFilter { name, icon }))
        })
        .collect()
}
