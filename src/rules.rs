// 线程角色识别与绑核策略
//
// 设计原则（来自 applist.prop 数据分析）：
//
// PRIME (超大核，天玑9400+为core7@3730MHz)：
//   → 主渲染线程、游戏主线程、最高优先级 UI 线程
//   → RenderThread, UnityMain, GameThread, UEGameThread, CrRendererMain 等
//
// BIG (大核，天玑9400+为core4-6@3300MHz)：
//   → 工作线程、IO线程、网络、GPU辅助、Binder、编解码
//   → Thread-*, mali-*, Job.Worker*, Chrome_*, OkHttp*, ExoPlayer:* 等
//
// LITTLE (小核/能效核，天玑9400+为core0-3@2400MHz)：
//   → 音频、后台加载、日志、GC辅助
//   → FMOD*, *Audio*, UnityChoreograp, HeapTaskDaemon (后台时)
//
// 注意：天玑9400+ 的"小核"其实是 A720@2400MHz，性能不弱，
// 但相比 X4/X925 仍是能效核，适合后台任务。

/// 线程绑核目标
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreTarget {
    /// 超大核（单核最强，用于主渲染/游戏主线程）
    Prime,
    /// 大核（高性能，用于工作线程/IO/网络）
    Big,
    /// 大核+超大核（渲染辅助线程，需要高性能但不独占Prime）
    BigAndPrime,
    /// 小核/能效核（后台任务、音频解码、GC）
    Little,
    /// 不干预（使用系统默认调度）
    Default,
}

/// 单条线程匹配规则
#[derive(Debug, Clone)]
pub struct ThreadRule {
    /// 线程名模式，支持通配符 * 和 ?
    pub pattern: &'static str,
    /// 目标核心类型
    pub target: CoreTarget,
    /// 优先级（越高越优先，精确匹配 > 通配符）
    pub priority: u8,
}

impl ThreadRule {
    const fn exact(pattern: &'static str, target: CoreTarget) -> Self {
        ThreadRule { pattern, target, priority: 100 }
    }
    const fn wildcard(pattern: &'static str, target: CoreTarget) -> Self {
        ThreadRule { pattern, target, priority: 50 }
    }
    const fn low(pattern: &'static str, target: CoreTarget) -> Self {
        ThreadRule { pattern, target, priority: 20 }
    }
}

/// 判断线程名是否匹配 pattern（支持 * 和 ?）
pub fn thread_matches(pattern: &str, name: &str) -> bool {
    // 精确匹配
    if pattern == name {
        return true;
    }
    // 无通配符
    if !pattern.contains('*') && !pattern.contains('?') {
        return false;
    }
    // 通配符匹配（简单实现，不依赖外部库）
    wildcard_match(pattern.as_bytes(), name.as_bytes())
}

