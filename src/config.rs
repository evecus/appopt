// 用户自定义覆盖配置（可选）
// 路径：/data/adb/modules/AppOpt_RS/custom.toml
//
// 普通用户完全不需要这个文件，程序自动识别。
// 仅在自动识别结果不符合预期时才需要手动覆盖。
//
// 配置格式示例：
//
// [override]
// # 按线程名覆盖，适用于所有 App
// "RenderThread" = "prime"
// "MyCustomThread" = "big"
//
// [override_app."com.example.game"]
// # 仅针对特定包名覆盖
// "GameMainThread" = "prime"
//
// [settings]
// scan_interval_ms = 2000   # 扫描间隔（毫秒），默认 2000
// log_level = "info"        # 日志级别：debug / info / warn

use std::collections::HashMap;
use std::fs;
use serde::Deserialize;
use crate::rules::CoreTarget;

#[derive(Debug, Deserialize, Default)]
pub struct UserConfig {
    #[serde(default)]
    pub override_threads: HashMap<String, String>,

    #[serde(default)]
    pub override_app: HashMap<String, HashMap<String, String>>,

    #[serde(default)]
    pub settings: Settings,
}

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub scan_interval_ms: u64,
    pub log_level: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            scan_interval_ms: 2000,
            log_level: "info".to_string(),
        }
    }
}

impl UserConfig {
    pub fn load(path: &str) -> Self {
        match fs::read_to_string(path) {
            Ok(content) => {
                match toml::from_str::<RawConfig>(&content) {
                    Ok(raw) => {
                        eprintln!("[config] 加载用户配置: {}", path);
                        UserConfig {
                            override_threads: raw.override_threads.unwrap_or_default(),
                            override_app: raw.override_app.unwrap_or_default(),
                            settings: raw.settings.unwrap_or_default(),
                        }
                    }
                    Err(e) => {
                        eprintln!("[config] 配置文件解析失败: {}", e);
                        UserConfig::default()
                    }
                }
            }
            Err(_) => {
                // 没有配置文件是正常情况，不报错
                UserConfig::default()
            }
        }
    }

    /// 查询线程的覆盖目标（全局覆盖）
    pub fn get_override(&self, thread_name: &str) -> Option<CoreTarget> {
        self.override_threads.get(thread_name)
            .and_then(|s| parse_target(s))
    }

    /// 查询特定包名下线程的覆盖目标
    pub fn get_app_override(&self, pkg: &str, thread_name: &str) -> Option<CoreTarget> {
        self.override_app.get(pkg)
            .and_then(|m| m.get(thread_name))
            .and_then(|s| parse_target(s))
    }
}

fn parse_target(s: &str) -> Option<CoreTarget> {
    match s.to_lowercase().as_str() {
        "prime" => Some(CoreTarget::Prime),
        "big" => Some(CoreTarget::Big),
        "big+prime" | "bigandprime" => Some(CoreTarget::BigAndPrime),
        "little" => Some(CoreTarget::Little),
        "default" | "none" => Some(CoreTarget::Default),
        _ => {
            eprintln!("[config] 未知核心类型: {}", s);
            None
        }
    }
}

// TOML 反序列化用的原始结构
#[derive(Deserialize)]
struct RawConfig {
    #[serde(rename = "override")]
    override_threads: Option<HashMap<String, String>>,

    #[serde(rename = "override_app")]
    override_app: Option<HashMap<String, HashMap<String, String>>>,

    settings: Option<Settings>,
}
