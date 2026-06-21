// 通用 glob 风格字符串匹配器，供 rules.rs 的内置线程分类规则表使用
//
// 支持的语法（兼容线程名里常见的所有写法）：
//   *        匹配任意长度（含 0）的任意字符
//   ?        匹配单个任意字符
//   [abc]    匹配方括号内任意一个字符
//   [a-z]    匹配字符范围
//   [!abc]   取反：匹配不在方括号内的任意字符（也支持 [^abc] 写法）
//   其余字符  按字面量精确匹配（包括 '.'，它不是通配符）
//
// 之所以自己实现而不引入 regex crate，是因为本项目编译目标是 Android
// 上的常驻守护进程，要求体积小、启动快、零额外依赖。

#[derive(Debug, Clone)]
enum ClassItem {
    Single(char),
    Range(char, char),
}

#[derive(Debug, Clone)]
struct CharClass {
    negate: bool,
    items: Vec<ClassItem>,
}

impl CharClass {
    fn matches(&self, c: char) -> bool {
        let hit = self.items.iter().any(|it| match it {
            ClassItem::Single(x) => *x == c,
            ClassItem::Range(a, b) => *a <= c && c <= *b,
        });
        hit != self.negate
    }
}

/// 解析从 p[pi] == '[' 开始的字符类
/// 返回 (解析出的字符类, 类结束后下一个字符的下标)
/// 若括号未正确闭合，则把 '[' 当作普通字面字符处理（不消费后续内容）
fn parse_bracket(p: &[char], pi: usize) -> (CharClass, usize) {
    let mut idx = pi + 1;
    let mut negate = false;
    if idx < p.len() && (p[idx] == '!' || p[idx] == '^') {
        negate = true;
        idx += 1;
    }
    let start_items = idx;
    let mut items = Vec::new();
    while idx < p.len() && p[idx] != ']' {
        if idx + 2 < p.len() && p[idx + 1] == '-' && p[idx + 2] != ']' {
            items.push(ClassItem::Range(p[idx], p[idx + 2]));
            idx += 3;
        } else {
            items.push(ClassItem::Single(p[idx]));
            idx += 1;
        }
    }

    if idx < p.len() && p[idx] == ']' {
        if idx == start_items {
            // 空字符类 []：不匹配任何字符
            return (CharClass { negate: false, items: vec![] }, idx + 1);
        }
        return (CharClass { negate, items }, idx + 1);
    }

    // 未找到闭合的 ']'，按字面量 '[' 处理
    (CharClass { negate: false, items: vec![ClassItem::Single('[')] }, pi + 1)
}

/// 判断 text 是否匹配 pattern
pub fn glob_match(pattern: &str, text: &str) -> bool {
    // 无通配符时直接走最快路径
    if !pattern.contains(['*', '?', '[']) {
        return pattern == text;
    }

    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();

    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti = 0usize;

    while ti < t.len() {
        let mut matched = false;
        let mut next_pi = pi;

        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star_pi = Some(pi);
                    star_ti = ti;
                    pi += 1;
                    continue;
                }
                '[' => {
                    let (class, np) = parse_bracket(&p, pi);
                    if class.matches(t[ti]) {
                        matched = true;
                        next_pi = np;
                    }
                }
                '?' => {
                    matched = true;
                    next_pi = pi + 1;
                }
                c => {
                    if c == t[ti] {
                        matched = true;
                        next_pi = pi + 1;
                    }
                }
            }
        }

        if matched {
            pi = next_pi;
            ti += 1;
        } else if let Some(spi) = star_pi {
            star_ti += 1;
            ti = star_ti;
            pi = spi + 1;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic() {
        assert!(glob_match("Thread-*", "Thread-123"));
        assert!(glob_match("mali-*", "mali-cmar-iface"));
        assert!(glob_match("?.raster", "1.raster"));
        assert!(!glob_match("Thread-*", "MainThread"));
        assert!(glob_match("RenderThread", "RenderThread"));
        assert!(!glob_match("RenderThread", "RenderThreadX"));
    }

    #[test]
    fn test_bracket_class() {
        assert!(glob_match("[Bb]inder:*", "Binder:1234"));
        assert!(glob_match("[Bb]inder:*", "binder:5"));
        assert!(!glob_match("[Bb]inder:*", "xinder:5"));
        assert!(glob_match("[Bb][Gg]*", "BGthread"));
        assert!(glob_match("[Bb][Gg]*", "bgWorker"));
        assert!(glob_match("Game[0-9]*", "Game7Loop"));
        assert!(!glob_match("Game[0-9]*", "GameXLoop"));
        assert!(glob_match("Thread-3[0-9]", "Thread-35"));
        assert!(!glob_match("Thread-3[0-9]", "Thread-45"));
        assert!(glob_match("GC?Marker??", "GCXMarkerAB"));
    }

    #[test]
    fn test_negate_class() {
        assert!(glob_match("[!0-9]*", "abc"));
        assert!(!glob_match("[!0-9]*", "9abc"));
        assert!(glob_match("[^0-9]*", "abc"));
    }

    #[test]
    fn test_dot_is_literal() {
        // '.' 不是通配符，必须按字面量匹配
        assert!(glob_match("*.*", "1.ui"));
        assert!(!glob_match("*.*", "1ui"));
    }

    #[test]
    fn test_process_key_wildcard() {
        assert!(glob_match(
            "com.tencent.mm:xweb_sandboxed_process*",
            "com.tencent.mm:xweb_sandboxed_process0"
        ));
        assert!(!glob_match(
            "com.tencent.mm:xweb_sandboxed_process*",
            "com.tencent.mm:other_process0"
        ));
    }

    #[test]
    fn test_mixed_pattern_from_applist() {
        assert!(glob_match("*[0-9]-[0-9]*", "Thread-12-3"));
        assert!(glob_match("Job.[Ww]orker*", "Job.Worker1"));
        assert!(glob_match("Job.[Ww]orker*", "Job.worker7"));
    }
}