fn wildcard_match(pat: &[u8], text: &[u8]) -> bool {
    let (mut pi, mut ti) = (0, 0);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0);

    while ti < text.len() {
        if pi < pat.len() && (pat[pi] == b'?' || pat[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pat.len() && pat[pi] == b'*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            star_ti += 1;
            ti = star_ti;
            pi = star_pi + 1;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' {
        pi += 1;
    }
    pi == pat.len()
}

/// 内置线程规则表
/// 顺序：精确匹配在前，通配符在后；同优先级下靠前的规则优先
pub static THREAD_RULES: &[ThreadRule] = &[
    // ========== PRIME：主渲染/游戏主线程 ==========
    // 系统渲染
    ThreadRule::exact("RenderThread",       CoreTarget::Prime),
    ThreadRule::exact("RenderEngine",       CoreTarget::Prime),
    // Android UI
    ThreadRule::exact("main",               CoreTarget::Prime),
    ThreadRule::exact("1.ui",               CoreTarget::Prime),
    ThreadRule::exact("2.ui",               CoreTarget::Prime),
    ThreadRule::exact("InputDispatcher",    CoreTarget::Prime),
    // Unity
    ThreadRule::exact("UnityMain",          CoreTarget::Prime),
    ThreadRule::exact("UnityGfxDeviceW",    CoreTarget::Prime),
    // Unreal
    ThreadRule::exact("GameThread",         CoreTarget::Prime),
    ThreadRule::exact("UEGameThread",       CoreTarget::Prime),
    ThreadRule::exact("RenderingThread",    CoreTarget::Prime),
    // Chromium/WebView
    ThreadRule::exact("CrRendererMain",     CoreTarget::Prime),
    ThreadRule::exact("Chrome_InProcGp",    CoreTarget::Prime),
    // Flutter
    ThreadRule::exact("1.raster",           CoreTarget::Prime),
    ThreadRule::exact("?.raster",           CoreTarget::Prime),
    // 其他主线程类
    ThreadRule::exact("MainThread",         CoreTarget::Prime),
    ThreadRule::exact("VCMainThread",       CoreTarget::Prime),
    ThreadRule::exact("Compositor",         CoreTarget::Prime),
    ThreadRule::exact("AnimationThread",    CoreTarget::Prime),

    // ========== BIG+PRIME：渲染辅助，高优先级但不独占Prime ==========
    ThreadRule::exact("UnityGfx*",          CoreTarget::BigAndPrime),
    ThreadRule::exact("GLThread*",          CoreTarget::BigAndPrime),
    ThreadRule::exact("VulkanRenderMan",    CoreTarget::BigAndPrime),
    ThreadRule::exact("Chrome_InProcRe",    CoreTarget::BigAndPrime),
    ThreadRule::exact("1.raster",           CoreTarget::BigAndPrime), // 已在Prime，此处备用

    // ========== BIG：工作线程 / IO / 网络 / GPU辅助 ==========
    // GPU/图形驱动
    ThreadRule::wildcard("mali-*",          CoreTarget::Big),
    ThreadRule::wildcard("RenderThread*",   CoreTarget::Big),
    ThreadRule::wildcard("hwuiTask*",       CoreTarget::Big),
    // Unity 工作线程
    ThreadRule::wildcard("Job.[Ww]orker*",  CoreTarget::Big),
    ThreadRule::wildcard("UnityPreload*",   CoreTarget::Big),
    ThreadRule::exact("IL2CPP*",            CoreTarget::Big),
    // Unreal 辅助
    ThreadRule::exact("RHIThread",          CoreTarget::Big),
    ThreadRule::wildcard("TaskGraph*",      CoreTarget::Big),
    ThreadRule::exact("ShaderCompiling",    CoreTarget::Big),
    // Flutter
    ThreadRule::wildcard("DartWorker*",     CoreTarget::Big),
    ThreadRule::wildcard("?.io",            CoreTarget::Big),
    // Chromium
    ThreadRule::exact("Chrome_IOThread",    CoreTarget::Big),
    ThreadRule::exact("Chrome_ChildIOT",    CoreTarget::Big),
    ThreadRule::exact("NetworkService",     CoreTarget::Big),
    ThreadRule::exact("VizWebView",         CoreTarget::Big),
    ThreadRule::exact("JavaBridge",         CoreTarget::Big),
    ThreadRule::wildcard("ChromiumNet*",    CoreTarget::Big),
    // React Native
    ThreadRule::wildcard("mqt_*",           CoreTarget::Big),
    // 通用工作线程
    ThreadRule::wildcard("Thread-*",        CoreTarget::Big),
    ThreadRule::wildcard("NativeThread*",   CoreTarget::Big),
    ThreadRule::wildcard("*worker*",        CoreTarget::Big),
    ThreadRule::wildcard("*Worker*",        CoreTarget::Big),
    ThreadRule::wildcard("Worker Thread*",  CoreTarget::Big),
    ThreadRule::wildcard("*[Pp]ool*",       CoreTarget::Big),
    ThreadRule::wildcard("Task*",           CoreTarget::Big),
    ThreadRule::wildcard("JobThread*",      CoreTarget::Big),
    // Binder IPC
    ThreadRule::wildcard("Binder:*",        CoreTarget::Big),
    ThreadRule::wildcard("binder:*",        CoreTarget::Big),
    ThreadRule::wildcard("HwBinder:*",      CoreTarget::Big),
    // 网络/IO
    ThreadRule::wildcard("OkHttp*",         CoreTarget::Big),
    ThreadRule::wildcard("*[Hh]ttp*",       CoreTarget::Big),
    ThreadRule::wildcard("ExoPlayer:*",     CoreTarget::Big),
    ThreadRule::wildcard("MediaCodec*",     CoreTarget::Big),
    ThreadRule::wildcard("CodecLooper*",    CoreTarget::Big),
    ThreadRule::wildcard("glide*",          CoreTarget::Big),
    ThreadRule::wildcard("storageQueue*",   CoreTarget::Big),
    ThreadRule::wildcard("arch_disk_*",     CoreTarget::Big),
    ThreadRule::exact("HeapTaskDaemon",     CoreTarget::Big),
    ThreadRule::exact("DefaultDispatch",    CoreTarget::Big),
    ThreadRule::wildcard("AsyncTask*",      CoreTarget::Big),
    ThreadRule::wildcard("*[Aa]nim*",       CoreTarget::Big),

    // ========== LITTLE：音频/后台/GC ==========
    // 音频
    ThreadRule::wildcard("FMOD*",           CoreTarget::Little),
    ThreadRule::wildcard("*[Aa]udio*",      CoreTarget::Little),
    ThreadRule::exact("AudioThread",        CoreTarget::Little),
    ThreadRule::exact("AudioTrack",         CoreTarget::Little),
    ThreadRule::wildcard("SoundDecoder*",   CoreTarget::Little),
    ThreadRule::wildcard("decodeQueue*",    CoreTarget::Little),
    // Unity 编曲/节拍（低优先级）
    ThreadRule::wildcard("UnityChoreograp*",CoreTarget::Little),
    ThreadRule::wildcard("UnityChoreo*",    CoreTarget::Little),
    // GC 辅助
    ThreadRule::wildcard("GC?Marker??",     CoreTarget::Little),
    ThreadRule::wildcard("[Bb][Gg]*",       CoreTarget::Little),
    // 后台加载
    ThreadRule::wildcard("*Loading*",       CoreTarget::Little),
    ThreadRule::wildcard("FAsyncLoading*",  CoreTarget::Little),
    ThreadRule::wildcard("Loading.Preload*",CoreTarget::Little),
    ThreadRule::wildcard("ThreadPoolServi*",CoreTarget::Little),
    ThreadRule::wildcard("ThreadPoolForeg*",CoreTarget::Little),
    // 日志/上报
    ThreadRule::exact("LogAppendWork",      CoreTarget::Little),
    ThreadRule::wildcard("IO-*",            CoreTarget::Little),
];

/// 查找线程对应的绑核目标
/// 返回第一个匹配的规则目标；无匹配则返回 Default
pub fn classify_thread(thread_name: &str) -> CoreTarget {
    let mut best_target = CoreTarget::Default;
    let mut best_priority = 0u8;

    for rule in THREAD_RULES {
        if thread_matches(rule.pattern, thread_name) {
            if rule.priority > best_priority {
                best_priority = rule.priority;
                best_target = rule.target;
            }
        }
    }

    best_target
}

/// 检测进程使用的引擎（用于日志/调试）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineHint {
    Unity,
    Unreal,
    Flutter,
    Chromium,
    Native,
    Unknown,
}

