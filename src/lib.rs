use std::fs;
use zed_extension_api::{
    self as zed, serde_json, settings::LspSettings, LanguageServerId, Result,
};

const ANNOTATIONS_REPO: &str = "astrochili/defold-annotations";
const ANNOTATIONS_DIR: &str = "defold_api";
const LIBS_API_DIR: &str = "libs_api";
const TARGET_LSP: &str = "lua-language-server";

struct DefoldKitExtension {
    cached_annotations_path: Option<String>,
    cached_lib_paths: Vec<String>,
}

#[derive(Default, Debug)]
struct DefoldKitConfig {
    editor_path: Option<String>,
    version: Option<String>,
}

impl DefoldKitConfig {
    fn from_settings(settings: Option<&serde_json::Value>) -> Self {
        let Some(block) = settings.and_then(|s| s.get("defold_kit")) else {
            return Self::default();
        };
        Self {
            editor_path: block
                .get("editor_path")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from),
            version: block
                .get("version")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
                .map(String::from),
        }
    }
}

fn is_defold_project(worktree: &zed::Worktree) -> bool {
    worktree.read_text_file("game.project").is_ok()
}

fn defold_config_path(editor_path: &str) -> String {
    let trimmed = editor_path.trim_end_matches('/');
    if trimmed.ends_with(".app") {
        format!("{trimmed}/Contents/Resources/config")
    } else {
        format!("{trimmed}/config")
    }
}

fn detect_version_from_editor(editor_path: &str) -> Option<String> {
    let config_path = defold_config_path(editor_path);
    let output = zed::process::Command::new("cat")
        .arg(&config_path)
        .output()
        .ok()?;
    if !matches!(output.status, Some(0)) {
        return None;
    }
    let content = String::from_utf8(output.stdout).ok()?;
    parse_version_from_ini(&content)
}

