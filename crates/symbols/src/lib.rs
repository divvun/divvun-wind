pub mod image;
#[cfg(windows)]
pub mod loaded;
#[cfg(all(windows, feature = "resolver"))]
pub mod resolver;

use image::{Headers, Identity};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MODULES: [&str; 2] = ["SystemTray.dll", "Taskbar.View.dll"];
pub const BUILD_SYMBOL: &str = "?BuildEnabledLanguagesList@LanguageSystemTrayIconDataModel@implementation@SystemTray@winrt@@AEAAXXZ";
pub const UPDATE_SYMBOL: &str =
    "?UpdateCurrentData@LanguageSystemTrayIconDataModel@implementation@SystemTray@winrt@@AEAAXXZ";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Symbol {
    pub name: String,
    pub rva: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cache {
    pub schema: u32,
    pub module: String,
    pub identity: Identity,
    pub build: Symbol,
    pub update: Symbol,
}

impl Cache {
    pub fn validate(
        &self,
        module: &str,
        headers: &Headers,
        identity: &Identity,
    ) -> Result<(), String> {
        if self.schema != 1 {
            return Err("unsupported symbol cache schema".into());
        }
        if !MODULES.iter().any(|n| n.eq_ignore_ascii_case(module))
            || !self.module.eq_ignore_ascii_case(module)
        {
            return Err("symbol cache is for a different module".into());
        }
        if &self.identity != identity {
            return Err("symbol cache does not match loaded image identity".into());
        }
        if self.build.name != BUILD_SYMBOL || self.update.name != UPDATE_SYMBOL {
            return Err("symbol cache has wrong names or overloads".into());
        }
        if self.build.rva == self.update.rva
            || !headers.executable(self.build.rva)
            || !headers.executable(self.update.rva)
        {
            return Err("symbol addresses must be distinct and in executable sections".into());
        }
        Ok(())
    }
}

pub fn cache_path(root: &Path, module: &str, identity: &Identity) -> PathBuf {
    root.join(format!(
        "{}-{}.json",
        module.to_ascii_lowercase(),
        identity.key()
    ))
}

pub fn default_cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Divvun")
        .join("Wind")
        .join("taskbar-symbols")
}

pub fn read_cache(
    root: &Path,
    module: &str,
    headers: &Headers,
    identity: &Identity,
) -> Result<Cache, String> {
    let path = cache_path(root, module, identity);
    let mut file = std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    use std::io::Read;
    let mut data = Vec::new();
    file.by_ref()
        .take(65537)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > 65536 {
        return Err("symbol cache exceeds 64 KiB".into());
    }
    let cache: Cache = serde_json::from_slice(&data)
        .map_err(|e| format!("invalid cache {}: {e}", path.display()))?;
    cache.validate(module, headers, identity)?;
    Ok(cache)
}

pub fn write_cache(root: &Path, cache: &Cache) -> Result<PathBuf, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let path = cache_path(root, &cache.module, &cache.identity);
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let data = serde_json::to_vec_pretty(cache).map_err(|e| e.to_string())?;
    std::fs::write(&temp, data).map_err(|e| e.to_string())?;
    // Windows rename replaces the destination atomically; readers never see a partial JSON file.
    let result = std::fs::rename(&temp, &path).map_err(|e| e.to_string());
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    Ok(path)
}
