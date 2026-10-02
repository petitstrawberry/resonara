//! Installed libraries are indexed without executing code. Descriptor inspection
//! happens only when the user opens/rescans the CLAP picker, on the control thread.
use super::*;
use std::{collections::BTreeMap, path::Path, sync::Mutex};

#[derive(Clone, Debug, PartialEq)]
pub struct ClapChoice {
    pub library: String,
    pub plugin_id: String,
    pub name: String,
    pub vendor: String,
}

#[derive(Default)]
pub struct ClapCatalog {
    pub effects: Vec<ClapChoice>,
    pub warnings: Vec<String>,
}

#[derive(Default, Clone)]
struct Index {
    roots: Vec<PathBuf>,
    libraries: BTreeMap<String, BTreeSet<PathBuf>>,
    warnings: Vec<String>,
}
static INDEX: Mutex<Option<Index>> = Mutex::new(None);
const MAX_ENTRIES: usize = 16_384;
const MAX_LIBRARIES: usize = 1024;

fn roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(paths) = std::env::var_os("CLAP_PATH") {
        roots.extend(std::env::split_paths(&paths).filter(|p| p.is_absolute()));
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        roots.push(directory.join("plugins"));
    }
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/CLAP"));
        }
    }
    #[cfg(any(target_os = "linux", target_os = "scarlet"))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        roots.extend(unix_roots(home.as_deref()));
    }
    roots
}

#[cfg(any(test, target_os = "linux", target_os = "scarlet"))]
fn unix_roots(home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = vec!["/usr/lib/clap".into(), "/usr/local/lib/clap".into()];
    if let Some(home) = home.filter(|p| p.is_absolute()) {
        roots.push(home.join(".clap"));
        roots.push(home.join(".local/lib/clap"));
    }
    roots
}

fn identity(name: &str) -> bool {
    name.ends_with(".clap") && !name.contains(['/', '\\', '\0']) && name.len() <= MAX_TEXT_BYTES
}

fn index(roots: Vec<PathBuf>) -> Index {
    let mut result = Index {
        roots: roots.clone(),
        ..Default::default()
    };
    let mut pending: Vec<_> = roots.into_iter().map(|p| (p, 0)).collect();
    let mut visited = BTreeSet::new();
    let mut entries = 0;
    let mut truncated = false;
    while let Some((directory, depth)) = pending.pop() {
        let Ok(canonical) = directory.canonicalize() else {
            continue;
        };
        if !visited.insert(canonical) {
            continue;
        }
        let children = match std::fs::read_dir(&directory) {
            Ok(children) => children,
            Err(error) => {
                result
                    .warnings
                    .push(format!("{}: {error}", directory.display()));
                continue;
            }
        };
        for child in children {
            entries += 1;
            if entries > MAX_ENTRIES || result.libraries.len() >= MAX_LIBRARIES {
                result
                    .warnings
                    .push("CLAP search limit reached; narrow CLAP_PATH".into());
                // A partial inventory cannot establish that a basename is unique.
                result.libraries.clear();
                return result;
            }
            let child = match child {
                Ok(child) => child,
                Err(error) => {
                    result.warnings.push(error.to_string());
                    continue;
                }
            };
            let path = child.path();
            if let Some(name) = path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| identity(n))
            {
                match path.canonicalize() {
                    Ok(binary) => {
                        result
                            .libraries
                            .entry(name.into())
                            .or_default()
                            .insert(binary);
                    }
                    Err(error) => result.warnings.push(format!("{name}: {error}")),
                }
            } else if path.is_dir() {
                if depth < 32 {
                    pending.push((path, depth + 1));
                } else {
                    truncated = true;
                    result
                        .warnings
                        .push(format!("CLAP search depth exceeded: {}", path.display()));
                }
            }
        }
    }
    if truncated {
        result.libraries.clear();
    }
    result
}

fn refresh_index() -> Index {
    let mut cache = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    *cache = Some(index(roots()));
    cache.as_ref().unwrap().clone()
}

fn resolve_index(index: &Index, library: &str) -> Result<PathBuf> {
    if !identity(library) {
        return Err("Saved CLAP library must be a .clap basename".into());
    }
    let paths = index
        .libraries
        .get(library)
        .ok_or("CLAP library is not installed; rescan after installing it")?;
    if paths.len() != 1 {
        return Err(
            format!("Ambiguous CLAP library {library}; remove duplicate installations").into(),
        );
    }
    let path = paths.first().unwrap();
    if !installed(path) {
        return Err("CLAP library is no longer installed".into());
    }
    Ok(path.clone())
}

pub(super) fn resolve(library: &str) -> Result<PathBuf> {
    // Validate before touching the filesystem; project data never becomes a path.
    if !identity(library) {
        return Err("Saved CLAP library must be a .clap basename".into());
    }
    let roots = roots();
    let mut cache = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if cache.as_ref().is_none_or(|i| i.roots != roots) {
        *cache = Some(index(roots));
    }
    resolve_index(cache.as_ref().unwrap(), library)
}

