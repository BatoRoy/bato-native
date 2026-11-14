//! GPU backend choice, made before the renderer starts.

use std::path::Path;

/// Call first thing in `main`, before any thread starts.
///
/// wgpu probes every backend by default. The GL probe loads Mesa/LLVM
/// (~45 MB PSS) only to drop it, and the dialog backdrop blur needs Vulkan.
/// So when a Vulkan driver is installed and the user has not chosen a backend,
/// pin wgpu to Vulkan. Without a Vulkan ICD (a VM, a minimal install) wgpu
/// keeps probing and falls back to GL, so the app still starts.
pub fn prefer_vulkan() {
    if std::env::var_os("WGPU_BACKEND").is_some() || !vulkan_icd_installed() {
        return;
    }
    // SAFETY: documented precondition: called before any other thread exists.
    unsafe { std::env::set_var("WGPU_BACKEND", "vulkan") };
}

/// True if a Vulkan installable client driver manifest is present.
pub fn vulkan_icd_installed() -> bool {
    if std::env::var_os("VK_ICD_FILENAMES").is_some()
        || std::env::var_os("VK_DRIVER_FILES").is_some()
    {
        return true;
    }
    [
        "/usr/share/vulkan/icd.d",
        "/etc/vulkan/icd.d",
        "/usr/local/share/vulkan/icd.d",
    ]
    .iter()
    .any(|d| has_json(Path::new(d)))
}

fn has_json(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().extension().is_some_and(|x| x == "json"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_icd_manifests_by_extension() {
        let dir = std::env::temp_dir().join(format!("bato-icd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!has_json(&dir));
        std::fs::write(dir.join("nvidia_icd.json"), "{}").unwrap();
        assert!(has_json(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
