// Fighters Guild Launcher — install/sync logic.
//
// Installs the Fighters Guild modpack on top of an official Minecraft
// installation, and leaves the official Minecraft Launcher to handle
// everything about login. No Microsoft/Xbox auth code anywhere in this
// tool, on purpose — same design as the original CLI downloader this was
// ported from (fightersguild-downloader), just with a GUI wrapped around it.
//
// Flow:
//   1. Find the official launcher's .minecraft folder. Require that the
//      user has already launched vanilla 1.20.1 through it once — that's
//      what actually downloads Java + the base game files.
//   2. Run NeoForge's own installer in --installClient mode. It writes the
//      version files AND merges the launcher_profiles.json entry itself.
//   3. Fetch manifest.json (hosted on the Fighters Guild Portal), diff it
//      against what was installed last time, and download/remove files
//      accordingly.
//   4. Create/update a desktop shortcut to the real Minecraft Launcher.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::Emitter;

const MANIFEST_URL: &str = "https://fightersguild.playit.quest/downloads/manifest.json";
const MINECRAFT_VERSION: &str = "1.20.1";
const FORGE_VERSION: &str = "47.1.84";

fn forge_installer_url() -> String {
    // This server actually runs NeoForge, not classic Minecraft Forge —
    // NeoForge's early 1.20.1 builds kept Forge's artifact naming/installer
    // for migration compatibility, which is why the version string looks
    // like a classic Forge one.
    format!(
        "https://maven.neoforged.net/releases/net/neoforged/forge/{mc}-{fg}/forge-{mc}-{fg}-installer.jar",
        mc = MINECRAFT_VERSION,
        fg = FORGE_VERSION
    )
}

#[derive(Deserialize)]
struct ManifestFile {
    path: String,
    url: String,
    sha256: String,
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "packVersion")]
    pack_version: String,
    files: Vec<ManifestFile>,
}

#[derive(Serialize, Deserialize, Default)]
struct State {
    #[serde(rename = "installedFiles", default)]
    installed_files: HashMap<String, String>,
    #[serde(rename = "packVersion", default)]
    pack_version: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusReport {
    pub vanilla_installed: bool,
    /// "install" (never synced before), "update" (synced before, manifest has
    /// moved on), or "ready" (already up to date) — meaningless if
    /// vanilla_installed is false.
    pub action: String,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
}

pub async fn check_status() -> Result<StatusReport, String> {
    let mc_dir = minecraft_dir()?;
    let vanilla_installed = mc_dir.join("versions").join(MINECRAFT_VERSION).exists();
    if !vanilla_installed {
        return Ok(StatusReport {
            vanilla_installed: false,
            action: "install".to_string(),
            installed_version: None,
            latest_version: None,
        });
    }

    let state = load_state();
    let client = reqwest::Client::new();
    let latest_version = match download_bytes(&client, MANIFEST_URL).await {
        Ok(bytes) => serde_json::from_slice::<Manifest>(&bytes).ok().map(|m| m.pack_version),
        Err(_) => None,
    };

    let action = match (&state.pack_version, &latest_version) {
        (None, _) => "install",
        (Some(cur), Some(latest)) if cur != latest => "update",
        (Some(_), None) => "ready", // can't reach the manifest — don't block Play on that
        _ => "ready",
    };

    Ok(StatusReport {
        vanilla_installed: true,
        action: action.to_string(),
        installed_version: state.pack_version,
        latest_version,
    })
}

fn emit_progress(window: &tauri::Window, message: &str, percent: f64) {
    let _ = window.emit(
        "install-progress",
        serde_json::json!({ "message": message, "percent": percent }),
    );
}

fn minecraft_dir() -> Result<PathBuf, String> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| "Could not resolve %APPDATA% — is this actually Windows?".to_string())?;
    Ok(PathBuf::from(appdata).join(".minecraft"))
}

fn require_vanilla_installed(mc_dir: &Path) -> Result<(), String> {
    let version_dir = mc_dir.join("versions").join(MINECRAFT_VERSION);
    if !version_dir.exists() {
        return Err(format!(
            "Minecraft {mc} isn't installed yet. Open the official Minecraft Launcher, select version {mc}, and click Play once (this downloads Java and the base game files), then try again.",
            mc = MINECRAFT_VERSION
        ));
    }
    Ok(())
}

