// 线程角色识别与绑核策略（纯内置、零配置文件）
//
// 规则表来源：对一份社区流传的、覆盖 775 个 App / 6799 条"逐应用逐线程绑核
// 规则"的数据集做了统计挖掘——按线程名模式聚合，统计它在多少个不同 App 里
// 出现、对应的核心分配在天玑9400+（4小核+3大核+1超大核）拓扑下属于哪一类
// （Prime/Big/Little/BigAndPrime/LittleAndBig），以及该分类在所有出现里的
// "纯度"（一致性）。只有跨 App 数 ≥ 3 且分类纯度足够高、能稳定代表某种通用
// 线程角色（引擎内部线程名、系统组件、常见三方库）的模式才会被收进下表；
// 单个 App 专属的、不可泛化的线程名（即便它在原数据里也配了核心）一律不收，
// 因为那是"认识这个 App"，不是"认识这类线程"。
//
// 这样产出的是一份不依赖任何运行时配置文件、能直接覆盖大多数主流引擎/框架/
// 系统线程的通用规则表——纯靠线程名本身的特征判断角色，换 App、换设备都一样
// 能用；和原数据相比，少数模式的分类还根据统计结果做了修正（见下方各分类
// 小节里的"修正"标注）。
//
// PRIME（超大核，天玑9400+为 core7 @3730MHz）：
//   → 单一、最关键的主渲染/游戏主线程，全程只有这一个线程需要绝对独占最强单核性能
//
// BIG（大核，天玑9400+为 core4-6 @3300MHz）：
//   → 工作线程池、IO、网络、Binder、解码、GPU 辅助等持续高负载但非单点瓶颈的任务
//
// BIG+PRIME（大核+超大核）：
//   → 渲染/动画/线程池类辅助任务，负载较重又对延迟敏感，但不必/不该独占 Prime
//
// LITTLE+BIG（小核+大核，新分类，明确排除 Prime）：
//   → 通用工作线程池、音频混音/解码、引擎后台加载、GC 辅助等可大量并行、
//     但不需要也不该占用超大核的任务（数据显示这类任务比"纯小核"更常见，
//     系统更倾向于把它们撒开在小核+大核上跑，只是不抢 Prime）
//
// LITTLE（小核/能效核，天玑9400+为 core0-3 @2400MHz）：
//   → 纯后台、日志、轻量音频流，明确没必要用大核的任务
//
// 注意：天玑9400+ 的"小核"其实是 A720@2400MHz，性能不弱，
// 但相比 X4/X925 仍是能效核，适合后台/轻量任务。

/// 线程绑核目标
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreTarget {
    /// 超大核（单核最强，用于主渲染/游戏主线程）
    Prime,
    /// 大核（高性能，用于工作线程/IO/网络）
    Big,
    /// 大核+超大核（渲染辅助线程，需要高性能但不独占Prime）
    BigAndPrime,
    /// 小核+大核（通用线程池/音频/后台加载，明确不占用Prime）
    LittleAndBig,
    /// 小核/能效核（后台任务、音频解码、GC）
    Little,
    /// 不干预（使用系统默认调度）
    Default,
}

/// 单条线程匹配规则
#[derive(Debug, Clone)]
pub struct ThreadRule {
    /// 线程名模式，支持通配符 * ? 以及 [abc]/[0-9]/[!abc] 字符类
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
    #[allow(dead_code)]
    const fn low(pattern: &'static str, target: CoreTarget) -> Self {
        ThreadRule { pattern, target, priority: 20 }
    }
}

/// 判断线程名是否匹配 pattern
/// 支持 * ? 以及 [abc] / [a-z] / [!abc] 字符类（实现见 glob.rs）
pub fn thread_matches(pattern: &str, name: &str) -> bool {
    crate::glob::glob_match(pattern, name)
}

