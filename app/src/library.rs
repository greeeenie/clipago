use crate::media::Clip;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Deserialize, Serialize)]
struct Entry {
    original: Option<PathBuf>,
    viewed: bool,
    #[serde(default)]
    movie: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}
#[derive(Default, Deserialize, Serialize)]
pub struct Catalog {
    entries: BTreeMap<PathBuf, Entry>,
    #[serde(skip)]
    file: PathBuf,
}
impl Catalog {
    pub fn load(file: PathBuf) -> Result<Self> {
        let mut catalog: Self = if file.exists() {
            serde_json::from_slice(&std::fs::read(&file)?)?
        } else {
            Self::default()
        };
        catalog.file = file;
        Ok(catalog)
    }
    fn save(&self) -> Result<()> {
        std::fs::create_dir_all(self.file.parent().unwrap())?;
        let temp = self.file.with_extension("tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(temp, &self.file)?;
        Ok(())
    }
    pub fn paths(&self) -> Vec<PathBuf> {
        self.entries.keys().cloned().collect()
    }
    pub fn decorate(&self, clip: &mut Clip) {
        if let Some(entry) = self.entries.get(&clip.path) {
            clip.original_path = entry.original.clone();
            clip.viewed = entry.viewed;
            clip.is_movie = entry.movie;
            clip.title.clone_from(&entry.title);
        }
    }
    pub fn register(&mut self, path: &Path) -> Result<()> {
        if !self.entries.contains_key(path) {
            self.entries.insert(path.to_owned(), Entry::default());
            self.save()?;
        }
        Ok(())
    }
    pub fn edited(&mut self, path: &Path, source: &Path, title: Option<String>) -> Result<()> {
        let original = self
            .entries
            .get(source)
            .and_then(|e| e.original.clone())
            .unwrap_or_else(|| source.to_owned());
        self.entries.entry(original.clone()).or_default().viewed = true;
        self.entries.insert(
            path.to_owned(),
            Entry {
                original: Some(original),
                viewed: true,
                movie: false,
                title,
            },
        );
        self.save()
    }
    pub fn movie(&mut self, path: &Path) -> Result<()> {
        self.entries.insert(
            path.to_owned(),
            Entry {
                original: None,
                viewed: true,
                movie: true,
                title: None,
            },
        );
        self.save()
    }
    pub fn viewed(&mut self, path: &Path) -> Result<()> {
        let root = self.entries.get(path).and_then(|e| e.original.clone());
        self.entries.entry(path.to_owned()).or_default().viewed = true;
        if let Some(root) = root {
            self.entries.entry(root).or_default().viewed = true;
        }
        self.save()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versions_and_viewed_state_survive_restart_without_touching_sources() {
        let work = crate::media::WorkDir::new(&std::env::temp_dir(), "rewind-library").unwrap();
        let raw = work.0.join("raw.mp4");
        let edit = work.0.join("edit.mp4");
        let edit2 = work.0.join("edit2.mp4");
        std::fs::write(&raw, b"original").unwrap();
        let file = work.0.join("library.json");
        let mut catalog = Catalog::load(file.clone()).unwrap();
        catalog.register(&raw).unwrap();
        assert!(!catalog.entries[&raw].viewed);
        catalog.edited(&edit, &raw, None).unwrap();
        catalog.edited(&edit2, &edit, None).unwrap();
        catalog.viewed(&edit).unwrap();
        let loaded = Catalog::load(file).unwrap();
        assert_eq!(loaded.entries[&edit2].original.as_ref(), Some(&raw));
        assert!(loaded.entries[&raw].viewed);
        assert_eq!(std::fs::read(raw).unwrap(), b"original");
    }
}
