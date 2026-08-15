// 跨平台辅助：路径归一化、大小写不敏感比较、Python 命令探测。
// 工具函数只服务于字符串匹配/比较，不改任何文件系统调用。

/// 反斜杠统一为正斜杠，仅用于持久化键与字符串比较。
pub fn to_posix(p: &str) -> String {
    p.replace('\\', "/")
}

/// 剥掉尾部 `/` 或 `\` 后做大小写不敏感比较（Windows 文件系统不区分大小写）。
pub fn path_eq_ignore_case(a: &str, b: &str) -> bool {
    let a = to_posix(a);
    let b = to_posix(b);
    a.trim_end_matches('/')
        .eq_ignore_ascii_case(b.trim_end_matches('/'))
}

/// 去掉 Windows 设备路径前缀 `\\?\`（canonicalize 返回）；非 Windows 原样返回。
pub fn strip_device_prefix(p: &str) -> String {
    #[cfg(windows)]
    {
        if let Some(rest) = p.strip_prefix(r"\\?\") {
            return rest.to_string();
        }
    }
    p.to_string()
}

/// canonicalize 并去掉 Windows 设备路径前缀。
pub fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let p = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    std::path::PathBuf::from(strip_device_prefix(&p.to_string_lossy()))
}

/// Python 解释器候选命令：win32 试 python/python3/py -3，其他试 python3/python。
pub fn python_candidates() -> Vec<&'static str> {
    if cfg!(windows) {
        vec!["python", "python3", "py -3"]
    } else {
        vec!["python3", "python"]
    }
}

/* ---- WSL 观察模式 ----
Windows 原生跑 Card、观察 WSL 内 Trellis 项目时，项目 canonical 路径统一用
Windows UNC 形式 `\\wsl$\<distro>\home\<user>\...`（Windows API 原生可访问），
hook 事件里上报的 Linux 路径（/home/alice/proj）在事件入口转成 UNC。
本模块只做纯字符串映射，不做文件系统调用，可单测。 */

/// 当前生效的 WSL 发行版名：env `TRELLIS_CARD_WSL_DISTRO` 优先（hook 子进程注入），
/// 其次应用配置里的 wsl_distro（GUI 设置）。None 表示未启用 WSL 观察。
pub fn wsl_distro() -> Option<String> {
    if let Ok(d) = std::env::var("TRELLIS_CARD_WSL_DISTRO") {
        let d = d.trim().to_string();
        if !d.is_empty() {
            return Some(d);
        }
    }
    crate::config::load()
        .wsl_distro
        .clone()
        .filter(|d| !d.is_empty())
}

/// Windows UNC `\\wsl$\<distro>\home\alice\proj` → WSL Linux 路径 `/home/alice/proj`。
/// 非 WSL UNC 输入返回 None。
pub fn linux_from_wsl_unc(unc: &str) -> Option<String> {
    let norm = to_posix(unc);
    let rest = norm.strip_prefix("//wsl$/")?;
    let (distro, tail) = rest.split_once('/').unwrap_or((rest, ""));
    if distro.is_empty() {
        return None;
    }
    if tail.is_empty() {
        return Some("/".into());
    }
    Some(format!("/{tail}"))
}

/// 从 WSL UNC 路径中提取发行版名：`\\wsl$\Ubuntu\home\alice` → `Ubuntu`。
/// 供未来校验「事件 UNC 路径的 distro 与当前配置是否一致」等场景；目前无生产调用点。
#[allow(dead_code)]
pub fn wsl_distro_from_unc(unc: &str) -> Option<String> {
    let norm = to_posix(unc);
    let rest = norm.strip_prefix("//wsl$/")?;
    let distro = rest.split('/').next().unwrap_or_default();
    (!distro.is_empty()).then(|| distro.to_string())
}

/// WSL Linux 路径 `/home/alice/proj` + 发行版 `Ubuntu` → `\\wsl$\Ubuntu\home\alice\proj`。
/// 非绝对 Linux 路径（不以 `/` 开头）或空 distro 返回 None。
pub fn wsl_unc_from_linux(linux: &str, distro: &str) -> Option<String> {
    let linux = to_posix(linux);
    if !linux.starts_with('/') || distro.trim().is_empty() {
        return None;
    }
    let mut out = format!(r"\\wsl$\{}", distro.trim());
    for part in linux.trim_start_matches('/').split('/') {
        if !part.is_empty() {
            out.push('\\');
            out.push_str(part);
        }
    }
    Some(out)
}

