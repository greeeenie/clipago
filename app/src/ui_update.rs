use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    io::Read,
    sync::{Arc, RwLock},
};
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};

const HOST_FILE: &str = "clipago-ui-host.txt";
const MAX_BUNDLE_SIZE: usize = 24 * 1024 * 1024;
const MAX_CACHE_SIZE: u64 = (MAX_BUNDLE_SIZE as u64 * 4 / 3) + 4096;

#[derive(Deserialize)]
struct Bundle {
    host: String,
    version: Version,
    files: BTreeMap<String, String>,
}

struct ActiveUi {
    version: Version,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Serialize, Deserialize)]
struct SignedBundle {
    signature: String,
    payload: String,
}

pub struct UiUpdates {
    native_version: Version,
    host: Option<String>,
    active: Arc<RwLock<Option<ActiveUi>>>,
}

impl UiUpdates {
    pub fn load(context: &mut tauri::Context<tauri::Wry>) -> Self {
        let host = context
            .assets()
            .get(&HOST_FILE.into())
            .and_then(|bytes| String::from_utf8(bytes.into_owned()).ok())
            .filter(|id| id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit()));
        let ui = Self {
            native_version: context.package_info().version.clone(),
            host,
            active: Arc::default(),
        };
        if ui.host.is_some() {
            let pubkey = context
                .config()
                .plugins
                .0
                .get("updater")
                .and_then(|config| config["pubkey"].as_str());
            if let Some(active) = pubkey.and_then(|key| ui.read_cached(key).ok()) {
                *ui.active.write().unwrap() = Some(active);
            }
            let embedded = context.set_assets(Box::new(EmptyAssets));
            context.set_assets(Box::new(Overlay {
                embedded,
                active: ui.active.clone(),
            }));
        }
        ui
    }

    pub fn current_version(&self) -> Version {
        self.active
            .read()
            .unwrap()
            .as_ref()
            .map_or(&self.native_version, |ui| &ui.version)
            .clone()
    }

    pub fn target(&self, update: &tauri_plugin_updater::Update) -> Option<String> {
        let target = format!("clipago-ui-{}", self.host.as_ref()?);
        update.raw_json["platforms"].get(&target).map(|_| target)
    }

    pub fn apply(&self, bytes: &[u8], signature: String, version: &str) -> Result<()> {
        let active = self.decode(bytes)?;
        ensure!(
            active.version == Version::parse(version)?,
            "Unexpected UI version"
        );
        ensure!(
            active.version > self.current_version(),
            "UI update is not newer"
        );
        let cache = rewind::settings::data_file("ui-update.json")?;
        let temp = cache.with_extension("tmp");
        fs::create_dir_all(cache.parent().context("Missing UI cache directory")?)?;
        let signed = SignedBundle {
            signature,
            payload: STANDARD.encode(bytes),
        };
        fs::write(&temp, serde_json::to_vec(&signed)?)?;
        fs::rename(temp, cache)?;
        *self.active.write().unwrap() = Some(active);
        Ok(())
    }

    fn decode(&self, bytes: &[u8]) -> Result<ActiveUi> {
        ensure!(bytes.len() <= MAX_BUNDLE_SIZE, "UI package is too large");
        let bundle: Bundle = serde_json::from_slice(bytes)?;
        ensure!(
            self.host.as_deref() == Some(bundle.host.as_str()),
            "Incompatible UI package"
        );
        ensure!(
            bundle.version > self.native_version,
            "UI package is too old"
        );
        ensure!(
            bundle.files.contains_key("index.html"),
            "Missing UI entry point"
        );
        let files = bundle
            .files
            .into_iter()
            .map(|(key, value)| {
                ensure!(
                    !key.contains('\\')
                        && key.split('/').all(|part| !matches!(part, "" | "." | "..")),
                    "Invalid UI asset path"
                );
                Ok((key, STANDARD.decode(value)?))
            })
            .collect::<Result<_>>()?;
        Ok(ActiveUi {
            version: bundle.version,
            files,
        })
    }

    fn read_cached(&self, pubkey: &str) -> Result<ActiveUi> {
        let path = rewind::settings::data_file("ui-update.json")?;
        let mut contents = Vec::new();
        fs::File::open(path)?
            .take(MAX_CACHE_SIZE + 1)
            .read_to_end(&mut contents)?;
        ensure!(
            contents.len() as u64 <= MAX_CACHE_SIZE,
            "UI cache is too large"
        );
        let signed: SignedBundle = serde_json::from_slice(&contents)?;
        let bytes = STANDARD.decode(signed.payload)?;
        let key = PublicKey::decode(&String::from_utf8(STANDARD.decode(pubkey)?)?)?;
        let signature = Signature::decode(&String::from_utf8(STANDARD.decode(signed.signature)?)?)?;
        key.verify(&bytes, &signature, true)?;
        self.decode(&bytes)
    }
}

struct Overlay {
    embedded: Box<dyn tauri::Assets<tauri::Wry>>,
    active: Arc<RwLock<Option<ActiveUi>>>,
}

impl tauri::Assets<tauri::Wry> for Overlay {
    fn setup(&self, app: &tauri::App) {
        self.embedded.setup(app);
    }

    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        if let Some(asset) = self
            .active
            .read()
            .unwrap()
            .as_ref()
            .and_then(|ui| ui.files.get(key.as_ref().trim_start_matches('/')))
        {
            return Some(Cow::Owned(asset.clone()));
        }
        self.embedded.get(key)
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.embedded.iter()
    }

    fn csp_hashes(&self, key: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        self.embedded.csp_hashes(key)
    }
}

struct EmptyAssets;

impl tauri::Assets<tauri::Wry> for EmptyAssets {
    fn get(&self, _: &AssetKey) -> Option<Cow<'_, [u8]>> {
        None
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}
