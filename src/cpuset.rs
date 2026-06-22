// 自定义 cpuset 分组管理
//
// Android 会按进程优先级把线程分到不同的 cpuset cgroup 分组（top-app /
// foreground / background 等），其中后台分组在不少机型上被限制成只能用
// 小核（省电策略）。这个限制是内核 cpuset 子系统强制的：直接调用
// sched_setaffinity 请求一个不在线程当前 cpuset 允许范围内的核心，会被
// 内核拒绝，返回 EINVAL —— 这不是权限问题，SELinux/capability 都不会拦，
// 单纯是"这个线程当前所在的 cpuset 压根不包含你要的那个核"。
//
// 解决办法：在 /dev/cpuset 下新建一个跟系统自带分组（background/
// foreground/top-app...）平级的自定义分组，下面再按"实际用到的核心组合"
// 各开一个子分组，子分组自己的 cpus 文件就是那个组合本身。绑核前先把
// 线程的 tid 写进对应子分组的 tasks 文件——cgroup v1 里一个线程同一时刻
// 只能属于某个 cpuset 分组下的一个节点，这一步会把它从系统分配的、可能
// 受限的分组里整个换到我们的分组，限制也就跟着换掉了。换完之后再调用
// sched_setaffinity，请求的核心已经完全落在线程当前 cpuset 允许的范围
// 内，不会再被拒绝。
//
// 不支持 cpuset（部分新设备只有 cgroup v2 统一层级，没有 /dev/cpuset
// 这个 cgroup v1 路径）的机型上，这一步会自动跳过，行为退化为只调用
// sched_setaffinity（和不加这个模块之前一样：能绑的能绑，受系统限制的
// 后台线程绑不上）。

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// 自定义分组的根目录，跟系统自带的 background/foreground/top-app 平级
const BASE_DIR: &str = "/dev/cpuset/AppOpt-RS";

pub struct CpusetManager {
    /// /dev/cpuset 不存在，或基础分组创建失败时为 false，
    /// 此时所有操作都直接跳过，调用方退化为只用 sched_setaffinity
    enabled: bool,
    /// mems 文件内容（绝大多数手机单 NUMA 节点，是 "0"），
    /// 创建子分组时需要带上，否则部分内核会拒绝挂载空 mems 的分组
    mems: String,
    /// 已经创建过 cpus/mems 的核心组合（如 "4-6"），避免重复 mkdir/write
    created: HashSet<String>,
}

impl CpusetManager {
    /// 初始化：尝试创建基础分组。
    /// all_cores_str 是本机全部核心的 cpuset 字符串（如 "0-7"），
    /// 用来初始化基础分组自身的 cpus（实际绑核都发生在它的子分组里，
    /// 基础分组本身的范围只在"无具体目标、只是想脱离系统限制"时使用）。
    pub fn init(all_cores_str: &str) -> Self {
        if all_cores_str.is_empty() || !Path::new("/dev/cpuset").exists() {
            return CpusetManager { enabled: false, mems: String::new(), created: HashSet::new() };
        }

        if !create_cpuset_dir(BASE_DIR, all_cores_str, "0") {
            eprintln!("[cpuset] 基础分组创建失败，回退为直接 sched_setaffinity（部分后台线程可能因系统自身的核心限制绑核失败）");
            return CpusetManager { enabled: false, mems: String::new(), created: HashSet::new() };
        }

        // 创建后读回实际的 mems 值（极少数机型创建时会被内核自动调整），
        // 后续所有子分组都用这个值，保证跟基础分组一致
        let mems = fs::read_to_string(format!("{}/mems", BASE_DIR))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "0".to_string());

        eprintln!("[cpuset] 已启用自定义分组 {}（mems={}），可绕过系统对后台线程的核心限制", BASE_DIR, mems);

        CpusetManager { enabled: true, mems, created: HashSet::new() }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// 把 tid 移入"允许 cores_str 这组核心"的自定义子分组。
    /// 子分组不存在则按需创建（每种核心组合全程序运行期间只创建一次）。
    /// 失败（目录创建失败、tasks 写入失败等）静默忽略——后续仍会尝试
    /// 直接 sched_setaffinity，跟没有这层 cpuset 辅助时的行为一致，
    /// 不会因为这一步失败而影响主流程。
    pub fn move_task(&mut self, tid: i32, cores_str: &str) {
        if !self.enabled || cores_str.is_empty() {
            return;
        }

        if !self.created.contains(cores_str) {
            let dir_path = format!("{}/{}", BASE_DIR, cores_str);
            if !create_cpuset_dir(&dir_path, cores_str, &self.mems) {
                return;
            }
            self.created.insert(cores_str.to_string());
        }

        let tasks_path = format!("{}/{}/tasks", BASE_DIR, cores_str);
        if let Ok(mut f) = OpenOptions::new().write(true).append(true).open(&tasks_path) {
            let _ = write!(f, "{}\n", tid);
        }
    }
}

/// 创建一个 cpuset 分组目录并设置它的 cpus/mems。
/// 目录已存在（EEXIST）视为成功，方便重复调用。
fn create_cpuset_dir(path: &str, cpus: &str, mems: &str) -> bool {
    if fs::create_dir(path).is_err() && !Path::new(path).is_dir() {
        return false;
    }
    write_kernel_file(&format!("{}/cpus", path), cpus)
        && write_kernel_file(&format!("{}/mems", path), mems)
}

/// 写 cpuset 暴露的控制文件（cpus/mems），这些是 mkdir 时内核自动生成的
/// 虚拟文件，不需要 O_CREAT；一次 write 就是整体替换当前值，不是追加
fn write_kernel_file(path: &str, content: &str) -> bool {
    match OpenOptions::new().write(true).open(path) {
        Ok(mut f) => f.write_all(content.as_bytes()).is_ok(),
        Err(_) => false,
    }
}