/// 判断路径是否为 WSL UNC（`\\wsl$\...`，大小写不敏感）。
pub fn is_wsl_unc(p: &str) -> bool {
    let norm = to_posix(p).to_ascii_lowercase();
    norm.starts_with("//wsl$")
}

/// Windows 盘符路径 `C:\Program Files\...` → WSL 挂载路径 `/mnt/c/Program Files/...`。
/// 供 WSL 内 hook 命令引用 Windows 侧 trellis-card.exe。非盘符路径返回 None。
pub fn windows_to_wsl_path(win: &str) -> Option<String> {
    let norm = to_posix(win);
    let bytes = norm.as_bytes();
    if bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'/' {
        let drive = (bytes[0] as char).to_ascii_lowercase();
        let rest = &norm[3..];
        let rest = rest.trim_matches('/');
        Some(format!(
            "/mnt/{}{}",
            drive,
            if rest.is_empty() {
                "/".to_string()
            } else {
                format!("/{rest}")
            }
        ))
    } else {
        None
    }
}

/// WSL 挂载盘路径 `/mnt/c/Users/alice` → Windows 盘符路径 `C:\Users\alice`。
/// 对齐上游 Trellis `_normalize_windows_shell_path` 对 `/mnt/<drive>/...` 的处理：
/// 这类路径表示 Windows 盘在 WSL 里的挂载，Windows 侧直接访问盘符即可，
/// 不必绕 UNC（`\\wsl$\<distro>\mnt\c\...` 是两层跳转、非规范）。非 `/mnt/<drive>/` 返回 None。
pub fn wsl_mount_to_windows(p: &str) -> Option<String> {
    let norm = to_posix(p);
    let rest = norm.strip_prefix("/mnt/")?;
    let (drive, tail) = rest.split_once('/')?;
    if drive.len() != 1 || !drive.chars().next()?.is_ascii_alphabetic() {
        return None;
    }
    let drive = drive.to_ascii_uppercase();
    Some(format!("{drive}:\\{}", tail.replace('/', "\\")))
}

/// 纯函数：给定发行版，Linux 绝对路径转 WSL UNC；已是 UNC / 相对路径 /
/// 非绝对 Linux / distro 为空 时原样返回。不依赖进程 env，可安全单测。
/// `/mnt/c/...`（WSL 挂载的 Windows 盘）优先转 Windows 盘符路径而非 UNC。
pub fn maybe_to_wsl_unc_with(p: &str, distro: Option<&str>) -> String {
    if let Some(distro) = distro {
        if !distro.trim().is_empty() && !is_wsl_unc(p) && p.starts_with('/') {
            /* /mnt/<drive>/... → C:\...（Windows 盘挂载到 WSL，直接走盘符） */
            if let Some(win) = wsl_mount_to_windows(p) {
                return win;
            }
            if let Some(unc) = wsl_unc_from_linux(p, distro) {
                return unc;
            }
        }
    }
    p.to_string()
}
/// 当前 WSL 观察模式下的转换入口：读 env / 配置的发行版名后委托纯函数。
pub fn maybe_to_wsl_unc(p: &str) -> String {
    maybe_to_wsl_unc_with(p, wsl_distro().as_deref())
}