/// Refresh installed locations and inspect descriptors. Native plug-ins execute
/// trusted code during inspection; call only on the owner/control thread.
pub fn scan_installed() -> ClapCatalog {
    let index = refresh_index();
    catalog(&index)
}

fn catalog(index: &Index) -> ClapCatalog {
    let mut result = ClapCatalog {
        warnings: index.warnings.clone(),
        ..Default::default()
    };
    for library in index
        .libraries
        .keys()
        .filter(|n| n.as_str() != BUNDLED_GAIN_LIBRARY)
    {
        let descriptors =
            resolve_index(index, library).and_then(|path| Ok(resonara_clap::discover(&path)?));
        match descriptors {
            Ok(descriptors) => {
                for descriptor in descriptors {
                    if descriptor.features.iter().any(|f| f == "audio-effect")
                        && !descriptor
                            .features
                            .iter()
                            .any(|f| f == "instrument" || f == "note-effect")
                    {
                        if result.effects.len() >= MAX_LIBRARIES {
                            result
                                .warnings
                                .push("CLAP effect listing limit reached".into());
                            return result;
                        }
                        result.effects.push(ClapChoice {
                            library: library.clone(),
                            plugin_id: descriptor.id,
                            name: descriptor.name,
                            vendor: descriptor.vendor,
                        });
                    }
                }
            }
            Err(error) => result.warnings.push(format!("{library}: {error}")),
        }
    }
    result.effects.sort_by(|a, b| {
        (&a.name, &a.library, &a.plugin_id).cmp(&(&b.name, &b.library, &b.plugin_id))
    });
    result
}

fn installed(path: &Path) -> bool {
    path.is_file() || (cfg!(target_os = "macos") && path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_search_includes_system_and_user_installations_without_relative_home() {
        assert_eq!(
            unix_roots(Some(Path::new("/home/musician"))),
            [
                "/usr/lib/clap",
                "/usr/local/lib/clap",
                "/home/musician/.clap",
                "/home/musician/.local/lib/clap",
            ]
            .map(PathBuf::from)
        );
        assert_eq!(unix_roots(Some(Path::new("relative"))), unix_roots(None));
    }
    struct Temp(PathBuf);
    impl Temp {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("resonara-catalog-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn recursive_inventory_rejects_paths_collisions_and_removed_libraries() {
        let temp = Temp::new("index");
        let nested = temp.0.join("vendor/subfolder");
        std::fs::create_dir_all(&nested).unwrap();
        let library = nested.join("effect.clap");
        std::fs::write(&library, b"not executable").unwrap();
        std::fs::write(nested.join("ignored.so"), b"not clap").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&temp.0, nested.join("cycle")).unwrap();
            std::fs::create_dir(temp.0.join("alias")).unwrap();
            std::os::unix::fs::symlink(&library, temp.0.join("alias/effect.clap")).unwrap();
        }
        let inventory = index(vec![temp.0.clone(), temp.0.clone()]);
        assert_eq!(inventory.libraries.len(), 1);
        assert_eq!(
            resolve_index(&inventory, "effect.clap").unwrap(),
            library.canonicalize().unwrap()
        );
        for invalid in [
            "../effect.clap",
            "/tmp/effect.clap",
            "sub/effect.clap",
            "sub\\effect.clap",
            "effect.so",
        ] {
            assert!(resolve_index(&inventory, invalid).is_err());
        }
        std::fs::create_dir(temp.0.join("other")).unwrap();
        std::fs::write(temp.0.join("other/effect.clap"), b"another library").unwrap();
        let duplicates = index(vec![temp.0.clone()]);
        assert!(
            resolve_index(&duplicates, "effect.clap")
                .unwrap_err()
                .to_string()
                .contains("Ambiguous")
        );
        std::fs::remove_file(&library).unwrap();
        assert!(resolve_index(&inventory, "effect.clap").is_err());
    }

    #[test]
    fn invalid_native_library_is_reported_without_hiding_other_inventory_entries() {
        let temp = Temp::new("invalid");
        std::fs::write(temp.0.join("invalid.clap"), b"not an ELF or Mach-O").unwrap();
        let result = catalog(&index(vec![temp.0.clone()]));
        assert!(result.effects.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("invalid.clap"));
    }

    #[test]
    fn truncated_search_does_not_resolve_from_a_partial_inventory() {
        let temp = Temp::new("depth");
        std::fs::write(temp.0.join("effect.clap"), b"not executable").unwrap();
        let mut nested = temp.0.clone();
        for _ in 0..34 {
            nested = nested.join("deep");
        }
        std::fs::create_dir_all(nested).unwrap();
        let result = index(vec![temp.0.clone()]);
        assert!(result.libraries.is_empty());
        assert!(result.warnings.iter().any(|w| w.contains("depth exceeded")));
    }
}
