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
}