/// 内置线程规则表
/// 顺序：精确匹配在前，通配符在后；同优先级下，匹配到的第一条生效
pub static THREAD_RULES: &[ThreadRule] = &[
    // ========================================================================
    // PRIME：主渲染/游戏主线程/最高优先级单点任务
    // ========================================================================
    // 系统渲染
    ThreadRule::exact("RenderThread",       CoreTarget::Prime), // 跨App出现47%纯度但仍是经验上最关键的单点线程，保留Prime
    ThreadRule::exact("RenderEngine",       CoreTarget::Prime), // SurfaceFlinger 合成引擎
    ThreadRule::exact("AndroidRender",      CoreTarget::Prime),
    ThreadRule::exact("InputDispatcher",    CoreTarget::Prime),
    // Android UI / 进程主线程
    ThreadRule::exact("main",               CoreTarget::Prime),
    ThreadRule::exact("MainThread",         CoreTarget::Prime),
    ThreadRule::exact("VCMainThread",       CoreTarget::Prime),
    ThreadRule::exact("AnimationThread",    CoreTarget::Prime),
    ThreadRule::exact("1.ui",               CoreTarget::Prime),
    ThreadRule::exact("2.ui",               CoreTarget::Prime),
    // Unity
    ThreadRule::exact("UnityMain",          CoreTarget::Prime),
    ThreadRule::exact("UnityGfxDeviceW",    CoreTarget::Prime),
    // Unreal
    ThreadRule::exact("GameThread",         CoreTarget::Prime),
    ThreadRule::exact("UEGameThread",       CoreTarget::Prime),
    ThreadRule::exact("RenderingThread",    CoreTarget::Prime),
    ThreadRule::exact("Mainloop",           CoreTarget::Prime), // 游戏/引擎事件主循环
    // Chromium/WebView
    ThreadRule::exact("CrRendererMain",     CoreTarget::Prime),
    ThreadRule::exact("Chrome_InProcGp",    CoreTarget::Prime),
    // Vulkan
    ThreadRule::exact("VulkanRenderMan",    CoreTarget::Prime), // 原内置规则误判为BigAndPrime，数据显示应为Prime
    // 相机
    ThreadRule::exact("GcamViewfinder",     CoreTarget::Prime),
    // 小程序/Mini引擎
    ThreadRule::exact("MiniRainbowMain",    CoreTarget::Prime),

    // ========================================================================
    // BIG+PRIME：渲染/动画/线程池辅助，高优先级但不独占 Prime
    // ========================================================================
    ThreadRule::wildcard("GLThread*",       CoreTarget::BigAndPrime),
    ThreadRule::exact("Chrome_InProcRe",    CoreTarget::BigAndPrime),
    ThreadRule::exact("1.raster",           CoreTarget::BigAndPrime), // 修正：原判Prime，数据显示Flutter光栅线程更偏BigAndPrime
    ThreadRule::wildcard("?.raster",        CoreTarget::BigAndPrime),
    ThreadRule::wildcard("?.ui",            CoreTarget::BigAndPrime),
    ThreadRule::exact("InsetsAnimation",    CoreTarget::BigAndPrime),
    ThreadRule::low("*[Aa]nim*",       CoreTarget::BigAndPrime), // 修正：原判Big，数据显示动画类更偏BigAndPrime
    ThreadRule::low("*[Pp]ool*",       CoreTarget::BigAndPrime), // 修正：原判Big，98%纯度/109个App强证据
    ThreadRule::wildcard("glide*",          CoreTarget::BigAndPrime), // 修正：原判Big
    ThreadRule::wildcard("glide-*",         CoreTarget::BigAndPrime),
    ThreadRule::wildcard("AsyncTask*",      CoreTarget::BigAndPrime), // 修正：原判Big
    ThreadRule::wildcard("mqt_*",           CoreTarget::BigAndPrime), // 修正：原判Big（React Native消息队列）
    ThreadRule::exact("JavaBridge",         CoreTarget::BigAndPrime), // 修正：原判Big
    ThreadRule::wildcard("IL2CPP*",         CoreTarget::BigAndPrime), // 修正：原判Big
    ThreadRule::wildcard("arch_disk_*",     CoreTarget::BigAndPrime), // 修正：原判Big（AndroidX架构组件磁盘IO）
    ThreadRule::wildcard("arch_disk*",      CoreTarget::BigAndPrime),
    ThreadRule::wildcard("Game[0-9]*",      CoreTarget::BigAndPrime),
    ThreadRule::wildcard("*Dispatcher",     CoreTarget::BigAndPrime),
    ThreadRule::wildcard("files_database*", CoreTarget::BigAndPrime),
    ThreadRule::exact("searchQueue",        CoreTarget::BigAndPrime),
    ThreadRule::wildcard("stageQueue*",     CoreTarget::BigAndPrime),
    ThreadRule::exact("fileUploadQueue",    CoreTarget::BigAndPrime),
    ThreadRule::wildcard("VDecod*",         CoreTarget::BigAndPrime),
    ThreadRule::exact("Lynx_JS",            CoreTarget::BigAndPrime),
    ThreadRule::exact("Socket Thread",      CoreTarget::BigAndPrime),
    ThreadRule::exact("RxCachedThreadS",    CoreTarget::BigAndPrime),
    ThreadRule::wildcard("IOThread*",       CoreTarget::BigAndPrime),
    ThreadRule::exact("AndroidUI",          CoreTarget::BigAndPrime),
    ThreadRule::exact("Gecko",              CoreTarget::BigAndPrime),
    ThreadRule::exact("Web Content",        CoreTarget::BigAndPrime),

    // ========================================================================
    // BIG：工作线程 / IO / 网络 / GPU辅助 / Binder / 解码
    // ========================================================================
    // GPU/图形驱动
    ThreadRule::wildcard("mali-*",          CoreTarget::Big),
    ThreadRule::wildcard("RenderThread*",   CoreTarget::Big),
    ThreadRule::wildcard("hwuiTask*",       CoreTarget::Big),
    ThreadRule::wildcard("hwui*",           CoreTarget::Big),
    ThreadRule::exact("Compositor",         CoreTarget::Big), // 修正：原判Prime，数据显示合成线程更偏Big
    ThreadRule::wildcard("UnityGfx*",       CoreTarget::Big), // 修正：原判BigAndPrime，92%纯度/138个App强证据
    ThreadRule::wildcard("VizCompositor",   CoreTarget::Big),
    // Unity 工作线程
    ThreadRule::wildcard("Job.[Ww]orker*",  CoreTarget::Big),
    ThreadRule::exact("UnityPreload",       CoreTarget::Big),
    ThreadRule::wildcard("UnityPreload*",   CoreTarget::Big),
    ThreadRule::wildcard("UnityMultiRende*",CoreTarget::Big),
    // Unreal 辅助
    ThreadRule::exact("RHIThread",          CoreTarget::Big),
    ThreadRule::wildcard("RHIThread*",      CoreTarget::Big),
    ThreadRule::wildcard("TaskGraph*",      CoreTarget::Big),
    ThreadRule::exact("ShaderCompiling",    CoreTarget::Big),
    ThreadRule::exact("PakMergeThread",     CoreTarget::Big),
    // Flutter
    ThreadRule::wildcard("DartWorker*",     CoreTarget::Big),
    ThreadRule::wildcard("?.io",            CoreTarget::Big),
    // Chromium
    ThreadRule::exact("Chrome_IOThread",    CoreTarget::Big),
    ThreadRule::exact("Chrome_ChildIOT",    CoreTarget::Big),
    ThreadRule::exact("NetworkService",     CoreTarget::Big),
    ThreadRule::exact("VizWebView",         CoreTarget::Big),
    ThreadRule::wildcard("ChromiumNet*",    CoreTarget::Big),
    ThreadRule::wildcard("IPC*",            CoreTarget::Big),
    ThreadRule::wildcard("IPDL*",           CoreTarget::Big),
    // React Native
    ThreadRule::low("*[Bb]ackStage*",  CoreTarget::Big),
    // 通用工作线程
    ThreadRule::wildcard("Thread-*",        CoreTarget::Big),
    ThreadRule::wildcard("NativeThread*",   CoreTarget::Big),
    ThreadRule::wildcard("MainThread*",     CoreTarget::Big), // 注意：精确"MainThread"已在Prime规则里，优先级更高
    ThreadRule::low("*worker*",        CoreTarget::Big),
    ThreadRule::low("*Worker*",        CoreTarget::Big),
    ThreadRule::exact("Worker Thread",      CoreTarget::Big),
    ThreadRule::wildcard("Task*",           CoreTarget::Big),
    ThreadRule::wildcard("JobThread*",      CoreTarget::Big),
    ThreadRule::wildcard("PoolThread*",     CoreTarget::Big),
    ThreadRule::exact("DispatchQueuePo",    CoreTarget::Big),
    ThreadRule::wildcard("io.worker*",      CoreTarget::Big),
    ThreadRule::exact("CoreThread",         CoreTarget::Big),
    ThreadRule::wildcard("Compute*",        CoreTarget::Big),
    ThreadRule::exact("Jit thread pool",    CoreTarget::Big),
    ThreadRule::exact("DefaultExecutor",    CoreTarget::Big),
    // Binder IPC
    ThreadRule::wildcard("Binder:*",        CoreTarget::Big),
    ThreadRule::wildcard("binder:*",        CoreTarget::Big),
    ThreadRule::wildcard("HwBinder:*",      CoreTarget::Big),
    ThreadRule::wildcard("[Bb]inder:*",     CoreTarget::Big),
    ThreadRule::low("*[Bb]inder*",     CoreTarget::Big),
    // 网络/IO
    ThreadRule::wildcard("OkHttp*",         CoreTarget::Big),
    ThreadRule::low("*[Hh]ttp*",       CoreTarget::Big),
    ThreadRule::wildcard("ExoPlayer:*",     CoreTarget::Big),
    ThreadRule::wildcard("beacon-thread-*", CoreTarget::Big),
    // 媒体解码
    ThreadRule::wildcard("MediaCodec*",     CoreTarget::Big),
    ThreadRule::wildcard("CodecLooper*",    CoreTarget::Big),
    ThreadRule::wildcard("storageQueue*",   CoreTarget::Big),
    ThreadRule::wildcard("mpv/*",           CoreTarget::Big),
    ThreadRule::wildcard("VOutle*",         CoreTarget::Big), // 截断的 VOutlet/VOutput（视频输出）
    ThreadRule::wildcard("AOutle*",         CoreTarget::Big), // 截断的 AOutlet/AOutput（音频输出，注意区别于纯解码走LITTLE）
    ThreadRule::wildcard("CRI*",            CoreTarget::Big), // CRIWARE 游戏中间件
    // 后台任务管理
    ThreadRule::exact("HeapTaskDaemon",     CoreTarget::Big),
    ThreadRule::exact("DefaultDispatch",    CoreTarget::Big),
    ThreadRule::low("*Event*",         CoreTarget::Big),
    ThreadRule::low("*TaskQueue*",     CoreTarget::Big),
    ThreadRule::wildcard("Shared_w*",       CoreTarget::Big),
    ThreadRule::exact("ThumbnailStorag",    CoreTarget::Big),
    ThreadRule::exact("JNISurfaceTextu",    CoreTarget::Big),
    ThreadRule::exact("MiniRenderThrea",    CoreTarget::Big),
    ThreadRule::exact("LogicThread",        CoreTarget::Big),
    ThreadRule::exact("GPM-DataThread",     CoreTarget::Big), // Google Play 游戏服务
    ThreadRule::exact("looper_monitor",     CoreTarget::Big),
    ThreadRule::exact("APM_light-weigh",    CoreTarget::Big), // 性能监控SDK（截断自 APM_light-weight）
    ThreadRule::exact("Phenix-Schedule",    CoreTarget::Big), // Phenix 直播流SDK
    ThreadRule::exact("alsoft-mixer",       CoreTarget::Big), // OpenAL Soft 音频混音线程

    // ========================================================================
    // LITTLE+BIG：通用线程池 / 音频混音解码 / 引擎后台加载（明确不占Prime）
    // ========================================================================
    // Android 系统线程池（修正：原判纯Little，数据显示实际跨小核+大核）
    ThreadRule::wildcard("ThreadPoolForeg*", CoreTarget::LittleAndBig),
    ThreadRule::wildcard("ThreadPoolServi*", CoreTarget::LittleAndBig),
    ThreadRule::wildcard("ThreadPool*",      CoreTarget::LittleAndBig),
    // Java 标准线程池（Executors.newFixedThreadPool）生成的命名格式：
    // pool-<pool_id>-thread-<thread_id>，是最常见的通用后台工作线程
    // 必须放在所有宽泛 *Pool* 规则之前，让精确前缀优先命中
    ThreadRule::wildcard("pool-[0-9]*-thread*", CoreTarget::LittleAndBig),
    ThreadRule::wildcard("#pool-[0-9]*-thread*", CoreTarget::LittleAndBig),
    // 常见业务侧线程池（对延迟不敏感，不需要超大核）
    ThreadRule::wildcard("onPool-worker-*",     CoreTarget::LittleAndBig),
    ThreadRule::wildcard("rx-pool-*",           CoreTarget::LittleAndBig),
    ThreadRule::wildcard("cached-pool-*",       CoreTarget::LittleAndBig),
    ThreadRule::wildcard("pivotal-pool-*",      CoreTarget::LittleAndBig),
    ThreadRule::wildcard("vrpool-*",            CoreTarget::LittleAndBig),
    ThreadRule::wildcard("ledThreadPool-*",     CoreTarget::LittleAndBig),
    ThreadRule::wildcard("*ColdPool*",          CoreTarget::LittleAndBig),
    ThreadRule::wildcard("*HotPool*",           CoreTarget::LittleAndBig),
    ThreadRule::wildcard("HadesPool*",          CoreTarget::LittleAndBig),
    ThreadRule::wildcard("HadesLibPool*",       CoreTarget::LittleAndBig),
    ThreadRule::wildcard("*ThreadPool*",        CoreTarget::LittleAndBig),
    // 音频（修正：原判纯Little）
    ThreadRule::wildcard("FMOD*",            CoreTarget::LittleAndBig),
    ThreadRule::exact("AudioTrack",          CoreTarget::LittleAndBig),
    ThreadRule::wildcard("SoundPool*",       CoreTarget::LittleAndBig), // 音频播放池，不需要超大核
    ThreadRule::wildcard("ijk_dash_pool_*",  CoreTarget::LittleAndBig), // ijkplayer dash流线程池
    // Unity 编曲/节拍（修正：原判纯Little）
    ThreadRule::wildcard("UnityChoreograp*", CoreTarget::LittleAndBig),
    ThreadRule::wildcard("UnityChoreo*",     CoreTarget::LittleAndBig),
    // 后台加载（修正：原判纯Little）
    ThreadRule::low("*Loading*",             CoreTarget::LittleAndBig),
    ThreadRule::wildcard("Loading.*",        CoreTarget::LittleAndBig),
    ThreadRule::wildcard("FAsyncLoading*",   CoreTarget::LittleAndBig),
    ThreadRule::wildcard("CriManaDecode*",   CoreTarget::LittleAndBig),
    ThreadRule::wildcard("DownloadThreadP*", CoreTarget::LittleAndBig),
    ThreadRule::exact("CallbackHndlr",       CoreTarget::LittleAndBig),
    ThreadRule::wildcard("IO*Thread*",       CoreTarget::LittleAndBig),

    // ========================================================================
    // LITTLE：纯音频解码/后台/GC辅助/日志（明确不需要大核）
    // ========================================================================
    ThreadRule::low("*[Aa]udio*",      CoreTarget::Little),
    ThreadRule::exact("AudioThread",        CoreTarget::Little),
    ThreadRule::wildcard("SoundDecoder*",   CoreTarget::Little),
    ThreadRule::wildcard("decodeQueue*",    CoreTarget::Little),
    ThreadRule::exact("AILocalThread",      CoreTarget::Little),
    ThreadRule::wildcard("Apollo-*",        CoreTarget::Little),
    ThreadRule::wildcard("GC?Marker??",     CoreTarget::Little),
    ThreadRule::wildcard("[Bb][Gg]*",       CoreTarget::Little),
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
        assert_eq!(classify_thread("HeapTaskDaemon"), CoreTarget::Big);
        assert_eq!(classify_thread("AudioThread"), CoreTarget::Little);
    }

    #[test]
    fn test_wildcard_match() {
        assert_eq!(classify_thread("mali-cmar-iface"), CoreTarget::Big);
        assert_eq!(classify_thread("Job.Worker1"), CoreTarget::Big);
        assert_eq!(classify_thread("Binder:1234_5"), CoreTarget::Big);
        assert_eq!(classify_thread("OkHttp ConnectionPool"), CoreTarget::Big);
    }

    #[test]
    fn test_little_and_big_category() {
        // 这些原先被旧版本归为纯 Little，根据数据挖掘修正为 LittleAndBig
        assert_eq!(classify_thread("FMOD stream"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("UnityChoreographer"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("AudioTrack"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("ThreadPoolForeground"), CoreTarget::LittleAndBig);
        // Java 标准线程池，不应该占用超大核
        assert_eq!(classify_thread("pool-3-thread-1"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("pool-128-thread-"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("#pool-5-thread-"), CoreTarget::LittleAndBig);
        // 音频播放池
        assert_eq!(classify_thread("SoundPool_1"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("SoundPool_2"), CoreTarget::LittleAndBig);
        // 美团 Hades 框架线程池
        assert_eq!(classify_thread("HadesPool#0"), CoreTarget::LittleAndBig);
        assert_eq!(classify_thread("HadesLibPool#1"), CoreTarget::LittleAndBig);
        // ijkplayer dash流
        assert_eq!(classify_thread("ijk_dash_pool_0"), CoreTarget::LittleAndBig);
    }

    #[test]
    fn test_corrected_categories() {
        // 数据挖掘修正：原内置规则把这些误判为 Big，实际更偏 BigAndPrime
        assert_eq!(classify_thread("glide-disk-cache-1"), CoreTarget::BigAndPrime);
        assert_eq!(classify_thread("AsyncTask #1"), CoreTarget::BigAndPrime);
        assert_eq!(classify_thread("mqt_js"), CoreTarget::BigAndPrime);
        // 数据挖掘修正：原内置规则把 UnityGfx* 误判为 BigAndPrime，实际更偏 Big
        assert_eq!(classify_thread("UnityGfxStats"), CoreTarget::Big);
    }

    #[test]
    fn test_unknown_thread() {
        assert_eq!(classify_thread("SomeRandomThread"), CoreTarget::Default);
    }

    #[test]
    fn test_priority_exact_over_wildcard() {
        // "MainThread" 精确匹配 -> Prime；"MainThread2" 只能命中通配符 -> Big
        assert_eq!(classify_thread("MainThread"), CoreTarget::Prime);
        assert_eq!(classify_thread("MainThread2"), CoreTarget::Big);
    }

    #[test]
    fn test_wildcard_fn() {
        assert!(thread_matches("Thread-*", "Thread-123"));
        assert!(thread_matches("mali-*", "mali-cmar-iface"));
        assert!(thread_matches("?.raster", "1.raster"));
        assert!(!thread_matches("Thread-*", "MainThread"));
    }
}
