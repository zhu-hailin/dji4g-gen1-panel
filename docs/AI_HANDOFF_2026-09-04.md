# DJI 一代 4G 面板：续作与最终验收交接

更新时间：2026-09-04（Asia/Shanghai）  
当前结论：**尚未到达最终验收门槛**。任务因当前 AI 额度即将耗尽，按用户要求主动停止并移交。请先完成“生产运行时接线”，再制作发布候选，最后执行本文的验收矩阵。

## 1. 用户目标与不可变范围

- 只支持 Windows，只支持大疆一代 4G 模块：`USB\\VID_2CA3&PID_4006`。
- 小窗口、简体中文、界面朴素专业；必须永久显示“是否可用”及具体原因。
- 支持托盘、单实例、可选开机自启。
- 必须基于目标模块的精确设备身份和精确网卡做检测；不得把手机热点、普通 Wi-Fi、Meta/Clash TUN 或系统默认路由误判成模块联网。
- 支持热点读取与开关，以及受控修复：DHCP、DNS、精确网卡重启、设备重新枚举、USB 网络模式 0/1、已有且非活动的 IP PDP 上下文 APN、模块重启。
- 禁止“任意 AT 命令”、任意 shell、模糊 FriendlyName 匹配、其他 PID、驱动或固件修改。
- 所有写操作必须：重新枚举目标 -> 校验 epoch/身份/操作前状态 -> 用户确认 -> 只执行一次 -> 新鲜回读 -> `Applied / Failed / OutcomeUnknown` 三态结果。不能自动重试不可重复操作。
- 不得推送 GitHub、签名或发布，除非用户后续明确授权。

## 2. 仓库与当前 Git 状态

- 主仓库：`C:\Users\22050\Desktop\dji4g-panel`
- 当前工作树：`C:\Users\22050\Desktop\dji4g-panel\.worktrees\dji4g-gen1-panel`
- 分支：`feature/dji4g-gen1-panel`
- 当前 HEAD：`4d860fa feat: add controlled repair workflows`
- 设计：`docs/superpowers/specs/2026-09-03-dji4g-gen1-panel-design.md`
- 实施计划：`docs/superpowers/plans/2026-09-03-dji4g-gen1-panel-implementation.md`
- SDD 过程资料（被忽略，不应强行纳入 Git）：`.superpowers/sdd/2026-09-03-dji4g-gen1-panel-implementation/`

当前工作树有**正在进行且未提交**的生产运行时改动，必须保留，不得 reset/clean/stash：

```text
 M apps/panel/src/lib.rs
 M apps/panel/src/main.rs
 M crates/application/src/confirmation.rs
 M crates/application/src/ports.rs
?? apps/panel/src/runtime.rs
?? apps/panel/tests/production_runtime.rs
```

先执行以下只读盘点，再继续：

```powershell
git status --short
git diff -- apps/panel/src/lib.rs apps/panel/src/main.rs crates/application/src/confirmation.rs crates/application/src/ports.rs
git diff --no-index -- NUL apps/panel/src/runtime.rs
git diff --no-index -- NUL apps/panel/tests/production_runtime.rs
```

## 3. 已完成并提交的功能

按提交顺序，以下功能已有实现及自动化测试：

1. 基础域模型、配置、隐私日志、精确 PnP 识别。
2. 安全 AT 协议和串口 actor，含拔插/取消语义。
3. 精确 Windows 网卡解析与绑定式联网探测。
4. 应用 reducer/controller，诊断状态与受控动作编排。
5. 紧凑中文窗口、中文字体回退、长文本布局。
6. 托盘、单实例、可选开机自启。
7. WinRT 移动热点后端与初步 MSIX 打包。
8. 已认证的提权 helper IPC。
9. Task 11 全部受控修复后端。

关键提交：

```text
4d860fa feat: add controlled repair workflows
319c628 feat: add authenticated elevated helper IPC
52d1857 feat: add Windows hotspot control and MSIX packaging
6cdcbdd feat: add Windows tray and optional autostart
9a508a0 docs: capture Chinese panel visual acceptance
b6b8486 fix: wrap long Chinese panel text
dbfcd36 feat: add compact Chinese desktop panel
aeeaad6 feat: orchestrate monitoring and safe actions
f27bbc7 fix: harden adapter-bound connectivity probes
1bff5b3 fix: pair serial cancellation with worker thread
```

Task 11 的独立验证曾通过：Windows platform `112` 个测试；实现代理报告 workspace fmt/check/test/clippy/release check 全通过。**这些结果只覆盖 HEAD `4d860fa`，不覆盖当前未提交的运行时改动。**未执行真实 UAC 或会改变硬件状态的 HIL。

## 4. 当前未完成点：生产运行时接线（最高优先级）

原程序的 `apps/panel/src/main.rs` 使用 `Controller::for_test`。当前未提交改动已经改为 `ProductionComposition`，并为普通无 `--demo` 路径连接 `MonitorPorts`；方向正确，但中断时仍未编译完成，也仍有安全占位。