// Reuse the Java runtime the official launcher already downloaded, rather
// than bundling our own.
fn find_java(mc_dir: &Path) -> String {
    let runtime_dir = mc_dir.join("runtime");
    if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
        for entry in entries.flatten() {
            let candidate = entry
                .path()
                .join("windows-x64")
                .join(entry.file_name())
                .join("bin")
                .join("javaw.exe");
            if candidate.exists() {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    "java".to_string() // fall back to PATH
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

async fn download_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GET {url} -> {e}"))?;
    if !res.status().is_success() {
        return Err(format!("GET {url} -> {}", res.status()));
    }
    res.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("reading response from {url}: {e}"))
}

async fn install_forge(window: &tauri::Window, mc_dir: &Path, java_path: &str) -> Result<(), String> {
    // The installer is idempotent — rerunning it over an existing install
    // just checksum-verifies the files and exits quickly, so we always run it.
    let tmp_dir = std::env::temp_dir().join("fightersguild-launcher");
    std::fs::create_dir_all(&tmp_dir).map_err(|e| e.to_string())?;
    let installer_path = tmp_dir.join(format!("forge-{FORGE_VERSION}-installer.jar"));

    emit_progress(window, "Downloading the NeoForge installer...", 10.0);
    let client = reqwest::Client::new();
    let bytes = download_bytes(&client, &forge_installer_url()).await?;
    std::fs::write(&installer_path, &bytes).map_err(|e| e.to_string())?;

    emit_progress(window, "Installing NeoForge into your Minecraft Launcher...", 20.0);
    let java_path = java_path.to_string();
    let installer_path_str = installer_path.to_string_lossy().to_string();
    let mc_dir_str = mc_dir.to_string_lossy().to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&java_path)
            .args(["-jar", &installer_path_str, "--installClient", &mc_dir_str])
            .output()
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("failed to run the NeoForge installer: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("NeoForge installer failed: {stderr}"));
    }
    emit_progress(window, "NeoForge installed.", 30.0);
    rebrand_profile(mc_dir)?;
    Ok(())
}

// NeoForge's installer names the profile it creates "forge" with its own
// grass-block-and-cog icon. Rename it and swap the icon so it reads
// "Fighters Guild" in the official launcher's own UI, and bump lastUsed so
// it sorts to the top there (launcher_profiles.json's own settings default
// to sorting profiles "ByLastPlayed").
fn rebrand_profile(mc_dir: &Path) -> Result<(), String> {
    let path = mc_dir.join("launcher_profiles.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading launcher_profiles.json: {e}"))?;
    let mut json: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("parsing launcher_profiles.json: {e}"))?;

    let profiles = json
        .get_mut("profiles")
        .and_then(|p| p.as_object_mut())
        .ok_or("launcher_profiles.json has no \"profiles\" object")?;

    let target_key = profiles
        .iter()
        .find(|(_, p)| {
            p.get("lastVersionId")
                .and_then(|v| v.as_str())
                .map(|v| v.starts_with(&format!("{MINECRAFT_VERSION}-forge-")))
                .unwrap_or(false)
        })
        .map(|(k, _)| k.clone());

    let Some(key) = target_key else {
        // Not fatal — the profile still works under whatever name NeoForge gave it.
        return Ok(());
    };

    if let Some(profile) = profiles.get_mut(&key).and_then(|p| p.as_object_mut()) {
        profile.insert("name".to_string(), serde_json::Value::String("Fighters Guild".to_string()));
        profile.insert(
            "lastUsed".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        );
        let icon_bytes = include_bytes!("../icons/128x128.png");
        let icon_b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, icon_bytes);
        profile.insert(
            "icon".to_string(),
            serde_json::Value::String(format!("data:image/png;base64,{icon_b64}")),
        );
        // NeoForge's installer doesn't set a memory override on the profile
        // it creates, so the launcher falls back to its own global default
        // (2 GB on a fresh account) — nowhere near enough for a 300+ mod
        // pack, and it crashes on load without this. Always enforced, same
        // as name/icon/lastUsed above, so it stays correct across updates
        // even if NeoForge's installer ever changes what it writes here.
        profile.insert(
            "javaArgs".to_string(),
            serde_json::Value::String(
                "-Xmx8G -XX:+UnlockExperimentalVMOptions -XX:+UseG1GC -XX:G1NewSizePercent=20 -XX:G1ReservePercent=20 -XX:MaxGCPauseMillis=50 -XX:G1HeapRegionSize=32M".to_string(),
            ),
        );
    }

    let updated = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
    std::fs::write(&path, updated).map_err(|e| format!("writing launcher_profiles.json: {e}"))?;
    Ok(())
}

