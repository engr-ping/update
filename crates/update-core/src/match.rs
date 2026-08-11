// 标签解析与平台/产物匹配（对应原 Go internal/version/match.go）。
use std::collections::HashMap;

/// 剥离常见前缀（v、V、release-），返回用于比较与展示的裸版本号。
pub fn clean_tag(tag: &str) -> String {
    let t = tag.strip_prefix('v').unwrap_or(tag);
    let t = t.strip_prefix('V').unwrap_or(t);
    let t = t.strip_prefix("release-").unwrap_or(t);
    t.to_string()
}

/// 报告产物是否应被选中用于给定平台。os_name 或 arch 为空、或等于 "all"
/// 时全部匹配；否则按常见 os-arch 命名约定匹配（"linux-amd64"、
/// "linux_amd64"、"windows-x86_64" ...）。无平台标识的产物视为平台中立、匹配一切。
pub fn match_asset(name: &str, os_name: &str, arch: &str) -> bool {
    if os_name.is_empty() || arch.is_empty() || os_name == "all" {
        return true;
    }
    let lower = name.to_lowercase();
    let o = os_name.to_lowercase();
    let a = arch.to_lowercase();

    // 常见平台别名
    let os_aliases: HashMap<&str, Vec<&str>> = HashMap::from([
        ("macos", vec!["darwin", "osx"]),
        ("osx", vec!["darwin", "macos"]),
    ]);
    let arch_aliases: HashMap<&str, Vec<&str>> = HashMap::from([
        ("amd64", vec!["x86_64", "x64"]),
        ("arm64", vec!["aarch64"]),
        ("386", vec!["i386", "i686", "x86"]),
    ]);

    let mut os_names = vec![o.as_str()];
    if let Some(extra) = os_aliases.get(o.as_str()) {
        os_names.extend(extra.iter().copied());
    }
    let mut arch_names = vec![a.as_str()];
    if let Some(extra) = arch_aliases.get(a.as_str()) {
        arch_names.extend(extra.iter().copied());
    }

    for sep in ["-", "_", "."] {
        for on in &os_names {
            for an in &arch_names {
                if lower.contains(&format!("{on}{sep}{an}"))
                    || lower.contains(&format!("{an}{sep}{on}"))
                {
                    return true;
                }
            }
        }
    }
    for on in &os_names {
        for an in &arch_names {
            if lower.contains(on) && lower.contains(an) {
                return true;
            }
        }
    }
    // 中立产物：仅当它不含任何已知平台标识时匹配
    !has_platform_marker(&lower)
}

const PLATFORM_MARKERS: &[&str] = &[
    "linux", "windows", "darwin", "macos", "freebsd", //
    "amd64", "x86_64", "x64", "arm64", "aarch64", "386", "i386", "armv7", "arm",
];

fn has_platform_marker(s: &str) -> bool {
    PLATFORM_MARKERS.iter().any(|m| s.contains(m))
}

/// 返回宿主平台（Go 语义）：os 为 "darwin"/"linux"/"windows"，
/// arch 为 "amd64"/"arm64"/"386"/"arm" 等 GOARCH 风格值。
pub fn host_platform() -> (String, String) {
    let os = match std::env::consts::OS {
        "macos" => "darwin".to_string(),
        other => other.to_string(),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "amd64".to_string(),
        "x86" => "386".to_string(),
        "aarch64" => "arm64".to_string(),
        other => other.to_string(),
    };
    (os, arch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_asset() {
        // 与 Go 版 internal/version/match_test.go 完全一致
        let cases: &[(&str, &str, &str, bool)] = &[
            ("app-linux-amd64.tar.gz", "linux", "amd64", true),
            ("app-linux-arm64.tar.gz", "linux", "arm64", true),
            ("app-windows-amd64.zip", "windows", "amd64", true),
            ("app-linux_amd64.deb", "linux", "amd64", true),
            ("app.linux.amd64.rpm", "linux", "amd64", true),
            ("app-amd64-linux.tar.gz", "linux", "amd64", true),
            ("app-darwin-x86_64.zip", "darwin", "amd64", true),
            ("app-linux-amd64.tar.gz", "windows", "amd64", false),
            ("app-windows-amd64.zip", "linux", "amd64", false),
            // 无 os 标识、已含 arch 标识 → 非中立 → 不匹配
            ("app-x86_64.exe", "windows", "amd64", false),
            ("README.md", "linux", "amd64", true),     // 中立
            ("LICENSE", "windows", "arm64", true),     // 中立
            ("checksums.txt", "linux", "amd64", true), // 中立
            ("", "linux", "amd64", true),              // 空视为中立
            ("app-linux-amd64", "", "", true),         // 无平台指定 → 全部
        ];
        for (name, os, arch, want) in cases {
            let got = match_asset(name, os, arch);
            assert_eq!(
                got, *want,
                "match_asset({name:?}, {os:?}, {arch:?}) = {got}, want {want}"
            );
        }
    }
}
