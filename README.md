# AppOpt-RS

智能 Android 线程绑核优化，Rust 编写，**完全不依赖任何配置文件**。

## 这是什么

后台常驻一个小型守护进程，自动识别手机 CPU 拓扑（超大核/大核/小核），
持续扫描所有进程的线程，按线程名特征识别线程角色（渲染主线程、GPU辅助、
工作线程池、IO/网络、Binder、音频解码……），用 `sched_setaffinity` 把它们
绑到合适的核心上——重负载/延迟敏感的线程给大核或超大核，后台/能效任务
留给小核，避免系统调度器把关键线程"飘"到能效核上拖慢帧率，也避免无关
紧要的后台线程抢占性能核浪费电。

整个过程只看**线程名**，不依赖任何外部规则文件，装上即用。

## 规则表是怎么来的

线程名识别规则（`src/rules.rs`）不是凭空猜的，而是对一份覆盖 775 个 App、
6799 条真实"逐应用逐线程绑核配置"的社区数据集做了统计挖掘：把所有数据
按**线程名模式**聚合，统计每个模式在多少个不同 App 里出现、对应的核心
分配一致性有多高，只保留那些跨 App 数足够多、分类足够一致、能代表某种
**通用线程角色**（引擎内部线程名、系统组件、常见三方库）的模式收进内置
规则表；单个 App 专属、不可泛化的线程名一律不收——因为那是"认出了这个
App"，不是"认出了这类线程"，没法迁移到其他没见过的 App 上。

这样产出的规则表是**通用的**：换一个从没收录过的 App、换一台设备，
照样能靠线程名本身的特征工作，不需要任何配置文件兜底。

挖掘过程中也顺手发现并修正了原有经验规则里若干判断不准的地方，例如：

| 线程名模式 | 原判断 | 数据显示 | 跨App数/纯度 |
|---|---|---|---|
| `ThreadPoolForeg*` / `FMOD*` / `UnityChoreo*` / `AudioTrack` | 小核 | 小核+大核混合 | 68~108个App，90%+ |
| `*[Pp]ool*` / `glide*` / `AsyncTask*` / `mqt_*` / `JavaBridge` | 大核 | 大核+超大核 | 18~109个App，90%+ |
| `UnityGfx*` | 大核+超大核 | 大核 | 138个App，92% |
| `1.raster` / `?.raster`（Flutter光栅线程） | 超大核 | 大核+超大核 | 26个App，69~86% |
| `Compositor` | 超大核 | 大核 | 14个App，81% |

为了承载"小核+大核混合，但明确不占超大核"这种数据里大量出现、原来
4 分类模型没法表达的情况，新增了第 5 种绑核目标 `LittleAndBig`。

## 绑核决策优先级

```
1. custom.toml 用户手动覆盖（可选，按包名或全局线程名）
2. 内置启发式规则（rules.rs，零配置开箱即用）
3. 不干预（未识别的线程，维持系统默认调度）
```

## CPU 拓扑分类

以天玑9400+（MT6991：4小核+3大核+1超大核）为例：

- **Prime（超大核）** = core 7（Cortex-X925 @ 3730MHz）—— 唯一的主渲染/游戏主线程
- **Big（大核）** = core 4-6（Cortex-X4 @ 3300MHz）—— 工作线程池/IO/网络/Binder/解码
- **Big+Prime** —— 渲染/动画/线程池类辅助任务，吃性能但不该独占 Prime
- **Little+Big（小核+大核）** —— 通用线程池、音频混音解码、引擎后台加载
- **Little（小核）** = core 0-3（Cortex-A720 @ 2400MHz）—— 纯后台、日志、轻量音频流

CPU 拓扑在启动时自动读取 `/sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq`
按频率聚类得出，不限定具体核心数和编号，换设备也能正确识别。

## 安装

通过 Magisk / KernelSU / APatch 刷入 zip 包，重启生效，无需任何配置。

## 可选：手动覆盖（`custom.toml`）

绝大多数情况不需要这个文件。只有当自动识别结果不符合预期、或者你想给
某个特定 App 的特定线程强制指定核心时才需要。

放在模块目录下 `custom.toml`：

```toml
[settings]
scan_interval_ms = 2000
log_level = "info"   # debug 模式会在 appopt.log 里打印每个线程绑核来源(toml/builtin)及目标

[override]
# 按线程名覆盖，对所有 App 生效
"RenderThread" = "prime"

[override_app."com.example.game"]
# 仅针对特定包名覆盖
"GameMainThread" = "prime"
```

可选值：`prime` / `big` / `big+prime` / `little+big` / `little` / `default`（不干预）。

## 命令行参数

```
appopt [选项]
  -c <路径>   custom.toml 路径（默认: /data/adb/modules/AppOpt_RS/custom.toml）
  -s <毫秒>   扫描间隔毫秒数（默认: 2000）
  -d          Debug 模式（输出每个线程的绑核日志，含命中来源与目标分类）
  -t          仅输出 CPU 拓扑信息后退出
  -v          显示版本
  -h          显示帮助
```

## 编译

需要 Android NDK（r27+）和 Rust 工具链：

```bash
rustup target add aarch64-linux-android
# 配置 NDK linker（参考 .cargo/config.toml.example）
cargo build --release --target aarch64-linux-android
```

或直接推送 tag 触发 GitHub Actions 自动构建打包：

```bash
git tag v1.0.0
git push origin v1.0.0
```

## 项目结构

```
src/
├── main.rs       # 入口，参数解析，主循环
├── topo.rs       # CPU 拓扑自动识别
├── rules.rs      # 内置线程分类规则表（基于大规模数据统计挖掘，零配置）
├── glob.rs       # 通用 glob 匹配（* ? [abc] [0-9] [!abc]），供线程名匹配使用
├── scanner.rs    # 进程扫描、绑核决策优先级、亲和性设置
└── config.rs     # 用户手动覆盖配置（可选 custom.toml）

.github/workflows/
└── main.yml      # 自动编译 + 打包 Magisk 模块
```

## License

GPL-3.0