### 4.1 当前已知编译错误

最后一次命令：

```powershell
C:\Users\22050\.cargo\bin\cargo.exe check -p dji4g-panel --all-targets --offline
```

错误摘要：

- `apps/panel/src/runtime.rs` 导入 `dji4g_at_protocol`，但 `apps/panel/Cargo.toml` 尚未添加该 workspace dependency。
- `TargetContext::new` 仍为 `pub(crate)`，组合层不可调用；需要最小安全构造方式，不能开放任意不受支持设备。
- `same_guid`、`probe_dto` 尚未定义。
- `AtControlAvailability` 未导入。
- 另有若干未使用导入。

请先修复编译，再运行测试；不要删除现有未提交文件。

### 4.2 不能以占位实现结束

中断时 `apps/panel/src/runtime.rs` 仍存在以下问题，必须全部消除：

- `ProductionAt` 已开始使用 `AtSessionActor::open_selected` 和只读 AT 查询，但需验证完整语义、隐私、错误映射和 epoch 失效；`invalidate` 仍是空实现。如果设计为每轮新 actor，需要明确证明没有旧会话可复用，并加测试。
- `ProductionProbe::observe` 已开始调用 `WindowsNetworkProbe::observe_now`，但参数 `active` 仍被忽略。必须让“主动探测关闭”产生明确的 `Unexecuted` 状态，并让开启时只对 resolver 返回的精确 `AdapterIdentity` 做绑定探测。
- `probe_dto` 必须完整映射 gateway/public/DNS、IPv4/IPv6 coverage、全局默认路由解释信息；全局路由只能解释，不能作为模块可用证据。
- `ProductionHotspot::revalidate_toggle` 中仍返回 `BeforeStateHash([0; 32])`。这是禁止的伪证明。必须基于新鲜 exact source profile、capability、status 生成与执行侧一致的哈希；执行前再次核验。
- `ProductionActionExecutor` 与 `PrivilegedExecutor` 仍固定返回 `privilege:helper_untrusted`。必须使用 `TrustedHelper::installed()` 和 `launch_elevated_helper(...)`，把 `ValidatedActionToken` 映射成封闭的 `HelperActionV1`，完整传递 epoch、身份哈希和 before-state hash，并把 helper 三态结果映射回 `ExecutionReceipt`。
- 开发 checkout 找不到“已安装且签名可信的 helper”时应明确 fail closed；正式 MSIX 安装路径必须能找到真实 helper。不要为了让测试通过放宽签名或路径校验。
- `apps/panel/tests/production_runtime.rs` 目前只验证类型存在及源码中没有 `Controller::for_test`，不足以证明委托行为。请增加端口委托/映射/失败关闭/普通启动路径测试。
- 检查 `ControllerRunner` 是否忙等；若仍用 `try_recv + thread::yield_now` 持续空转，应改为有界等待或事件驱动，避免常驻托盘时占 CPU。

可复用后端：

- AT：`crates/windows-platform/src/serial.rs` 的 `AtSessionActor::open_selected`、`safe_handshake`、`execute`。
- 精确网卡：`WindowsAdapterResolver`。
- 绑定探测：`WindowsNetworkProbe::observe_now`。
- 热点：`WindowsHotspotControl::{capability,status,set_enabled}`。
- 提权：`TrustedHelper::installed`、`launch_elevated_helper`、Task 10 IPC 类型。
- 修复：`WindowsNativeRepairBackend` / `WindowsRepairExecutor`，helper 端已经执行新鲜 prepare、epoch/identity/before-hash 比较及一次性执行。

## 5. 完成生产接线后的自动化门禁

在工作树中运行；任何失败都不能写成“通过”：

```powershell
cargo fmt --all -- --check
cargo test --workspace --all-targets --offline
cargo check --workspace --all-targets --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo check --workspace --release --all-targets --offline
git diff --check
```

还需执行针对性静态检查并人工阅读命中项：

```powershell
rg -n "Controller::for_test|FakeActionExecutor|dependency_unavailable|session_unavailable|\[0; 32\]|TODO|FIXME|unimplemented!|todo!" apps crates
rg -n "FriendlyName|powershell|cmd\.exe|Command::new|raw AT|AT\+" apps crates
rg -n "VID_2CA3|PID_4006|2CA3|4006" apps crates
```

生产接线通过后单独提交，建议提交信息：

```text
feat: wire production Windows runtime
```

## 6. 发布候选与文档任务（尚未开始）

生产接线提交后再做：

- `deny.toml`
- `.github/workflows/ci.yml`
- `packaging/scripts/build-msix.ps1`
- `packaging/scripts/verify-release.ps1`
- `README.md`（简体中文优先，写明只支持一代模块）
- `SECURITY.md`
- `docs/architecture.md`
- `tests/hardware/README.md`
- `tests/hardware/report-template.md`
- 最终更新本文件或另建 `docs/FINAL_ACCEPTANCE_HANDOFF.md`