fn parse_version_from_ini(content: &str) -> Option<String> {
    let mut in_build = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_build = trimmed == "[build]";
            continue;
        }
        if !in_build {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            if key.trim() == "version" {
                let v = value.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn parse_include_dirs(content: &str) -> Vec<String> {
    let mut in_library = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_library = trimmed == "[library]";
            continue;
        }
        if !in_library {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            if key.trim() == "include_dirs" {
                return value
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    Vec::new()
}

fn list_zip_files(dir: &str) -> Vec<String> {
    let output = zed::process::Command::new("find")
        .args([dir, "-maxdepth", "1", "-name", "*.zip", "-type", "f"])
        .output()
        .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let Ok(content) = String::from_utf8(output.stdout) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|p| p.rsplit('/').next().map(String::from))
        .collect()
}

fn zip_root_folder(archive_path: &str) -> Option<String> {
    let output = zed::process::Command::new("unzip")
        .args(["-Z1", archive_path])
        .output()
        .ok()?;
    if !matches!(output.status, Some(0)) {
        return None;
    }
    let content = String::from_utf8(output.stdout).ok()?;
    let first = content.lines().next()?;
    let root = first.split('/').next()?;
    if root.is_empty() {
        None
    } else {
        Some(root.to_string())
    }
}

fn extract_zip(archive_path: &str, dest_dir_abs: &str) -> bool {
    zed::process::Command::new("unzip")
        .args(["-o", "-q", archive_path, "-d", dest_dir_abs])
        .output()
        .is_ok_and(|o| matches!(o.status, Some(0)))
}

fn merge_library_path(target: &mut serde_json::Value, path: String) {
    if !target.is_object() {
        *target = serde_json::json!({});
    }
    let lua = target
        .as_object_mut()
        .unwrap()
        .entry("Lua".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !lua.is_object() {
        *lua = serde_json::json!({});
    }
    let workspace = lua
        .as_object_mut()
        .unwrap()
        .entry("workspace".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !workspace.is_object() {
        *workspace = serde_json::json!({});
    }
    let library = workspace
        .as_object_mut()
        .unwrap()
        .entry("library".to_string())
        .or_insert_with(|| serde_json::json!([]));
    if !library.is_array() {
        *library = serde_json::json!([]);
    }
    let arr = library.as_array_mut().unwrap();
    let already_present = arr
        .iter()
        .filter_map(serde_json::Value::as_str)
        .any(|p| p == path);
    if !already_present {
        arr.push(serde_json::Value::String(path));
    }
}

impl DefoldKitExtension {
    fn resolve_annotations_version(&self, config: &DefoldKitConfig) -> Result<String> {
        if let Some(v) = &config.version {
            return Ok(v.clone());
        }
        if let Some(p) = &config.editor_path {
            if let Some(v) = detect_version_from_editor(p) {
                return Ok(v);
            }
        }
        let release = zed::latest_github_release(
            ANNOTATIONS_REPO,
            zed::GithubReleaseOptions {
                require_assets: true,
                pre_release: false,
            },
        )?;
        Ok(release.version)
    }

    fn ensure_annotations(&mut self, config: &DefoldKitConfig) -> Result<String> {
        let version = self.resolve_annotations_version(config)?;
        let target_dir = format!("{ANNOTATIONS_DIR}/{version}");

        if !fs::metadata(&target_dir).is_ok_and(|m| m.is_dir()) {
            fs::create_dir_all(ANNOTATIONS_DIR)
                .map_err(|e| format!("failed to create annotations dir: {e}"))?;

            let url = format!(
                "https://github.com/{ANNOTATIONS_REPO}/releases/download/{version}/defold_api_{version}.zip"
            );

            zed::download_file(&url, &target_dir, zed::DownloadedFileType::Zip)
                .map_err(|e| format!("failed to download annotations {version}: {e}"))?;

            self.prune_old_annotations(&version);
        }

        let cwd = std::env::current_dir()
            .map_err(|e| format!("failed to get working directory: {e}"))?;
        let abs = cwd.join(&target_dir);
        Ok(abs.to_string_lossy().into_owned())
    }

    fn prune_old_annotations(&self, current_version: &str) {
        let Ok(entries) = fs::read_dir(ANNOTATIONS_DIR) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name_str) = name.to_str() else { continue };
            if name_str != current_version {
                fs::remove_dir_all(entry.path()).ok();
            }
        }
    }

    fn sync_lib_annotations(&self, worktree: &zed::Worktree) -> Vec<String> {
        let lib_dir = format!("{}/.internal/lib", worktree.root_path());
        let archives = list_zip_files(&lib_dir);
        if archives.is_empty() {
            let _ = fs::remove_dir_all(LIBS_API_DIR);
            return Vec::new();
        }

        if fs::create_dir_all(LIBS_API_DIR).is_err() {
            return Vec::new();
        }

        let Ok(cwd) = std::env::current_dir() else {
            return Vec::new();
        };
        let abs_libs_api = cwd.join(LIBS_API_DIR);
        let abs_libs_api_str = abs_libs_api.to_string_lossy().into_owned();

        let mut library_paths = Vec::new();
        let mut keep_roots = std::collections::HashSet::new();

        for archive in &archives {
            let archive_path = format!("{lib_dir}/{archive}");

            let Some(root) = zip_root_folder(&archive_path) else {
                continue;
            };
            keep_roots.insert(root.clone());

            let extract_marker = format!("{LIBS_API_DIR}/{root}");
            if !fs::metadata(&extract_marker).is_ok_and(|m| m.is_dir())
                && !extract_zip(&archive_path, &abs_libs_api_str)
            {
                continue;
            }

            let inner_project = format!("{extract_marker}/game.project");
            let Ok(content) = fs::read_to_string(&inner_project) else {
                continue;
            };

            for dir in parse_include_dirs(&content) {
                let abs = cwd.join(&extract_marker).join(&dir);
                library_paths.push(abs.to_string_lossy().into_owned());
            }
        }

        if let Ok(entries) = fs::read_dir(LIBS_API_DIR) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let Some(name_str) = name.to_str() else {
                    continue;
                };
                if !keep_roots.contains(name_str) {
                    fs::remove_dir_all(entry.path()).ok();
                }
            }
        }

        library_paths
    }
}

impl zed::Extension for DefoldKitExtension {
    fn new() -> Self {
        Self {
            cached_annotations_path: None,
            cached_lib_paths: Vec::new(),
        }
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &LanguageServerId,
        _worktree: &zed::Worktree,
    ) -> Result<zed::Command> {
        Err("defold-lua-ls is config-only and does not run a language server".into())
    }

    fn language_server_additional_workspace_configuration(
        &mut self,
        _language_server_id: &LanguageServerId,
        target_language_server_id: &LanguageServerId,
        worktree: &zed::Worktree,
    ) -> Result<Option<serde_json::Value>> {
        if target_language_server_id.as_ref() != TARGET_LSP {
            return Ok(None);
        }
        if !is_defold_project(worktree) {
            return Ok(None);
        }

        let user_settings = LspSettings::for_worktree(target_language_server_id.as_ref(), worktree)
            .ok()
            .and_then(|s| s.settings);
        let kit_cfg = DefoldKitConfig::from_settings(user_settings.as_ref());

        if self.cached_annotations_path.is_none() {
            self.cached_annotations_path = self.ensure_annotations(&kit_cfg).ok();
        }
        self.cached_lib_paths = self.sync_lib_annotations(worktree);

        let mut output = serde_json::json!({});
        if let Some(annotations_path) = self.cached_annotations_path.clone() {
            merge_library_path(&mut output, annotations_path);
        }
        for lib_path in &self.cached_lib_paths {
            merge_library_path(&mut output, lib_path.clone());
        }

        Ok(Some(output))
    }
}

zed::register_extension!(DefoldKitExtension);
