// 进程扫描与 CPU 亲和性设置
//
// 主要流程：
// 1. 扫描 /proc 找到所有进程
// 2. 读取 /proc/<pid>/cmdline 获取包名
// 3. 遍历 /proc/<pid>/task/<tid>/comm 获取线程名
// 4. 通过 rules::classify_thread 识别线程角色
// 5. 通过 sched_setaffinity 绑核

use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;

use crate::rules::{classify_thread, CoreTarget, detect_engine};
use crate::topo::CpuTopology;
use crate::config::UserConfig;
use crate::cpuset::CpusetManager;

/// 已处理的线程缓存，避免重复设置
#[derive(Default)]
pub struct ProcCache {
    /// 上一轮已处理的 tid 集合
    pub seen_tids: HashSet<i32>,
    /// 上一轮扫描的 pid 集合（用于检测进程退出）
    pub known_pids: HashSet<i32>,
    /// pid -> 包名
    pub pid_pkg: HashMap<i32, String>,
}

impl ProcCache {
    pub fn clear(&mut self) {
        self.seen_tids.clear();
        self.known_pids.clear();
        self.pid_pkg.clear();
    }
}

/// 读取进程的包名（从 cmdline）
fn read_cmdline(pid: i32) -> Option<String> {
    let path = format!("/proc/{}/cmdline", pid);
    let content = fs::read(&path).ok()?;
    // cmdline 以 \0 分隔，取第一段
    let end = content.iter().position(|&b| b == 0).unwrap_or(content.len());
    let raw = std::str::from_utf8(&content[..end]).ok()?;
    // 去掉路径前缀，取最后一段
    let name = raw.rsplit('/').next().unwrap_or(raw);
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// 读取线程名（从 /proc/<pid>/task/<tid>/comm）
fn read_comm(pid: i32, tid: i32) -> Option<String> {
    let path = format!("/proc/{}/task/{}/comm", pid, tid);
    let content = fs::read_to_string(&path).ok()?;
    Some(content.trim().to_string())
}

/// 将 CoreTarget 解析为实际的 cpu_set（cpu 编号列表）
/// 供「custom.toml 覆盖」与「内置启发式规则」两条路径共用
fn resolve_target_cores(target: CoreTarget, topo: &CpuTopology) -> Vec<u32> {
    match target {
        CoreTarget::BigAndPrime => {
            let mut c = topo.big.clone();
            c.extend_from_slice(&topo.prime);
            c
        }
        CoreTarget::LittleAndBig => {
            let mut c = topo.little.clone();
            c.extend_from_slice(&topo.big);
            c
        }
        CoreTarget::Prime => topo.prime.clone(),
        CoreTarget::Big => topo.big.clone(),
        CoreTarget::Little => topo.little.clone(),
        CoreTarget::Default => Vec::new(),
    }
}

/// 设置线程 CPU 亲和性（通过 sched_setaffinity 系统调用）
/// 返回原始结果：0 表示成功，负数为 -errno（失败原因）
fn set_affinity(tid: i32, cores: &[u32]) -> i32 {
    if cores.is_empty() {
        return -22; // EINVAL
    }
    // 构建 cpu_set_t（128字节 = 1024 bit，支持最多1024核）
    let mut cpu_set = [0u8; 128];
    for &cpu in cores {
        if cpu < 1024 {
            cpu_set[(cpu / 8) as usize] |= 1 << (cpu % 8);
        }
    }
    unsafe {
        libc_sched_setaffinity(tid, 128, cpu_set.as_ptr())
    }
}

/// 把常见 errno 转成可读名称，方便从 debug 日志直接判断失败原因：
/// - EPERM/EACCES：权限不足，常见于 SELinux 拒绝跨域 setsched（需要 sepolicy.rule）
/// - ESRCH：线程在两次扫描间已经退出（无害，下一轮会跳过这个已消失的线程）
/// - EINVAL：参数非法（理论上不应出现，出现说明有 bug）
fn errno_name(ret: i32) -> &'static str {
    match -ret {
        1 => "EPERM",
        3 => "ESRCH",
        13 => "EACCES",
        22 => "EINVAL",
        _ => "?",
    }
}

/// sched_setaffinity 系统调用封装
#[cfg(target_arch = "aarch64")]
unsafe fn libc_sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u8) -> i32 {
    let ret: i64;
    std::arch::asm!(
        "svc #0",
        in("x8") 122i64, // __NR_sched_setaffinity on aarch64
        in("x0") pid as i64,
        in("x1") cpusetsize as i64,
        in("x2") mask as i64,
        lateout("x0") ret,
        options(nostack)
    );
    ret as i32
}

#[cfg(target_arch = "arm")]
unsafe fn libc_sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u8) -> i32 {
    let ret: i32;
    std::arch::asm!(
        "swi #0",
        in("r7") 241i32, // __NR_sched_setaffinity on arm32
        in("r0") pid,
        in("r1") cpusetsize as i32,
        in("r2") mask as i32,
        lateout("r0") ret,
        options(nostack)
    );
    ret
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "arm")))]
unsafe fn libc_sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u8) -> i32 {
    // 非 ARM 平台（开发机上编译测试用），通过 libc
    // libc 的约定是返回 -1 并设置全局 errno，这里转换成跟原始 syscall
    // 一致的 "0=成功，负数为 -errno" 约定，方便上层统一处理
    extern "C" {
        fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *const u8) -> i32;
        fn __errno_location() -> *mut i32;
    }
    let ret = sched_setaffinity(pid, cpusetsize, mask);
    if ret == 0 {
        0
    } else {
        -(*__errno_location())
    }
}

