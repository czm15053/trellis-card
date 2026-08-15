// 应用配置：初始扫描根目录 + Hook 发现的动态项目 + 窗口置顶偏好
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub roots: Vec<String>,
    pub dynamic_projects: Vec<String>,
    pub initialized: bool,
    pub always_on_top: bool,
    /* WSL 观察模式：设为发行版名（如 "Ubuntu"）后，hook 安装写入 WSL 侧配置、
    hook 事件里的 Linux 项目路径映射为 \\wsl$\<distro>\... 。None = 未启用。 */
    pub wsl_distro: Option<String>,
}

pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("trellis-card")
}

pub fn config_path() -> PathBuf {
    app_config_dir().join("config.json")
}

pub fn inbox_dir() -> PathBuf {
    app_config_dir().join("inbox")
}

/* Unix socket paths have a small platform-defined limit on macOS. Keep the
socket in /tmp and derive a stable per-config name so long HOME paths do not
break hook delivery.
Windows 没有 Unix socket，用 Named Pipe（\\?\pipe\...），名字固定、不依赖
用户主目录长度，与 ipc.rs 的 NamedPipeListener 对应。 */
pub fn socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in app_config_dir().to_string_lossy().as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        PathBuf::from(format!("/tmp/trellis-card-{hash:016x}.sock"))
    }
    #[cfg(windows)]
    {
        PathBuf::from(r"\\.\pipe\trellis-card-hook")
    }
    #[cfg(all(not(unix), not(windows)))]
    {
        app_config_dir().join("events.sock")
    }
}

pub fn load() -> AppConfig {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &AppConfig) -> Result<(), String> {
    let dir = app_config_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let data = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(config_path(), data).map_err(|e| e.to_string())
}

pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        if let Some(home) = dirs::home_dir() {
            // 兼容 Windows 的 ~\proj 与 macOS 的 ~/proj；其余分隔符交给 join
            let rest = rest
                .trim_start_matches('/')
                .trim_start_matches('\\')
                .replace('\\', "/");
            return home.join(rest);
        }
    }
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_home_handles_tilde_and_plain() {
        let plain = expand_home("/tmp/x");
        assert_eq!(plain, PathBuf::from("/tmp/x"));
        let home = expand_home("~/code");
        assert!(home.ends_with("code"));
        assert!(!home.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn config_roundtrip() {
        let cfg = AppConfig {
            roots: vec!["/a".into(), "~/b".into()],
            dynamic_projects: vec![],
            initialized: true,
            always_on_top: true,
            wsl_distro: Some("Ubuntu".into()),
        };
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("\"alwaysOnTop\":true"));
        let back: AppConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.roots.len(), 2);
        assert!(back.initialized);
        assert!(back.always_on_top);
        assert_eq!(back.wsl_distro.as_deref(), Some("Ubuntu"));
    }

    #[test]
    fn config_tolerates_missing_fields() {
        let back: AppConfig = serde_json::from_str("{}").unwrap();
        assert!(back.roots.is_empty());
        assert!(back.dynamic_projects.is_empty());
        assert!(!back.initialized);
        assert!(!back.always_on_top);
    }

    #[cfg(unix)]
    #[test]
    fn socket_path_is_short_and_stable() {
        let first = socket_path();
        assert_eq!(first, socket_path());
        assert!(first.to_string_lossy().len() < 100);
    }
}
