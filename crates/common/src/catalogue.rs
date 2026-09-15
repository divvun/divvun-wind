//! Bounded catalogue loading, shared by the daemon, DLL and installer checks.
use crate::locale::{LocaleConfig, LocalizedText};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::Read,
    path::{Path, PathBuf},
};

pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_FILES: usize = 1024;
pub const MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

// serde_json::Value normally discards repeated members. Detect them during
// deserialization, before any object can be converted into a map.
struct UniqueValue(Value);
impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON without duplicate members")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut object = serde_json::Map::new();
                let mut keys = BTreeSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.to_ascii_lowercase()) {
                        return Err(de::Error::custom(format!("duplicate member {key:?}")));
                    }
                    object.insert(key, map.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(object)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

fn tag(value: &str) -> Result<String, String> {
    // Parsing checks BCP47 grammar. Do not call validate(): registration and
    // recognition by Windows are explicitly not requirements for an override.
    value
        .parse::<language_tags::LanguageTag>()
        .map_err(|e| format!("invalid language tag {value:?}: {e}"))?;
    Ok(value.to_ascii_lowercase())
}

fn validate_text(text: &mut LocalizedText) -> Result<(), String> {
    fn label(text: &str) -> Result<(), String> {
        if text.trim().is_empty() || text.contains('\0') {
            return Err("labels must be nonempty and contain no NUL characters".into());
        }
        Ok(())
    }
    match text {
        LocalizedText::Literal(text) => label(text),
        LocalizedText::Translations(translations) => {
            if translations.is_empty() {
                return Err("translation map is empty".into());
            }
            let mut normalized = BTreeMap::new();
            for (language, text) in translations.iter() {
                label(text)?;
                normalized.insert(tag(language)?, text.clone());
            }
            *translations = normalized;
            Ok(())
        }
    }
}

pub fn parse_file(bytes: &[u8]) -> Result<LocaleConfig, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("file exceeds 256 KiB".into());
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let value: UniqueValue = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let entries: LocaleConfig = serde_json::from_value(value.0).map_err(|e| e.to_string())?;
    let mut result = LocaleConfig::new();
    for (language, mut entry) in entries {
        let language = tag(&language)?;
        if entry.abbreviation.is_none() && entry.display_name.is_none() {
            return Err(format!("{language}: entry has no label"));
        }
        for text in [&mut entry.abbreviation, &mut entry.display_name]
            .into_iter()
            .flatten()
        {
            validate_text(text).map_err(|e| format!("{language}: {e}"))?;
        }
        result.insert(language, entry);
    }
    Ok(result)
}

#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub severity: &'static str,
    pub code: &'static str,
    pub paths: Vec<PathBuf>,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub entries: LocaleConfig,
    pub sources: BTreeMap<String, Vec<PathBuf>>,
    pub diagnostics: Vec<Diagnostic>,
    pub stale_files: Vec<PathBuf>,
    pub scan_complete: bool,
}
impl Snapshot {
    pub fn valid(&self) -> bool {
        self.scan_complete && !self.diagnostics.iter().any(|d| d.severity == "error")
    }
}

fn error(code: &'static str, paths: Vec<PathBuf>, message: impl Into<String>) -> Diagnostic {
    Diagnostic {
        severity: "error",
        code,
        paths,
        message: message.into(),
    }
}

fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn candidates(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let items = match fs::read_dir(directory) {
        Ok(items) => items,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let mut paths = Vec::new();
    for item in items {
        let item = item.map_err(|e| e.to_string())?;
        let path = item.path();
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            continue;
        }
        // Count unreadable candidates too. Failure to inspect one file is not
        // failure to enumerate the directory; the per-file reader reports it.
        if item.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        paths.push(path);
        if paths.len() > MAX_FILES {
            return Err("catalogue exceeds 1024 candidate files".into());
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_file(path: &Path, total: &mut usize) -> Result<LocaleConfig, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if is_reparse(&metadata) || !metadata.is_file() {
        return Err("candidate is not a regular file (reparse points are not followed)".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("file exceeds 256 KiB".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Close the metadata/open race: open the reparse point itself, then
        // inspect this handle before reading any content from it.
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if is_reparse(&metadata) || !metadata.is_file() {
        return Err("candidate changed to a non-regular file".into());
    }
    if metadata.len() > MAX_FILE_BYTES as u64 {
        return Err("file exceeds 256 KiB".into());
    }
    let limit = (MAX_FILE_BYTES + 1).min(MAX_TOTAL_BYTES.saturating_sub(*total) + 1);
    let mut bytes = Vec::new();
    let result = file.take(limit as u64).read_to_end(&mut bytes);
    *total += bytes.len();
    result.map_err(|e| e.to_string())?;
    parse_file(&bytes)
}

pub fn check_file(path: &Path) -> Snapshot {
    let mut snapshot = Snapshot {
        scan_complete: true,
        ..Snapshot::default()
    };
    match read_file(path, &mut 0) {
        Ok(entries) => {
            snapshot.sources = entries
                .keys()
                .map(|tag| (tag.clone(), vec![path.into()]))
                .collect();
            snapshot.entries = entries;
        }
        Err(e) => snapshot
            .diagnostics
            .push(error("invalid_file", vec![path.into()], e)),
    }
    snapshot
}

#[derive(Default)]
pub struct Loader {
    accepted: BTreeMap<PathBuf, LocaleConfig>,
    snapshot: Snapshot,
}
impl Loader {
    pub fn scan(&mut self, directory: &Path) -> Snapshot {
        match self.scan_inner(directory) {
            Ok((accepted, snapshot)) => {
                self.accepted = accepted;
                self.snapshot = snapshot;
            }
            Err(message) => {
                // Preserve the whole last accepted view on an incomplete scan.
                self.snapshot.scan_complete = false;
                self.snapshot
                    .diagnostics
                    .retain(|d| d.code != "scan_unavailable");
                self.snapshot.diagnostics.push(error(
                    "scan_unavailable",
                    vec![directory.into()],
                    message,
                ));
            }
        }
        self.snapshot.clone()
    }

    fn scan_inner(
        &self,
        directory: &Path,
    ) -> Result<(BTreeMap<PathBuf, LocaleConfig>, Snapshot), String> {
        let mut accepted = BTreeMap::new();
        let mut snapshot = Snapshot {
            scan_complete: true,
            ..Snapshot::default()
        };
        let mut total = 0;
        for path in candidates(directory)? {
            let result = read_file(&path, &mut total);
            if total > MAX_TOTAL_BYTES {
                return Err("catalogue exceeds 8 MiB aggregate read limit".into());
            }
            match result {
                Ok(entries) => {
                    accepted.insert(path, entries);
                }
                Err(e) => {
                    snapshot
                        .diagnostics
                        .push(error("invalid_file", vec![path.clone()], e));
                    if let Some(previous) = self.accepted.get(&path) {
                        accepted.insert(path.clone(), previous.clone());
                        snapshot.stale_files.push(path);
                    }
                }
            }
        }
        let mut definitions: BTreeMap<String, Vec<_>> = BTreeMap::new();
        for (path, entries) in &accepted {
            for (tag, entry) in entries {
                definitions
                    .entry(tag.clone())
                    .or_default()
                    .push((path, entry));
            }
        }
        for (tag, definitions) in definitions {
            let paths: Vec<_> = definitions.iter().map(|(p, _)| (*p).clone()).collect();
            let first = definitions[0].1;
            if definitions.iter().any(|(_, entry)| *entry != first) {
                snapshot.diagnostics.push(error(
                    "conflicting_tag",
                    paths,
                    format!("conflicting definitions disable {tag}"),
                ));
                continue;
            }
            if definitions.len() > 1 {
                snapshot.diagnostics.push(Diagnostic {
                    severity: "info",
                    code: "identical_tag",
                    paths: paths.clone(),
                    message: format!("identical definitions coalesced for {tag}"),
                });
            }
            snapshot.sources.insert(tag.clone(), paths);
            snapshot.entries.insert(tag, first.clone());
        }
        Ok((accepted, snapshot))
    }
}