fn state_file_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().to_string());
    PathBuf::from(base).join("FightersGuildLauncher").join("state.json")
}

fn load_state() -> State {
    std::fs::read_to_string(state_file_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_state(state: &State) -> Result<(), String> {
    let path = state_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

async fn sync_modpack(window: &tauri::Window, mc_dir: &Path) -> Result<(), String> {
    emit_progress(window, "Fetching the modpack manifest...", 32.0);
    let client = reqwest::Client::new();
    let manifest_bytes = download_bytes(&client, MANIFEST_URL).await?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("could not parse manifest.json: {e}"))?;

    let mut state = load_state();
    let mut new_installed: HashMap<String, String> = HashMap::new();
    let manifest_paths: std::collections::HashSet<&str> =
        manifest.files.iter().map(|f| f.path.as_str()).collect();

    let total = manifest.files.len().max(1);
    let base_url = reqwest::Url::parse(MANIFEST_URL).map_err(|e| e.to_string())?;

    for (i, entry) in manifest.files.iter().enumerate() {
        let dest_path = mc_dir.join(&entry.path);
        let previously_installed = state.installed_files.get(&entry.path);
        let percent = 32.0 + (i as f64 / total as f64) * 60.0;

        if previously_installed == Some(&entry.sha256) && dest_path.exists() {
            new_installed.insert(entry.path.clone(), entry.sha256.clone());
            continue;
        }

        emit_progress(window, &format!("Downloading {}", entry.path), percent);
        let absolute_url = base_url
            .join(&entry.url)
            .map_err(|e| format!("bad file URL for {}: {e}", entry.path))?;
        let bytes = download_bytes(&client, absolute_url.as_str()).await?;

        let actual_hash = sha256_hex(&bytes);
        if actual_hash != entry.sha256 {
            return Err(format!(
                "hash mismatch for {} — expected {}, got {actual_hash}. Aborting rather than installing a corrupt/tampered file.",
                entry.path, entry.sha256
            ));
        }
        if let Some(parent) = dest_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&dest_path, &bytes).map_err(|e| e.to_string())?;
        new_installed.insert(entry.path.clone(), actual_hash);
    }

    // Remove files that were part of a previous sync but aren't in this
    // manifest anymore.
    let stale: Vec<String> = state
        .installed_files
        .keys()
        .filter(|p| !manifest_paths.contains(p.as_str()))
        .cloned()
        .collect();
    for old_path in stale {
        let full_path = mc_dir.join(&old_path);
        let _ = std::fs::remove_file(&full_path);
    }

    state.installed_files = new_installed;
    state.pack_version = Some(manifest.pack_version.clone());
    save_state(&state)?;

    emit_progress(
        window,
        &format!(
            "Modpack sync complete (v{}, {} files).",
            manifest.pack_version,
            manifest.files.len()
        ),
        95.0,
    );
    Ok(())
}

pub fn find_official_launcher_exe() -> Option<PathBuf> {
    let candidates = [
        std::env::var("ProgramFiles(x86)").ok(),
        std::env::var("ProgramFiles").ok(),
    ];
    for base in candidates.into_iter().flatten() {
        let candidate = PathBuf::from(base)
            .join("Minecraft Launcher")
            .join("MinecraftLauncher.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn create_desktop_shortcut(target_exe: &Path) -> Result<String, String> {
    // Not %USERPROFILE%\Desktop — OneDrive's "Known Folder Move" silently
    // relocates Desktop to <homedir>\OneDrive\Desktop on plenty of real
    // machines, and that old path just doesn't exist there. Ask Windows for
    // the real, current Desktop location instead.
    let target = target_exe.to_string_lossy().replace('\'', "''");
    let ps = format!(
        r#"
        $desktop = [Environment]::GetFolderPath('Desktop')
        $shortcutPath = Join-Path $desktop 'Fighters Guild.lnk'
        $s = (New-Object -COMObject WScript.Shell).CreateShortcut($shortcutPath)
        $s.TargetPath = '{target}'
        $s.Description = 'Fighters Guild - Minecraft'
        $s.Save()
        Write-Output $shortcutPath
        "#
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .output()
        .map_err(|e| format!("failed to run powershell: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not create the desktop shortcut: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub async fn run(window: tauri::Window) -> Result<(), String> {
    emit_progress(&window, "Looking for your Minecraft install...", 2.0);
    let mc_dir = minecraft_dir()?;
    require_vanilla_installed(&mc_dir)?;

    let java_path = find_java(&mc_dir);
    install_forge(&window, &mc_dir, &java_path).await?;
    sync_modpack(&window, &mc_dir).await?;

    if let Some(launcher_exe) = find_official_launcher_exe() {
        emit_progress(&window, "Creating a desktop shortcut...", 97.0);
        let shortcut = create_desktop_shortcut(&launcher_exe)?;
        emit_progress(&window, &format!("Shortcut ready: {shortcut}"), 99.0);
    } else {
        emit_progress(
            &window,
            "Could not find the official Minecraft Launcher to shortcut, but everything else installed fine.",
            99.0,
        );
    }

    let _ = window.emit("install-done", ());
    Ok(())
}

pub fn launch_official_launcher() -> Result<(), String> {
    let exe = find_official_launcher_exe()
        .ok_or_else(|| "Could not find the official Minecraft Launcher installed.".to_string())?;
    Command::new(exe)
        .spawn()
        .map_err(|e| format!("failed to launch Minecraft: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebrand_profile_renames_icons_and_bumps_lastused() {
        let tmp = std::env::temp_dir().join(format!("fgl-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let profiles_path = tmp.join("launcher_profiles.json");

        // A trimmed but structurally real launcher_profiles.json, matching
        // what NeoForge's installer actually produces (verified against the
        // real file on a test machine).
        std::fs::write(
            &profiles_path,
            r#"{
              "profiles": {
                "forge": {
                  "name": "forge",
                  "type": "custom",
                  "lastUsed": "2020-01-01T00:00:00.000Z",
                  "lastVersionId": "1.20.1-forge-47.1.84",
                  "icon": "data:image/png;base64,AAAA"
                },
                "unrelated": {
                  "name": "some other profile",
                  "type": "custom",
                  "lastUsed": "2020-01-01T00:00:00.000Z",
                  "lastVersionId": "1.20.1"
                }
              },
              "settings": {},
              "version": 3
            }"#,
        )
        .unwrap();

        rebrand_profile(&tmp).unwrap();

        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&profiles_path).unwrap()).unwrap();
        let forge = &updated["profiles"]["forge"];
        assert_eq!(forge["name"], "Fighters Guild");
        assert_ne!(forge["lastUsed"], "2020-01-01T00:00:00.000Z", "lastUsed should be bumped to now");
        assert!(forge["lastUsed"].as_str().unwrap().ends_with('Z'), "lastUsed should stay in the same Zulu format Mojang uses");
        assert_ne!(forge["icon"], "data:image/png;base64,AAAA", "icon should be swapped to the embedded crossed-swords PNG");
        assert!(forge["icon"].as_str().unwrap().starts_with("data:image/png;base64,"));
        assert!(forge["javaArgs"].as_str().unwrap().contains("-Xmx8G"), "should set 8GB max heap, NeoForge's installer leaves this unset which falls back to the launcher's 2GB default and crashes");

        // The unrelated profile must be left completely untouched.
        assert_eq!(updated["profiles"]["unrelated"]["name"], "some other profile");
        assert_eq!(updated["profiles"]["unrelated"]["lastUsed"], "2020-01-01T00:00:00.000Z");

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn rebrand_profile_is_a_no_op_when_no_forge_profile_exists() {
        let tmp = std::env::temp_dir().join(format!("fgl-test-noforge-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let profiles_path = tmp.join("launcher_profiles.json");
        let original = r#"{"profiles":{"vanilla":{"name":"","lastVersionId":"1.20.1"}},"settings":{},"version":3}"#;
        std::fs::write(&profiles_path, original).unwrap();

        rebrand_profile(&tmp).unwrap();

        // Should return Ok and leave the file's meaning unchanged (no panic,
        // no forge-shaped profile invented out of nowhere).
        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&profiles_path).unwrap()).unwrap();
        assert_eq!(updated["profiles"]["vanilla"]["name"], "");
        assert!(updated["profiles"].get("forge").is_none());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
