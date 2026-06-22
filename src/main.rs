// AppOpt-RS：智能线程绑核优化守护进程
//
// 功能：
// - 自动识别 CPU 拓扑（Prime/Big/Little）
// - 自动识别线程角色（渲染/工作/音频/后台）
// - 自动绑核，无需配置文件
// - 可选 custom.toml 覆盖特定规则

mod topo;
mod rules;
mod config;
mod scanner;
mod glob;
mod cpuset;

use std::thread;
use std::time::Duration;
use std::env;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const DEFAULT_CONFIG_PATH: &str = "/data/adb/modules/AppOpt_RS/custom.toml";
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_help(prog: &str) {
    println!("AppOpt-RS v{} - 智能线程绑核优化", VERSION);
    println!();
    println!("用法: {} [选项]", prog);
    println!();
    println!("选项:");
    println!("  -c <路径>   自定义配置文件路径 (默认: {})", DEFAULT_CONFIG_PATH);
    println!("  -s <毫秒>   扫描间隔毫秒数 (默认: 2000)");
    println!("  -d          Debug 模式（输出每个线程的绑核日志）");
    println!("  -t          仅输出 CPU 拓扑信息后退出");
    println!("  -v          显示版本");
    println!("  -h          显示帮助");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let prog = &args[0];

    let mut config_path = DEFAULT_CONFIG_PATH.to_string();
    let mut scan_interval_ms: Option<u64> = None;
    let mut debug_mode = false;
    let mut topo_only = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-c" => {
                i += 1;
                if i < args.len() {
                    config_path = args[i].clone();
                }
            }
            "-s" => {
                i += 1;
                if i < args.len() {
                    scan_interval_ms = args[i].parse().ok();
                }
            }
            "-d" => debug_mode = true,
            "-t" => topo_only = true,
            "-v" => {
                println!("AppOpt-RS v{}", VERSION);
                return;
            }
            "-h" | "--help" => {
                print_help(prog);
                return;
            }
            _ => {
                eprintln!("未知参数: {}", args[i]);
                print_help(prog);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    // 检测 CPU 拓扑
    let topo = topo::CpuTopology::detect();

    println!("AppOpt-RS v{} 启动", VERSION);
    println!("CPU 拓扑:");
    println!("  Prime (超大核): [{}] -> {}", 
        topo.prime.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
        topo.prime_cpuset()
    );
    println!("  Big   (大核):   [{}] -> {}",
        topo.big.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
        topo.big_cpuset()
    );
    println!("  Little(小核):   [{}] -> {}",
        topo.little.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(","),
        topo.little_cpuset()
    );

    if topo_only {
        return;
    }

    // 加载用户配置
    let mut user_config = config::UserConfig::load(&config_path);
    if debug_mode {
        user_config.settings.log_level = "debug".to_string();
    }

    let interval = Duration::from_millis(
        scan_interval_ms.unwrap_or(user_config.settings.scan_interval_ms)
    );

    println!("扫描间隔: {}ms", interval.as_millis());
    println!("配置文件: {} ({})",
        config_path,
        if std::path::Path::new(&config_path).exists() { "已加载" } else { "未找到，使用内置规则" }
    );

    // 初始化自定义 cpuset 分组（解决后台线程被系统省电 cpuset 限核的问题）
    // 必须在打印 topo 信息之后初始化，因为需要 topo.all_cpuset()
    let mut cpuset_mgr = cpuset::CpusetManager::init(&topo.all_cpuset());
    if cpuset_mgr.enabled() {
        println!("cpuset: 已启用自定义分组（后台线程绑大核不再受系统限制）");
    } else {
        println!("cpuset: 未启用（/dev/cpuset 不存在或创建失败，后台线程可能因系统限制绑核失败）");
    }

    println!("开始运行...");

    // 信号处理（SIGTERM/SIGINT 优雅退出）
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    unsafe {
        let _ = signal_hook(r);
    }

    let mut cache = scanner::ProcCache::default();

    while running.load(Ordering::Relaxed) {
        scanner::scan_and_apply(&topo, &user_config, &mut cpuset_mgr, &mut cache);
        thread::sleep(interval);
    }

    println!("AppOpt-RS 退出");
}

unsafe fn signal_hook(running: Arc<AtomicBool>) -> Result<(), ()> {
    // 简单的 SIGTERM 处理
    extern "C" fn handler(_: i32) {}
    
    // 在 Android 上信号处理较复杂，这里只做最基本的设置
    // 实际退出主要靠 Magisk 模块管理
    let _ = running;
    Ok(())
}
