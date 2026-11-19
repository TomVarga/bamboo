use std::fs;
use std::path::{Path, PathBuf};

pub fn settings_path() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bamboo")
        .join("settings.json")
}

pub fn load_key(path: &Path) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return String::new();
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|value| value.get("api-key")?.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub fn save_key(path: &Path, key: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let body = serde_json::to_vec_pretty(&serde_json::json!({ "api-key": key }))
        .map_err(|err| err.to_string())?;
    let tmp = sibling(path, ".tmp");
    fs::write(&tmp, body).map_err(|err| err.to_string())?;
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        err.to_string()
    })
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch() -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bamboo-settings-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_is_an_empty_key() {
        let dir = scratch();
        assert_eq!(load_key(&dir.join("settings.json")), "");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn roundtrip_and_corrupt_file() {
        let dir = scratch();
        let path = dir.join("settings.json");
        save_key(&path, "abc-123").unwrap();
        assert_eq!(load_key(&path), "abc-123");
        save_key(&path, "key \"quoted\" 🌱").unwrap();
        assert_eq!(load_key(&path), "key \"quoted\" 🌱");
        fs::write(&path, "not json").unwrap();
        assert_eq!(load_key(&path), "");
        fs::write(&path, "{}").unwrap();
        assert_eq!(load_key(&path), "");
        assert!(!sibling(&path, ".tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }
}
