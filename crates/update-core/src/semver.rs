// semver 2.0 比较，零第三方依赖的手写实现（对应原 Go internal/version/semver.go）。
use std::cmp::Ordering;

/// 解析后的语义化版本组件。
#[derive(Debug, Clone)]
pub struct Semver {
    core: [i64; 3],   // major, minor, patch
    pre: Vec<String>, // 预发布标识（不含 '-'）
    has_pre: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    NotSemver,
}

/// 将版本字符串解析为 Semver。
///
/// 规则：
///   - 可选 "v"/"V" 前缀忽略
///   - '+' 之后的构建元数据忽略
///   - 两位版本号 "1.2" 接受（patch 视为 0）
fn parse(s: &str) -> Option<Semver> {
    let s = s.trim();
    let s = s
        .strip_prefix('v')
        .or_else(|| s.strip_prefix('V'))
        .unwrap_or(s);
    if s.is_empty() {
        return None;
    }
    // 去掉构建元数据
    let s = match s.find('+') {
        Some(i) => &s[..i],
        None => s,
    };
    let (s, pre) = match s.find('-') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    if s.is_empty() {
        return None;
    }

    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut core = [0i64; 3];
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            return None;
        }
        core[i] = p.parse::<i64>().ok()?;
    }
    let mut sv = Semver {
        core,
        pre: Vec::new(),
        has_pre: false,
    };
    if let Some(pre) = pre {
        let ids: Vec<&str> = pre.split('.').collect();
        for id in &ids {
            if id.is_empty() {
                return None;
            }
        }
        sv.pre = ids.iter().map(|s| s.to_string()).collect();
        sv.has_pre = true;
    }
    Some(sv)
}

/// 比较两个版本字符串，返回 -1/0/+1（a 小于/等于/大于 b）。
///
/// 规则：
///   - 可选 "v"/"V" 前缀忽略
///   - '+' 之后构建元数据不参与比较
///   - 无预发布的版本排在有预发布之后（1.0.0 > 1.0.0-rc1）
///   - 预发布标识：数字按数值比较，字母数字按字典序，数字排在字母数字前
///   - 非 semver 字符串回退为不区分大小写的字典序比较
///   - 非 semver 排在 semver 之下
pub fn compare(a: &str, b: &str) -> i32 {
    let (sa, sb) = (parse(a), parse(b));
    match (sa, sb) {
        (Some(sa), Some(sb)) => compare_semver(&sa, &sb),
        (None, None) => {
            let c = a.to_lowercase().cmp(&b.to_lowercase());
            match c {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            }
        }
        (None, Some(_)) => -1, // 非 semver 排在 semver 之下
        (Some(_), None) => 1,
    }
}

fn compare_semver(a: &Semver, b: &Semver) -> i32 {
    for i in 0..3 {
        match a.core[i].cmp(&b.core[i]) {
            Ordering::Less => return -1,
            Ordering::Greater => return 1,
            Ordering::Equal => {}
        }
    }
    match (a.has_pre, b.has_pre) {
        (false, false) => return 0,
        (true, false) => return -1,
        (false, true) => return 1,
        _ => {}
    }
    // 双方都有预发布标识
    let n = a.pre.len().min(b.pre.len());
    for i in 0..n {
        let c = compare_pre_id(&a.pre[i], &b.pre[i]);
        if c != 0 {
            return c;
        }
    }
    match a.pre.len().cmp(&b.pre.len()) {
        Ordering::Less => -1,
        Ordering::Greater => 1,
        Ordering::Equal => 0,
    }
}

/// 按 semver 2.0 比较两个预发布标识。
fn compare_pre_id(a: &str, b: &str) -> i32 {
    let ai = a.parse::<i64>();
    let bi = b.parse::<i64>();
    match (ai, bi) {
        (Ok(ai), Ok(bi)) => match ai.cmp(&bi) {
            Ordering::Less => -1,
            Ordering::Greater => 1,
            Ordering::Equal => 0,
        },
        (Ok(_), Err(_)) => -1, // 数字排在字母数字之前
        (Err(_), Ok(_)) => 1,
        (Err(_), Err(_)) => match a.cmp(b) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        },
    }
}

/// 报告 s 能否按语义化版本解析。
pub fn is_semver(s: &str) -> bool {
    parse(s).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compare() {
        // 与 Go 版 internal/version/semver_test.go 完全一致
        let cases: &[(&str, &str, i32)] = &[
            ("1.0.0", "1.0.0", 0),
            ("v1.0.0", "1.0.0", 0),
            ("1.0.0", "1.0.1", -1),
            ("1.0.1", "1.0.0", 1),
            ("1.2.0", "1.10.0", -1),
            ("1.10.0", "1.2.0", 1),
            ("2.0.0", "1.99.99", 1),
            ("1.0.0-alpha", "1.0.0", -1),
            ("1.0.0", "1.0.0-alpha", 1),
            ("1.0.0-alpha", "1.0.0-alpha", 0),
            ("1.0.0-alpha.1", "1.0.0-alpha.2", -1),
            ("1.0.0-rc.1", "1.0.0-beta.1", 1),
            ("1.0.0-1", "1.0.0-alpha", -1), // 数字 < 字母数字
            ("1.0.0+build.1", "1.0.0", 0),  // 构建元数据忽略
            ("v1.2.3", "1.2.4", -1),
            ("release-1.2.0", "1.2.0", -1), // Compare 不去 release- 前缀
            ("abc", "def", -1),             // 非 semver 字典序
            ("1.2", "1.2.0", 0),            // 两位版本号接受
            ("", "1.0.0", -1),
        ];
        for (a, b, want) in cases {
            let got = compare(a, b);
            assert_eq!(got, *want, "compare({a:?}, {b:?}) = {got}, want {want}");
        }
    }

    #[test]
    fn test_clean_tag() {
        // 见 match.rs 的 clean_tag
        let cases: &[(&str, &str)] = &[
            ("v1.2.3", "1.2.3"),
            ("V1.2.3", "1.2.3"),
            ("release-1.2.0", "1.2.0"),
            ("1.2.3", "1.2.3"),
        ];
        for (inp, want) in cases {
            assert_eq!(crate::r#match::clean_tag(inp), *want, "{inp:?}");
        }
    }

    #[test]
    fn test_is_semver() {
        let cases: &[(&str, bool)] = &[
            ("1.2.3", true),
            ("v1.2.3", true),
            ("1.2.3-rc.1", true),
            ("1.2", true),
            ("abc", false),
            ("", false),
            ("1.2.x", false),
        ];
        for (inp, want) in cases {
            assert_eq!(is_semver(inp), *want, "{inp:?}");
        }
    }
}
