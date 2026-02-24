# YanMiRestore（Rust）

一个以“源盘只读、安全优先”为核心的数据恢复工具，适用于：
- 电脑硬盘（HDD/SSD）
- 移动硬盘、U 盘等外接存储
- 手机导出的合法备份/镜像文件

## 当前已实现能力

项目目前包含 5 条可用恢复路径：
- 挂载目录下回收站/垃圾桶的逻辑删除文件恢复
- NTFS 删除 MFT 记录扫描（含数据运行列表解析）
- FAT12/16/32 与 exFAT 删除目录项扫描
- ext4 删除 inode 扫描（含 extent / 直连块映射）
- 原始镜像签名雕刻恢复

## 项目层级（标准化）

当前 `src` 目录按“入口 + 领域模块”划分，统一使用目录模块（`mod.rs`）：

```text
src/
├─ main.rs                 # 程序入口（参数为空时提示、日志初始化）
├─ app/mod.rs              # 命令分发与流程编排
├─ cli/mod.rs              # 命令行参数定义
├─ config/mod.rs           # 运行时配置加载（YanMiRestore.toml）
├─ error/mod.rs            # 统一错误类型
├─ model/
│  ├─ mod.rs               # 领域模型统一导出
│  ├─ enums.rs             # 枚举模型（目标类型/深度/文件系统提示）
│  ├─ request.rs           # 请求模型
│  ├─ plan.rs              # 方案模型
│  ├─ scan.rs              # 扫描报告模型
│  ├─ recovery.rs          # 恢复会话模型
│  ├─ fs.rs                # 文件系统扫描中间模型
│  └─ device.rs            # 设备列表模型
├─ scanner/mod.rs          # 扫描流程
├─ recovery/mod.rs         # 恢复流程
├─ report/mod.rs           # 控制台输出与报告写入
├─ utils/
│  ├─ mod.rs               # 工具模块入口
│  └─ label.rs             # 公共文案标签工具
├─ device/mod.rs           # 设备识别
├─ carver/mod.rs           # 签名雕刻
└─ fs/
   ├─ mod.rs               # 文件系统扫描聚合
   ├─ ntfs.rs
   ├─ fat.rs
   ├─ ext4.rs
   └─ partition.rs
```

## 命令说明