CI/发布门禁至少包括：fmt、clippy `-D warnings`、workspace 全测试、release build、依赖许可/漏洞检查（工具缺失写 `Environment-blocked`，不要伪造通过）、SBOM/许可证清单、文件哈希、MSIX 内容验证。

只允许构建**未签名开发候选**；不得生成/安装证书，不得声称已签名或可正式发布。建议产物放 `dist/`，并记录每个文件 SHA-256。若复制到桌面，保留 repo 内同源产物和哈希。

## 7. 最终验收：由接手 AI 执行

结果只能使用四种状态：`Passed`、`Failed`、`Environment-blocked`、`Unexecuted`。自动化测试不能替代人工或硬件验收。

### 7.1 静态与自动化

- 普通 debug/release 无 `--demo` 路径绝不构造测试 controller/fake port。
- 所有生产端口均调用真实 Windows 后端，无固定“不可用”占位。
- 一代 PID 白名单、无 FriendlyName 回退、无任意 shell/任意 AT。
- 错误码稳定且中文可解释；日志不泄露 APN、设备标识、串口内容、nonce、pipe 名。
- 全部第 5 节命令通过。
- release verifier 能发现缺文件、错误哈希、错误 package identity、意外调试二进制。

### 7.2 UI/生命周期人工验收

- 100%/125%/150%/200% DPI，小窗口无截断；中文无方框字。
- “是否可用”永久可见；未知/过期状态不能显示绿色可用。
- 托盘显示/隐藏/退出；关闭窗口只隐藏；资源管理器重启后托盘恢复。
- 单实例：重复启动聚焦现有窗口且不丢刷新意图。
- 开机自启开/关及 drift 显示；卸载后无残留启动项。
- 睡眠/唤醒、锁屏/解锁、快速拔插均不崩溃、不高 CPU、不展示旧设备数据。

### 7.3 UAC/helper 验收

- 未安装/未签名/路径不可信 helper：明确失败关闭。
- 用户取消 UAC：状态明确，无动作、无自动重试。
- 用户允许 UAC：只接受当前用户、当前 nonce、当前 pipe、协议版本和签名可信进程。
- 重放、过期请求、身份/epoch/before hash 改变均被拒绝。
- helper 每次只执行一个封闭动作并退出，结果三态正确。

### 7.4 PID4006 真实硬件 HIL

以下项目会读取或改变真实设备/网络状态，必须先得到用户针对维护窗口和测试 SIM/设备的明确授权：

- 直插、扩展坞、换 USB 口；移除/重插；COM 号变化；设备重新枚举。
- 绿灯状态下识别、SIM 缺失/PIN/拒绝/正常、注册搜索/拒绝/本地/漫游、附着与 PDP。
- DHCP 正常、APIPA、无网关、DNS 超时；IPv4/IPv6 分别记录。
- 手机热点开启/关闭、Wi-Fi、Meta/Clash TUN 与模块同时存在；验证只有 adapter-bound 证据能让模块显示可用。
- 公网绑定成功但模块 DNS 失败时，应显示“数据链路可达、DNS 失败”，不得笼统写驱动故障。
- Windows 移动热点：精确以模块网卡为 source，开启、关闭、客户端接入、客户端计数、失败回读。
- 每个受控修复逐项验证：成功、失败、拔插竞态、UAC 取消、结果未知；不可重复动作不能自动重试。
- 睡眠/唤醒后重新枚举和新鲜探测。

每一项保存：时间、commit、Windows 版本、安装方式、模块硬件 ID、驱动版本、连接方式、Clash/Meta/TUN 状态、步骤、预期、实际、截图/日志位置、四态结论。敏感值必须脱敏。

## 8. 当前环境遗留事项

- 历史 Task 9 测试进程 `hotspot_spike.exe` PID `16152` 在 2026-09-04 仍可见，过去终止时曾遇到 Access Denied。未调用任何热点状态变更。接手 AI 先只读确认其路径、父进程和状态；不要反复强杀，也不要把它误当正式应用进程。若需终止，先向用户说明并获得相应权限。
- 真实设备/UAC/HIL 尚未执行，全部应记为 `Unexecuted`，不是 `Passed`。
- 当前未提交运行时改动处于编译失败的中间态；这是移交时最重要的事实。

## 9. 停止与发布判定

出现以下任一情况必须停止发布并记录 `Failed`：

- 普通启动仍使用 fake/demo 或生产端口固定返回占位错误。
- 能匹配非 `VID_2CA3&PID_4006` 设备，或使用 FriendlyName/默认路由兜底。
- 热点、修复动作缺少 fresh identity/epoch/before-state 校验。
- 用户取消 UAC 后仍执行，或 outcome unknown 被自动重试。
- 手机热点/TUN 的公网结果被记为大疆模块可用。
- 日志/界面泄露敏感标识，或 package/helper 信任边界被放宽。
- 任一必需自动化、UI、安装、UAC 或 HIL 项为 `Failed`。

只有在必需项全部 `Passed`，且剩余 `Environment-blocked/Unexecuted` 均被用户明确接受时，才可称为“最终验收通过”。在此之前只能称“发布候选”。

