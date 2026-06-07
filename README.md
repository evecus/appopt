# AppOpt-RS

智能 Android 线程绑核优化，Rust 编写，无需配置文件。

## 工作原理

```
启动
 ↓
读取 /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq
 ↓
自动识别 Prime / Big / Little 核心布局
 ↓
循环扫描 /proc，读取所有进程线程名
 ↓
内置规则匹配（Unity/Unreal/Flutter/Chromium/原生）
 ↓
sched_setaffinity 绑核
```

## 核心分配策略

| 线程类型 | 目标核心 | 示例 |
|---------|---------|------|
| 主渲染/游戏主线程 | Prime（超大核） | RenderThread, UnityMain, GameThread |
| 渲染辅助/GPU | Big+Prime | UnityGfx*, GLThread* |
| 工作线程/网络/IO | Big（大核） | mali-*, Job.Worker*, Binder:*, OkHttp* |
| 音频/后台/GC | Little（能效核） | FMOD*, *Audio*, UnityChoreograp* |

天玑9400+（MT6991）对应：
- Prime = core 7（Cortex-X925 @ 3730MHz）
- Big = core 4-6（Cortex-X4 @ 3300MHz）
- Little = core 0-3（Cortex-A720 @ 2400MHz）

## 安装

通过 Magisk / KernelSU / APatch 刷入 zip 包，重启生效。

## 可选配置

模块目录下放置 `custom.toml`（参考 `custom.toml.example`），
可覆盖特定线程的绑核策略。不放则完全使用内置规则。

## 编译

需要 Android NDK（r27+）和 Rust 工具链：

```bash
# 添加编译目标
rustup target add aarch64-linux-android

# 配置 NDK linker（参考 .cargo/config.toml.example）

# 编译
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
├── main.rs      # 入口，参数解析，主循环
├── topo.rs      # CPU 拓扑自动识别
├── rules.rs     # 线程分类规则表（80+ 条）
├── scanner.rs   # 进程扫描与亲和性设置
└── config.rs    # 用户覆盖配置（可选 TOML）

.github/workflows/
└── build.yml    # 自动编译 + 打包 Magisk 模块

magisk/          # 模块模板文件
```

## License

GPL-3.0