### 近期更新
- 进度条改为**动态刷新**（默认输出到 `stdout`），包含：
  - 动态旋转符（`| / - \`）
  - 已耗时、当前进度、百分比与阶段信息
- 恢复新增 `--skip-carved`，可仅恢复文件系统候选项，避免优先导出可能碎片化损坏的雕刻文件。
- 新增 `YanMiRestore.toml` 配置文件，可统一管理扫描阈值、恢复缓冲区、进度条刷新频率等参数。

### 1. `design`（生成恢复方案）
- 根据 `case-id`、目标类型、扫描深度、文件系统提示生成执行计划
- 输出阶段步骤、安全规则、前置假设

### 2. `scan`（只读扫描）
- 自动识别输入是挂载目录还是镜像文件
- 当 `--target-kind auto` 且在交互终端运行时，会先显示自动识别结果，并要求键入 `y/yes` 确认后继续
- 可通过 `--yes` 跳过键入确认（适合脚本/批处理）
- 逻辑扫描目录：
  - `$Recycle.Bin`、`RECYCLER`、`Recycler`、`RECYCLED`
  - `.Trash`、`.Trashes`
- 镜像扫描能力：
  - NTFS：扫描删除 MFT 项，解析 `$DATA` 常驻/非常驻内容
  - FAT/exFAT：扫描删除目录项，尝试按链式或连续簇重建文件段
  - ext4：扫描 `i_dtime != 0` 的删除 inode，提取可恢复段
- 分区识别：
  - 支持 MBR/GPT
  - 支持扩展分区/逻辑分区链
  - 可按分区偏移探测 NTFS/FAT/ext4
- 识别但暂不恢复元数据：
  - APFS 签名探测
  - F2FS 签名探测
- 可选签名雕刻（`--include-carving`）：
  - `jpg`、`png`、`pdf`、`sqlite`、`docx`、`xlsx`、`mp4`
- 生成 JSON 扫描报告，包含候选项与恢复坐标
- CLI 会打印扫描摘要与文件系统统计信息
- CLI 会显示动态阶段进度条（设备信息、文件系统扫描、签名特征扫描、结果汇总）

### 3. `recover`（恢复输出）
- 默认 `dry-run`：仅生成计划动作，不写文件
- 指定 `--execute`：执行实际导出
  - 逻辑项：从回收站路径复制
  - NTFS/ext4 项：按 `source_segments` 分段提取
  - 雕刻项：按 `offset + size` 从镜像提取
- 默认优先使用原文件名恢复（重名时自动追加 `_1`、`_2`...）
- 默认在可获取时保留源文件访问/修改时间戳
- 可通过 `--no-keep-original-name` 改为旧版“条目前缀命名”策略
- 可通过 `--no-preserve-timestamps` 关闭时间戳同步
- 可通过 `--skip-carved` 跳过签名雕刻候选项（避免恢复出部分损坏文件）
- 生成 `恢复清单.json` 清单
- CLI 会显示按条目推进的动态恢复进度条，并实时显示当前处理项
- 在不支持 TTY 的日志环境中，进度条可能退化为普通文本输出

### 4. 配置文件（新增）
- 程序启动时会优先尝试读取当前目录下的 `YanMiRestore.toml`
- 也可以通过全局参数 `--config <路径>` 指定配置文件
- 配置采用 TOML 格式，支持：
  - `ui`：进度条刷新频率、动画间隔、模板、字符、旋转帧与阶段前缀
  - `scan`：逻辑扫描上限与回收站目录名
  - `carver`：签名雕刻读取范围、单签名条目上限、各格式大小阈值与 ZIP/MP4 搜索窗口
  - `fs.ntfs` / `fs.fat` / `fs.ext4`：文件系统扫描阈值
  - `partition`：分区解析上限
  - `recovery`：恢复缓冲区与原始卷对齐参数

## 结构化统计指标

扫描报告中 `fs_result.metrics` 包含：
- `ntfs`：可恢复/仅元数据/压缩/加密/运行列表失败等统计
- `fat`：卷类型数量、删除文件/目录、分段恢复与仅元数据统计
- `ext4`：卷数、删除文件/目录、稀疏段、深度不支持、旧指针统计

## 安全约束

- 工具不会写入源介质
- 恢复输出必须写入目标路径
- `recover` 会阻止以下危险目标路径：
  - 目标目录位于源路径内
  - 目标目录是源目录父路径
  - 目标路径与源路径相同
  - Windows 下与源路径同卷（同盘符前缀）
- 保留报告与清单，便于审计追踪

### 常见安全报错说明

如果你看到类似报错：

```text
Error: 目标路径不安全：源路径（\\.\F:）与目标路径（F:\restore）位于同一 Windows 卷
```

表示你把恢复输出目录设置在了源盘 `F:` 上。为避免覆盖待恢复数据，程序会强制拦截。

请把 `--destination` 改到其他盘符，例如：

```powershell
.\YanMiRestore.exe recover --report E:\output\CASE-F-001-scan-report.json --destination G:\restore --execute
```

## 使用示例

```bash
# 方式 A：直接使用 cargo 运行

# 1) 生成恢复方案（深度扫描 + 启用雕刻）
cargo run -- design --case-id CASE-1001 --target-kind pc-disk --depth deep --include-carving

# 2) 扫描 PC 磁盘镜像（自动识别文件系统）
cargo run -- scan --source E:\evidence\disk.img --output .\output --case-id CASE-1001 --target-kind pc-disk --depth deep --include-carving

# 3) 扫描 Linux/ext4 镜像（强制 ext4）
cargo run -- scan --source E:\evidence\linux.img --output .\output --case-id CASE-EXT4-001 --fs-hint ext4 --depth deep

# 4) 扫描手机导出目录（逻辑恢复路径）
cargo run -- scan --source E:\evidence\phone-backup --output .\output --case-id CASE-PHONE-001 --target-kind phone --depth metadata

# 5) 恢复预演（不写文件，只生成动作计划）
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore

# 6) 执行实际恢复
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore --execute

# 7) 执行恢复并关闭“原文件名+时间戳”策略（兼容旧命名方式）
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore --execute --no-keep-original-name --no-preserve-timestamps

# 8) 只恢复文件系统候选项（跳过雕刻结果）
cargo run -- recover --report .\output\CASE-1001-scan-report.json --destination G:\restore --execute --skip-carved

# 9) 自动识别模式下跳过键入确认
cargo run -- scan --source F:\ --target-kind auto --depth deep --include-carving --output E:\output --case-id CASE-F-001 --yes