pub fn detect_engine(thread_names: &[String]) -> EngineHint {
    for name in thread_names {
        let n = name.as_str();
        if n == "UnityMain" || n.starts_with("UnityGfx") {
            return EngineHint::Unity;
        }
        if n == "GameThread" || n == "UEGameThread" || n == "RHIThread" {
            return EngineHint::Unreal;
        }
        if n == "1.ui" || n == "1.raster" || n.starts_with("DartWorker") {
            return EngineHint::Flutter;
        }
        if n == "CrRendererMain" || n == "Chrome_IOThread" || n == "Chrome_InProcGp" {
            return EngineHint::Chromium;
        }
    }
    EngineHint::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert_eq!(classify_thread("RenderThread"), CoreTarget::Prime);
        assert_eq!(classify_thread("UnityMain"), CoreTarget::Prime);
        assert_eq!(classify_thread("GameThread"), CoreTarget::Prime);
        assert_eq!(classify_thread("AudioTrack"), CoreTarget::Little);
        assert_eq!(classify_thread("HeapTaskDaemon"), CoreTarget::Big);
    }

    #[test]
    fn test_wildcard_match() {
        assert_eq!(classify_thread("mali-cmar-iface"), CoreTarget::Big);
        assert_eq!(classify_thread("Job.Worker1"), CoreTarget::Big);
        assert_eq!(classify_thread("FMOD stream"), CoreTarget::Little);
        assert_eq!(classify_thread("UnityChoreograph"), CoreTarget::Little);
        assert_eq!(classify_thread("Binder:1234_5"), CoreTarget::Big);
        assert_eq!(classify_thread("OkHttp ConnectionPool"), CoreTarget::Big);
    }

    #[test]
    fn test_unknown_thread() {
        assert_eq!(classify_thread("SomeRandomThread"), CoreTarget::Default);
    }

    #[test]
    fn test_wildcard_fn() {
        assert!(thread_matches("Thread-*", "Thread-123"));
        assert!(thread_matches("mali-*", "mali-cmar-iface"));
        assert!(thread_matches("?.raster", "1.raster"));
        assert!(!thread_matches("Thread-*", "MainThread"));
    }
}