/// 解码 `wsl.exe` 等 Windows 命令行工具的输出。
/// `wsl.exe` 的输出编码因版本而异：可能是 UTF-16LE（带 `\xff\xfe` BOM）、
/// UTF-8（带 `\xef\xbb\xbf` BOM）或纯 UTF-8。统一做自适应解码：
/// 优先按 BOM 判断，无 BOM 时检测 NUL 字节判断是否为 UTF-16LE。
/// 仅在 Windows 生产路径调用（cfg(windows)），非 Windows 仅测试使用。
#[cfg_attr(not(windows), allow(dead_code))]
pub fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xff, 0xfe]) {
        // UTF-16LE + BOM：按小端 u16 解码
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xef, 0xbb, 0xbf]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.iter().skip(1).step_by(2).any(|&b| b == 0) {
        // 无 BOM 但疑似 UTF-16LE：ASCII 字符的高字节（奇数位置）是 NUL
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// 枚举可用的 WSL 发行版名。非 Windows 平台或枚举失败返回空。
/// 测试可用 env `TRELLIS_CARD_WSL_DISTROS`（逗号分隔）注入固定结果。
pub fn detect_wsl_distros() -> Vec<String> {
    if let Ok(ds) = std::env::var("TRELLIS_CARD_WSL_DISTROS") {
        return ds
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
    }
    #[cfg(windows)]
    {
        let mut out = Vec::new();
        if let Ok(output) = std::process::Command::new("wsl.exe")
            .args(["-l", "-q"])
            .output()
        {
            if output.status.success() {
                for line in decode_wsl_output(&output.stdout).lines() {
                    let d = line.trim();
                    if !d.is_empty() && !d.to_ascii_lowercase().starts_with("windows") {
                        out.push(d.to_string());
                    }
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_posix_converts_backslashes() {
        assert_eq!(to_posix(r"C:\Users\Alice\proj"), "C:/Users/Alice/proj");
        assert_eq!(
            to_posix("/repo/.trellis/tasks/01-a"),
            "/repo/.trellis/tasks/01-a"
        );
        assert_eq!(to_posix(r"\.trellis\tasks\01-a"), "/.trellis/tasks/01-a");
    }

    #[test]
    fn path_eq_ignore_case_handles_case_and_separators() {
        assert!(path_eq_ignore_case(
            r"C:\Users\Alice\proj",
            "c:/users/alice/proj"
        ));
        assert!(path_eq_ignore_case("/repo/alpha", "/REPO/alpha"));
        assert!(path_eq_ignore_case(r"C:\proj\", "c:/proj"));
        assert!(!path_eq_ignore_case("/repo/alpha", "/repo/beta"));
    }

    #[test]
    fn python_candidates_cover_platforms() {
        let cands = python_candidates();
        assert!(!cands.is_empty());
        if cfg!(windows) {
            assert_eq!(cands[0], "python");
        } else {
            assert_eq!(cands[0], "python3");
        }
    }

    /* ---- WSL 路径映射（纯函数） ---- */

    #[test]
    fn wsl_unc_round_trip_linux_path() {
        let unc = wsl_unc_from_linux("/home/alice/proj", "Ubuntu").unwrap();
        assert_eq!(unc, r"\\wsl$\Ubuntu\home\alice\proj");
        assert_eq!(
            linux_from_wsl_unc(&unc).as_deref(),
            Some("/home/alice/proj")
        );
    }

    #[test]
    fn wsl_unc_rejects_relative_and_empty_distro() {
        assert!(wsl_unc_from_linux("home/alice", "Ubuntu").is_none());
        assert!(wsl_unc_from_linux("/home/alice", " ").is_none());
        assert!(wsl_unc_from_linux("/home/alice", "").is_none());
    }

    #[test]
    fn linux_from_wsl_unc_handles_distro_and_tail() {
        assert_eq!(
            linux_from_wsl_unc(r"\\wsl$\Debian\home\bob").as_deref(),
            Some("/home/bob")
        );
        assert_eq!(linux_from_wsl_unc(r"\\wsl$\Debian").as_deref(), Some("/"));
        assert!(linux_from_wsl_unc(r"C:\Users\alice").is_none());
        assert!(linux_from_wsl_unc("relative").is_none());
    }

    #[test]
    fn wsl_distro_from_unc_extracts_distro() {
        assert_eq!(
            wsl_distro_from_unc(r"\\wsl$\Ubuntu\home\alice").as_deref(),
            Some("Ubuntu")
        );
        assert!(wsl_distro_from_unc(r"C:\Users\alice").is_none());
    }

    #[test]
    fn is_wsl_unc_matches_prefix_case_insensitive() {
        assert!(is_wsl_unc(r"\\wsl$\Ubuntu\home\alice"));
        assert!(is_wsl_unc(r"\\WSL$\Ubuntu\x"));
        assert!(is_wsl_unc(r"\\wsl$\"));
        assert!(!is_wsl_unc(r"\\server\share\path"));
        assert!(!is_wsl_unc(r"C:\Users\alice"));
        assert!(!is_wsl_unc("/home/alice"));
    }

    #[test]
    fn windows_to_wsl_path_converts_drive_letters() {
        assert_eq!(
            windows_to_wsl_path(r"C:\Program Files\Trellis-Card\trellis-card.exe").as_deref(),
            Some("/mnt/c/Program Files/Trellis-Card/trellis-card.exe")
        );
        assert_eq!(
            windows_to_wsl_path(r"D:\repo\proj").as_deref(),
            Some("/mnt/d/repo/proj")
        );
        assert_eq!(windows_to_wsl_path(r"C:\").as_deref(), Some("/mnt/c/"));
        assert!(windows_to_wsl_path(r"\\wsl$\Ubuntu\x").is_none());
        assert!(windows_to_wsl_path("/mnt/c/x").is_none());
    }

    #[test]
    fn wsl_mount_to_windows_converts_mnt_drive_paths() {
        assert_eq!(
            wsl_mount_to_windows("/mnt/c/Users/alice/proj").as_deref(),
            Some(r"C:\Users\alice\proj")
        );
        assert_eq!(
            wsl_mount_to_windows("/mnt/d/repo").as_deref(),
            Some(r"D:\repo")
        );
        /* 非 /mnt/<drive>/ 模式不转换 */
        assert!(wsl_mount_to_windows("/home/alice").is_none());
        assert!(wsl_mount_to_windows("/mnt/disk2/x").is_none()); // 多字符不是盘符
        assert!(wsl_mount_to_windows(r"C:\x").is_none());
        assert_eq!(wsl_mount_to_windows("/mnt/c/x").as_deref(), Some(r"C:\x"));
        /* 与 windows_to_wsl_path 互为逆 */
        let win = r"C:\Users\alice\proj";
        let mount = windows_to_wsl_path(win).unwrap();
        assert_eq!(wsl_mount_to_windows(&mount).as_deref(), Some(win));
    }

    #[test]
    fn maybe_to_wsl_unc_with_is_pure_and_distro_driven() {
        /* 纯函数：传入发行版 → Linux 绝对路径被转换 */
        assert_eq!(
            maybe_to_wsl_unc_with("/home/alice/proj", Some("Ubuntu")),
            r"\\wsl$\Ubuntu\home\alice\proj"
        );
        /* /mnt/c/...（WSL 挂载的 Windows 盘）→ Windows 盘符，不走 UNC */
        assert_eq!(
            maybe_to_wsl_unc_with("/mnt/c/Users/alice/proj", Some("Ubuntu")),
            r"C:\Users\alice\proj"
        );
        /* 已是 UNC / 相对路径 / Windows 盘符路径不转换 */
        assert_eq!(
            maybe_to_wsl_unc_with(r"\\wsl$\Ubuntu\home\alice", Some("Ubuntu")),
            r"\\wsl$\Ubuntu\home\alice"
        );
        assert_eq!(
            maybe_to_wsl_unc_with("relative/path", Some("Ubuntu")),
            "relative/path"
        );
        assert_eq!(
            maybe_to_wsl_unc_with(r"C:\Users\alice", Some("Ubuntu")),
            r"C:\Users\alice"
        );
        /* distro 为空 → 不转换 */
        assert_eq!(
            maybe_to_wsl_unc_with("/home/alice/proj", None),
            "/home/alice/proj"
        );
        assert_eq!(
            maybe_to_wsl_unc_with("/home/alice/proj", Some("  ")),
            "/home/alice/proj"
        );
    }

    #[test]
    fn decode_wsl_output_handles_utf16_bom() {
        /* UTF-16LE + BOM：\xff\xfe + "Ubuntu" 每个字符后跟 \x00 */
        let mut bytes = vec![0xff, 0xfe];
        for unit in "Ubuntu".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_wsl_output(&bytes), "Ubuntu");
    }

    #[test]
    fn decode_wsl_output_handles_utf16_without_bom() {
        /* 无 BOM 的 UTF-16LE：偶数位置 NUL 触发 UTF-16 解码 */
        let mut bytes = Vec::new();
        for unit in "Debian".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_wsl_output(&bytes), "Debian");
    }

    #[test]
    fn decode_wsl_output_handles_utf8_bom_and_plain() {
        /* UTF-8 + BOM */
        let mut bom = vec![0xef, 0xbb, 0xbf];
        bom.extend_from_slice(b"Ubuntu");
        assert_eq!(decode_wsl_output(&bom), "Ubuntu");
        /* 纯 UTF-8（新版 WSL） */
        assert_eq!(decode_wsl_output(b"Ubuntu"), "Ubuntu");
    }
}
