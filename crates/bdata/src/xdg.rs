//! DR-3.15: XDG-based directory layout for sabr-rs state.
//!
//! Config/cache/state respect `$SABR_*_HOME` overrides, then `$XDG_*_HOME/sabr-rs`,
//! then `$HOME/.x/sabr-rs`. `data` always resolves under XDG data home (no SABR
//! override) so the bundled registry/artifact set has one stable location (spec §4.6).
//!
//! `open` creates all directories with mode 0700 (private to the user).

use std::path::PathBuf;

/// Resolved sabr-rs directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XdgDirs {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub state: PathBuf,
    pub data: PathBuf,
}

fn env_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

/// Base dir for a class: `$SABR_*_HOME` override, then `$XDG_*_HOME/sabr`, then `$HOME/.x`.
fn base_dir(override_var: &str, xdg_var: &str, home_sub: &str) -> PathBuf {
    let root = env_var(override_var)
        .map(PathBuf::from)
        .or_else(|| env_var(xdg_var).map(PathBuf::from))
        .or_else(|| env_var("HOME").map(|home| PathBuf::from(home).join(home_sub)));
    match root {
        Some(root) => root.join("sabr-rs"),
        // No HOME either: relative path keeps resolution total without panicking.
        None => PathBuf::from(home_sub).join("sabr-rs"),
    }
}

fn data_dir() -> PathBuf {
    env_var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env_var("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("sabr-rs")
}

fn create_dir(path: &PathBuf) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
}

impl XdgDirs {
    /// Resolve from the environment without touching the filesystem.
    pub fn from_env() -> Self {
        let config = base_dir("SABR_CONFIG_HOME", "XDG_CONFIG_HOME", ".config");
        let cache = base_dir("SABR_CACHE_HOME", "XDG_CACHE_HOME", ".cache");
        let state = base_dir("SABR_STATE_HOME", "XDG_STATE_HOME", ".local/state");
        let data = data_dir();
        Self { config, cache, state, data }
    }

    /// Resolve from the environment and create all directories with mode 0700.
    pub fn open() -> std::io::Result<Self> {
        let dirs = Self::from_env();
        for p in [&dirs.config, &dirs.cache, &dirs.state, &dirs.data] {
            create_dir(p)?;
        }
        Ok(dirs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn probe(_dirs: &XdgDirs, p: &Path) -> (bool, Option<u32>) {
        (p.exists(), fs::metadata(p).ok().map(|m| m.permissions().mode() & 0o777))
    }

    #[test]
    fn env_overrides_and_open() {
        // Single test fn: env manipulation must not race other env tests.
        // Env swap must stay in one serial test; `set_var`/`remove_var` are unsafe in edition 2024.
        #[allow(unused_unsafe)]
        unsafe {
            let tmp = tempfile::tempdir().expect("tmpdir");
            let root = tmp.path().to_path_buf();
            let saved: Vec<(&str, Option<std::ffi::OsString>)> = [
                "SABR_CONFIG_HOME",
                "SABR_CACHE_HOME",
                "SABR_STATE_HOME",
                "XDG_CONFIG_HOME",
                "XDG_CACHE_HOME",
                "XDG_STATE_HOME",
                "XDG_DATA_HOME",
                "HOME",
            ]
            .iter()
            .map(|&k| (k, std::env::var_os(k)))
            .collect();

            let set = |k: &str, v: Option<&str>| match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            };
            set("SABR_CONFIG_HOME", None);
            set("SABR_CACHE_HOME", None);
            set("SABR_STATE_HOME", None);
            set("XDG_CONFIG_HOME", None);
            set("XDG_CACHE_HOME", None);
            set("XDG_STATE_HOME", None);
            set("XDG_DATA_HOME", None);

            // HOME fallback for all four classes.
            set("HOME", Some(root.join("home").to_str().unwrap()));
            let dirs = XdgDirs::from_env();
            assert_eq!(dirs.config, root.join("home/.config/sabr-rs"));
            assert_eq!(dirs.cache, root.join("home/.cache/sabr-rs"));
            assert_eq!(dirs.state, root.join("home/.local/state/sabr-rs"));
            assert_eq!(dirs.data, root.join("home/.local/share/sabr-rs"));

            // SABR_* overrides beat XDG_* and HOME.
            set("SABR_CONFIG_HOME", Some(root.join("c").to_str().unwrap()));
            set("SABR_CACHE_HOME", Some(root.join("k").to_str().unwrap()));
            set("SABR_STATE_HOME", Some(root.join("s").to_str().unwrap()));
            let dirs = XdgDirs::from_env();
            assert!(dirs.config.ends_with("c/sabr-rs"));
            assert!(dirs.cache.ends_with("k/sabr-rs"));
            assert!(dirs.state.ends_with("s/sabr-rs"));
            // data has no SABR override; falls through to HOME.
            assert!(dirs.data.ends_with(".local/share/sabr-rs"));
            set("SABR_CONFIG_HOME", None);
            set("SABR_CACHE_HOME", None);
            set("SABR_STATE_HOME", None);

            // XDG_* layer beats HOME.
            set("XDG_CONFIG_HOME", Some(root.join("xc").to_str().unwrap()));
            set("XDG_CACHE_HOME", Some(root.join("xk").to_str().unwrap()));
            set("XDG_STATE_HOME", Some(root.join("xs").to_str().unwrap()));
            set("XDG_DATA_HOME", Some(root.join("xd").to_str().unwrap()));
            let dirs = XdgDirs::from_env();
            assert_eq!(dirs.config, root.join("xc/sabr-rs"));
            assert_eq!(dirs.cache, root.join("xk/sabr-rs"));
            assert_eq!(dirs.state, root.join("xs/sabr-rs"));
            assert_eq!(dirs.data, root.join("xd/sabr-rs"));

            // open() creates them all with 0700.
            let dirs = XdgDirs::open().expect("open");
            for p in [&dirs.config, &dirs.cache, &dirs.state, &dirs.data] {
                let (exists, mode) = probe(&dirs, p);
                assert!(exists, "missing {}", p.display());
                assert_eq!(mode, Some(0o700), "mode for {}", p.display());
            }

            // Restore environment.
            for (k, v) in saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
    }
}
