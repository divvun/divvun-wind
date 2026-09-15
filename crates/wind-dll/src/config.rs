use wind_common::catalogue::{Loader, Snapshot};
pub use wind_common::locale::LocaleConfig;

pub fn load_config() -> Result<Snapshot, String> {
    Ok(Loader::default().scan(&wind_common::locale::catalogue_directory()?))
}