# 10) 使用指定配置文件执行扫描
cargo run -- --config .\YanMiRestore.toml scan --source E:\evidence\disk.img --output .\output --case-id CASE-CFG-001 --depth deep
```

```powershell
# 方式 B：打包后使用 exe（推荐给最终用户）

# 1) 构建 release 版本
cargo build --release

# 2) 进入可执行文件目录
cd .\target\release

# 3) 查看帮助
.\YanMiRestore.exe --help

# 4) 扫描镜像（启用雕刻）
.\YanMiRestore.exe scan --source E:\evidence\disk.img --output E:\output --case-id CASE-2001 --depth deep --include-carving

# 5) 执行恢复
.\YanMiRestore.exe recover --report E:\output\CASE-2001-scan-report.json --destination G:\restore --execute

# 6) 指定配置文件
.\YanMiRestore.exe --config E:\tools\YanMiRestore.toml scan --source E:\evidence\disk.img --output E:\output --case-id CASE-2001 --depth deep
```

```text
# 常见输出文件

output\CASE-xxxx-scan-report.json     # 扫描报告（输入给 recover）
restore\恢复清单.json                 # 恢复执行结果清单
```

## Windows 双击说明

- `YanMiRestore.exe` 是命令行程序，不是图形界面程序。
- 直接双击时如果未传参数，程序会显示帮助并等待你按回车退出，不再“一闪而过”。
- 推荐在 `cmd` 或 `PowerShell` 中按上面的示例命令运行。

## 已知限制

- NTFS 压缩/加密流的解码尚未实现（当前会标注为仅元数据）
- 当前版本的 FAT/exFAT 与 ext4 删除项解析处于稳态占位实现，主要用于识别卷类型并输出统一报告结构
- FAT/exFAT 在碎片化严重场景下仍可能仅能给出部分恢复段
- exFAT `NoFatChain` 场景仍依赖连续簇假设
- ext4 当前以 inode 为粒度，未做完整删除文件名重建
- ext4 旧式多级间接块（间接/双间接/三间接）尚未完整解析
- APFS/F2FS 暂为签名识别，未实现删除元数据解析
- 签名雕刻属于启发式方法，可能误报或受碎片影响
- 手机场景默认基于合法导出数据，不包含绕过加密/锁的能力

## 合规说明

仅可在已获得授权的设备和数据上使用本工具。

## 开源协作

仓库已提供基础开源治理文件：

- 许可证：`LICENSE`
- 贡献指南：`CONTRIBUTING.md`
- 行为准则：`CODE_OF_CONDUCT.md`
- 安全策略：`SECURITY.md`
- 模板与自动化：`.github/`

## 自动识别设备（新增）

已新增 `devices` 子命令，用于自动识别本机可用设备，并标注为：
- 电脑硬盘
- 移动硬盘
- 手机
- 其他设备

### 快速使用

```powershell
# 1) 先列出设备（推荐先执行）
.\YanMiRestore.exe devices

# 2) 复制上一步输出的路径作为 source 扫描
.\YanMiRestore.exe scan --source E:\ --target-kind auto --output .\output --case-id CASE-AUTO-001
```

### 说明

- `--target-kind auto` 时，程序会根据路径与系统盘符类型自动判断设备类型。
- 在 Windows 下，会优先使用系统盘符类型（可移动/固定/网络/光驱等）识别。
- 扫描摘要会显示“识别设备类型”和“设备识别依据”。

## F盘恢复注意事项（重要）

如果源是盘符根目录（如 `F:\`），程序会自动尝试切换到底层卷路径（如 `\\.\F:`）进行扫描。

请确保：
- 以**管理员权限**运行命令行，否则无法访问底层卷。
- 恢复输出目录必须在**其他盘符**（例如源是 `F:`，输出用 `E:` 或 `G:`）。

示例：

```powershell
# 管理员 PowerShell
.\YanMiRestore.exe scan --source F:\ --target-kind auto --depth deep --include-carving --output E:\output --case-id CASE-F-001

.\YanMiRestore.exe recover --report E:\output\CASE-F-001-scan-report.json --destination G:\restore --execute
```

如果你希望自动识别后不再手动键入确认，可在扫描命令追加：

```powershell
.\YanMiRestore.exe scan --source F:\ --target-kind auto --depth deep --include-carving --output E:\output --case-id CASE-F-001 --yes
```

若扫描后仍为 0 候选，通常是：
- 该盘近期删除数据已被覆盖；
- SSD/U盘控制器已执行 TRIM/垃圾回收；
- 删除时间较久，元数据已复用。

#   Y a n M i R e s t o r e 
 
 