/// 一次完整扫描：找到所有进程的线程，识别并绑核
pub fn scan_and_apply(
    topo: &CpuTopology,
    config: &UserConfig,
    cpuset_mgr: &mut CpusetManager,
    cache: &mut ProcCache,
) {
    let proc_dir = match fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return,
    };

    let mut new_pids: HashSet<i32> = HashSet::new();
    let mut new_tids: HashSet<i32> = HashSet::new();

    for entry in proc_dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // 只处理数字目录（pid）
        let pid: i32 = match name_str.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        new_pids.insert(pid);

        // 获取包名（优先用缓存）
        let pkg = if let Some(p) = cache.pid_pkg.get(&pid) {
            p.clone()
        } else {
            match read_cmdline(pid) {
                Some(p) => {
                    cache.pid_pkg.insert(pid, p.clone());
                    p
                }
                None => continue,
            }
        };

        // 检查是否是新进程（或配置更新后需要重新处理）
        let is_new_proc = !cache.known_pids.contains(&pid);

        // pkg 可能形如 "包名" 或 "包名:子进程名"（Android 多进程应用的真实
        // cmdline 就是这个格式）。custom.toml 的 override_app 按纯包名配置，
        // 这里取冒号前半部分用于匹配。
        let base_pkg = pkg.split(':').next().unwrap_or(&pkg);

        // 扫描该进程的所有线程
        let task_path = format!("/proc/{}/task", pid);
        let task_dir = match fs::read_dir(&task_path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut thread_names: Vec<String> = Vec::new();

        // 先收集所有线程名，用于引擎检测
        for tid_entry in task_dir.flatten() {
            let tid_name = tid_entry.file_name();
            let tid_str = tid_name.to_string_lossy();
            let tid: i32 = match tid_str.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };

            if let Some(comm) = read_comm(pid, tid) {
                thread_names.push(comm);
            }
            new_tids.insert(tid);
        }

        if thread_names.is_empty() {
            continue;
        }

        let engine = detect_engine(&thread_names);

        // 再次扫描并应用亲和性
        let task_dir2 = match fs::read_dir(&task_path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        for tid_entry in task_dir2.flatten() {
            let tid_name = tid_entry.file_name();
            let tid_str = tid_name.to_string_lossy();
            let tid: i32 = match tid_str.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };

            // 已处理过且不是新进程，跳过
            if cache.seen_tids.contains(&tid) && !is_new_proc {
                continue;
            }

            let comm = match read_comm(pid, tid) {
                Some(c) => c,
                None => continue,
            };

            // 查找绑核目标，优先级：
            //   1) custom.toml 用户覆盖（按包名或全局线程名）
            //   2) 内置启发式规则（rules.rs，按 Prime/Big/Little/LittleAndBig
            //      角色分类，规则表是对大量真实 App 线程命名数据统计挖掘得出的）
            let mut source = "builtin";
            let cores: Vec<u32>;

            if let Some(target) = config.get_app_override(base_pkg, &comm)
                .or_else(|| config.get_override(&comm))
            {
                source = "toml";
                cores = resolve_target_cores(target, topo);
            } else {
                let target = classify_thread(&comm);
                if target == CoreTarget::Default {
                    continue;
                }
                cores = resolve_target_cores(target, topo);
            }

            if cores.is_empty() {
                continue;
            }

            let cpuset_str = CpuTopology::cores_to_cpuset(&cores);

            // 第一步：把线程移入允许这组核心的自定义 cpuset 分组
            // 这一步解决 Android 后台省电 cpuset 限制导致的 EINVAL 问题：
            // 系统把后台 App 的线程限制在小核组，直接调 sched_setaffinity
            // 请求大核会被内核拒绝（线程当前 cpuset 不包含目标核心）；
            // 换入自定义分组后，限制跟着换掉，后续的 sched_setaffinity 才能成功。
            // CpusetManager 不可用（设备没有 /dev/cpuset）时此调用直接返回，无副作用。
            cpuset_mgr.move_task(tid, &cpuset_str);

            // 第二步：sched_setaffinity 精确绑核
            let ret = set_affinity(tid, &cores);

            if config.settings.log_level == "debug" {
                if ret == 0 {
                    eprintln!(
                        "[affinity] pid={} pkg={} tid={} comm={} engine={:?} src={} -> cores={} ✓",
                        pid, pkg, tid, comm, engine, source, cpuset_str
                    );
                } else {
                    eprintln!(
                        "[affinity] pid={} pkg={} tid={} comm={} engine={:?} src={} -> cores={} ✗ errno={}({})",
                        pid, pkg, tid, comm, engine, source, cpuset_str, -ret, errno_name(ret)
                    );
                }
            }
        }
    }

    // 更新缓存
    cache.known_pids = new_pids;
    cache.seen_tids = new_tids;

    // 清理退出进程的包名缓存
    cache.pid_pkg.retain(|pid, _| cache.known_pids.contains(pid));
}
