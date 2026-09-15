# DJI 一代 4G 面板：最终验收交接（FINAL ACCEPTANCE HANDOFF）

更新时间：2026-09-04（Asia/Shanghai）
执行者：接手 AI（依据 `docs/AI_HANDOFF_2026-09-04.md`）
当前 HEAD：`b9b2a7e docs: add README, security policy, architecture, and HIL templates`
分支：`feature/dji4g-gen1-panel`（工作树 `.worktrees/dji4g-gen1-panel`）

## 0. 总体结论

**当前状态：发布候选（release candidate），尚未到达“最终验收通过”。**

依据交接文档第 9 节：只有在必需项全部 `Passed`，且剩余 `Environment-blocked`/`Unexecuted`
均被用户明确接受时，才可称为“最终验收通过”。本次：

- 第 4 节（生产运行时接线）与第 5 节（自动化门禁）：**Passed**，已提交。
- 第 6 节（发布工程与文档）：**Passed**（产物已构建并验证），已提交。
- 第 7.1 节（静态与自动化验收）：**Passed**（逐项见下）。
- 第 7.2 / 7.3 / 7.4 节（人工 UI/生命周期、UAC/helper、真机 HIL）：**Unexecuted**——
  需要真实硬件、真实 UAC 与用户对维护窗口/测试 SIM 的明确授权，本环境无法执行，也不得伪造通过。
- 依赖许可/漏洞检查（cargo-deny）与 SBOM（cargo-cyclonedx）：本机 **Environment-blocked**
  （工具未安装）；CI 工作流已把它们接为硬门禁/尽力门禁。

因此：**不存在任何 `Failed` 必需项**，但存在用户必须显式接受的 `Unexecuted`/`Environment-blocked`
项。在用户接受或亲自执行真机验收之前，只能称“发布候选”。

本次会话产生的提交：

```text
b9b2a7e docs: add README, security policy, architecture, and HIL templates
20de544 chore: add release engineering for unsigned MSIX candidates
6a8639b refactor(panel): tidy layout with full-width sections and aligned grids
1faf3ab feat: wire production Windows runtime   ← 第 4/5 节（上一阶段提交）
```

## 1. 第 4 节：生产运行时接线 —— Passed

交接时该部分处于**编译失败的中间态**，现已修复、消除占位并提交（`1faf3ab`）。

| 交接要求 | 结果 | 证据 |
| --- | --- | --- |
| 修复 `runtime.rs` 全部编译错误（缺依赖、`TargetContext::new` 私有、`same_guid`/`probe_dto` 未定义、`AtControlAvailability` 未导入、未用导入） | Passed | `cargo check --workspace --all-targets` 退出 0 |
| `ProductionAt::invalidate` 不得为伪空实现 | Passed | 每轮观测新建并丢弃 `AtSessionActor`，无缓存会话；`runtime.rs` 测试 `at_port_invalidate_is_a_stateless_noop` |
| `ProductionProbe::observe` 不得忽略 `active` | Passed | active=false → 全部 `Unexecuted`（`probe:disabled_by_setting`），无网络 I/O；active=true → 仅对 resolver 返回的精确 `AdapterIdentity` 绑定探测，GUID 不符即 `probe:route_identity_mismatch` 失败关闭 |
| `probe_dto` 完整映射 gateway/public/DNS、IPv4/IPv6 coverage、全局默认路由（仅解释） | Passed | `SystemRouteDto { explanation_only: true }`；测试 `probe_dto_maps_single_family_success_and_explanation_only_route` |
| `revalidate_toggle` 不得返回 `BeforeStateHash([0; 32])` | Passed | 经 `WindowsRepairExecutor` 新鲜观测生成真实 before-state 哈希；全仓 `rg "\[0; 32\]"` 无命中 |
| `ProductionActionExecutor`/`PrivilegedExecutor` 不得固定返回 `privilege:helper_untrusted`，须走 `TrustedHelper::installed()` + `launch_elevated_helper(...)` 并映射三态 | Passed（实现）/ 见第 5 节已知限制 | `execute_via_helper`：重扫精确目标 → 封闭映射 → 本地新鲜 prepare 算权威哈希 → `TrustedHelper::installed()` → `build_helper_request` → `launch_elevated_helper` → `map_helper_response` |
| 开发 checkout 找不到可信 helper 时明确 fail closed；不得放宽签名/路径校验 | Passed | 测试 `trusted_helper_fails_closed_outside_a_signed_install`；`TrustedHelper::installed()` 未放宽 |
| `production_runtime.rs` 增加端口委托/映射/失败关闭/普通启动路径测试 | Passed | 该集成测试断言 main.rs 无 `Controller::for_test`/`FakeActionExecutor`、runtime.rs 无占位、`TrustedHelper::installed().is_err()` |
| `ControllerRunner` 不得忙等占 CPU | Passed | `monitor.rs::run` 改为 `recv_timeout(IDLE_POLL_INTERVAL=250ms)` 有界等待，移除 `try_recv + yield_now` 自旋；空闲时线程阻塞，刷新延迟 ≤ 一个间隔 |

## 2. 第 5 节：自动化门禁 —— Passed

在工作树中实际运行，退出码均为 0（任何失败都不会被写成“通过”）：

| 命令 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | Passed（退出 0） |
| `cargo test --workspace --offline` | Passed（退出 0；含 application 13 项受控修复测试、panel 20 项 bin 测试 + 3 + 6 + 5、localization_complete 3 项、windows-platform 7 项 doc-test 等） |
| `cargo check --workspace --all-targets --offline` | Passed（退出 0） |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | Passed（退出 0） |
| `cargo check --workspace --release --all-targets --offline` | Passed（退出 0） |
| `git diff --check` | Passed（退出 0；仅 LF→CRLF 提示，非空白错误） |

针对性静态检查（人工阅读命中项）：

- `rg "Controller::for_test|FakeActionExecutor|dependency_unavailable|session_unavailable|\[0; 32\]|TODO|FIXME|unimplemented!|todo!" apps crates`
  → 生产路径**无命中**。`for_test`/`FakeActionExecutor` 仅存在于 `crates/application` 的测试支撑
  （`controller.rs`/`ports.rs`/`lib.rs` 再导出）、`crates/application/tests/*` 集成测试，以及
  `apps/panel/tests/production_runtime.rs` 中“断言其不存在”的守卫。
- `rg "FriendlyName|powershell|cmd\.exe|Command::new|raw AT" apps crates`
  → 仅 3 处命中：`runtime.rs`/`repair.rs` 中“绝不按 FriendlyName / 绝不接受 raw AT”的**保证性注释**，
  以及 `apps/panel/src/bin/hotspot_spike.rs` 中开发期 spike 用 `Command::new(current_exe())`
  **自我重启**（supervisor/worker 模式，非任意 shell；spike 不打入 MSIX，verify-release 的
  `hotspot_spike*` 禁用模式会拦截）。
- `rg "VID_2CA3|PID_4006|2CA3|4006" apps crates`
  → 白名单为单一封闭常量 `DJI_GEN1 = DeviceProfile { vid: 0x2CA3, pid: 0x4006 }`
  （`crates/domain/src/device.rs`），`matches()` 要求 vid 与 pid 精确相等，`is_supported()` 双重校验
  解析出的 USB VID/PID。测试断言 PID 4009（同 VID 异 PID）与 VID 1234 均**不匹配**。无其他 PID 被接受。

## 3. 第 6 节：发布工程与文档 —— Passed

产物（提交 `20de544` 发布工程、`b9b2a7e` 文档）：

- `deny.toml`：cargo-deny v2，许可白名单（MIT/Apache-2.0/BSD/ISC/Zlib/MPL-2.0/CC0/Unicode 等），
  强 copyleft 与未知许可拒绝；yanked 拒绝；目标锁定 `x86_64-pc-windows-msvc`；仅允许 crates.io。
- `.github/workflows/ci.yml`：`verify` 作业（fmt、clippy `-D warnings`、workspace 全测试、release build、
  cargo-deny、尽力 SBOM）+ `package` 作业（dry-run 测试、未签名构建、verify、上传）。工具缺失显式记
  `Environment-blocked`，绝不静默通过。
- `packaging/scripts/build-msix.ps1`：构建并打包 **panel + helper 两个可执行文件**，默认输出 `dist/`，
  生成无 BOM 的 SHA-256 清单（整包哈希、逐文件哈希、package identity、源 commit）。
- `packaging/scripts/verify-release.ps1`：以 OPC/ZIP 方式检视 MSIX（无需 Windows SDK），对缺文件、
  哈希不符、package identity 不符、意外/调试二进制**失败关闭**。
- 文档：`README.md`（简体中文优先）、`SECURITY.md`、`docs/architecture.md`、`tests/hardware/README.md`
  与 `report-template.md`（A/B/C 组对应交接 7.4/7.3/7.2，全部默认 `Unexecuted`）。

### 3.1 MSIX 构建与验证（本机实际执行）

- 构建：从 HEAD `b9b2a7e`  release 构建 panel + helper，makeappx 打包 6 个载荷文件
  （`AppxManifest.xml`、3 个 Assets PNG、`dji4g-panel.exe`、`dji4g-helper.exe`；容器另含
  `[Content_Types].xml`、`AppxBlockMap.xml`）。
- 产物：`dist/Dji4GPanel-0.1.0.0-unsigned-development-only.msix`
  SHA-256 `3eaaec428c731fd32a3fece391c3de5cf30d75592479f89cc9be9354433ab330`，`signed=false`。
  清单记录的 commit 与 HEAD 完全一致（`b9b2a7e…`）。
- 正向验证：`verify-release.ps1` → `status=passed`，退出 0
  （`hash_ok:*.msix`、`package_identity_ok`、`hash_ok:dji4g-panel.exe`、`hash_ok:dji4g-helper.exe`）。
- 负向验证（篡改清单副本，不污染真实 `dist/`）——全部失败关闭、退出 1：
  - 整包哈希错误 → `verify:hash_mismatch:Dji4GPanel-…msix`
  - helper 哈希错误 → `verify:hash_mismatch:dji4g-helper.exe`
  - package identity 名称错误 → `verify:identity_mismatch:name`
  - 产物缺失 → `verify:artifact_missing:DoesNotExist.msix`
- 未签名姿态：未生成/安装任何证书，未声称已签名或可正式发布；`dist/` 已被 `.gitignore` 忽略，
  产物不入库。

## 4. 第 7.1 节：静态与自动化验收 —— Passed

| 验收项 | 结论 | 依据 |
| --- | --- | --- |
| 普通 debug/release 无 `--demo` 路径绝不构造测试 controller/fake port | Passed | `main.rs` 始终构造 `ProductionComposition`；release（`not(debug_assertions)`）下 `--demo` 直接打印 `DemoRejectedRelease` 并返回；`production_runtime.rs` 守卫断言 |
| 所有生产端口均调用真实 Windows 后端，无固定“不可用”占位 | Passed | `runtime.rs` 委托真实后端；`rg` 无 `dependency_unavailable`/`session_unavailable`/`[0; 32]`/`todo!` 命中 |
| 一代 PID 白名单、无 FriendlyName 回退、无任意 shell/任意 AT | Passed | 见第 2 节静态检查 |
| 错误码稳定且中文可解释；日志不泄露 APN/设备标识/串口内容/nonce/pipe 名 | Passed | 脱敏在 `logging.rs`、`at-protocol/redact.rs` 与协议 `Debug`（`[REDACTED]`）三层；localization_complete 与 redaction 测试通过 |
| 全部第 5 节命令通过 | Passed | 见第 2 节 |
| release verifier 能发现缺文件、错误哈希、错误 package identity、意外调试二进制 | Passed | 缺文件/错误哈希（整包+exe）/错误 identity 已本机负向复现；意外/调试二进制由 `forbiddenPatterns`（`hotspot_spike*`、`*debug*`、`*.pdb` 等）+ `requiredEntries` 白名单拦截 |

## 5. 第 7.2 / 7.3 / 7.4 节：人工与真机验收 —— Unexecuted

以下均**未实际执行**，按交接第 9 节记为 `Unexecuted`，绝不以自动化测试替代：

- **7.2 UI/生命周期人工验收（C 组）**：`Unexecuted`。本环境无 GUI 截图/多 DPI 能力。
  界面布局已由子代理优化（仅用 egui 内置 `Grid`/`horizontal_wrapped`/`Frame` 原语，未自建组件，
  未改动状态/颜色/语义与“是否可用”头部），fmt/test/clippy 全绿；但 100%/125%/150%/200% DPI 无截断、
  中文无方框字、托盘/单实例/自启/睡眠唤醒等**视觉与生命周期行为仍需人工逐项验收**。
- **7.3 UAC/helper 验收（B 组）**：`Unexecuted`。需要真实 UAC 提权与“已安装且签名可信的 helper”。
  注意：因第 6 节已知限制（签名校验缺失），即便安装 helper，B05“允许 UAC 后成功执行”当前也**不可能通过**；
  B01–B04（未安装/未签名/路径不可信/取消 UAC 的失败关闭）可在真机验证，但尚未执行。
- **7.4 PID4006 真机 HIL（A 组）**：`Unexecuted`。会读取/改变真实设备与网络状态，
  **必须先获用户对维护窗口与测试 SIM/设备的明确授权**；未授权前不得执行，只能记 `Unexecuted`。
  逐项矩阵与报告模板见 `tests/hardware/`。

## 6. 已知限制（务必阅读）

1. **特权修复路径整体失败关闭（有意为之）**：`crates/windows-platform/src/privilege.rs` 的
   `TrustedHelper::installed()` 把 `signature_verified` 硬编码为 `false`，因此总是返回“helper 不可信”。
   在实现真正的 Authenticode 签名验证之前，**即使 helper 已安装到受保护路径，特权修复也不可能成功**。
   在当前生产接线中，所有已确认写操作（含标记 `requires_elevation=false` 的 DHCP 续租与热点开关）
   都经同一条 helper 路径，故均以 `privilege:helper_untrusted` 失败关闭；**只读诊断不受影响**。
   这是保守姿态，不是疏漏；不得为“让测试通过”而放宽签名/路径校验。
2. **cargo-deny / SBOM 本机 Environment-blocked**：本机未安装 `cargo-deny` 与 `cargo-cyclonedx`，
   故依赖许可/漏洞检查与 SBOM 在本机记 `Environment-blocked`（绝不伪造通过）；二者已在 `ci.yml` 中
   分别接为硬门禁与尽力门禁，CI（Windows runner）可实际执行。
3. **英文 UI 暂不可选**：localization 建模了 `Language::EnUs`，但 `english_available()` 返回 `false`，
   仅暴露简体中文目录，直到完整英文审校存在。
4. **遗留进程 `hotspot_spike.exe`（交接第 8 节，PID 16152）**：上一阶段经用户授权尝试终止，
   遇 Access Denied 失败；按交接要求**未反复强杀**。它是开发期 spike（`apps/panel/src/bin/hotspot_spike.rs`，
   supervisor/worker 自我重启），**不是正式应用进程，也不打入 MSIX**。如需终止，请在具备相应权限的
   上下文中处理；本次未调用任何热点状态变更。

## 7. 停止/发布判定（交接第 9 节复核）

逐条核对“必须停止并记 Failed”的条件，本次**均未触发**：

- 普通启动未使用 fake/demo，生产端口未固定返回占位错误。
- 未匹配非 `VID_2CA3&PID_4006` 设备，无 FriendlyName/默认路由兜底。
- 热点与修复动作均具备 fresh identity/epoch/before-state 校验。
- 用户取消 UAC → 明确 `OperationCancelled`；超时 → `OutcomeUnknown`；均不自动重试。
- 手机热点/TUN 公网结果不会被记为模块可用（仅 adapter-bound 证据计入）。
- 日志/界面未泄露敏感标识；package/helper 信任边界未放宽。
- 无任何必需自动化/静态项为 `Failed`。

**结论：可称“发布候选”。** 距离“最终验收通过”还差：用户在真机执行（或显式接受其 `Unexecuted`）
7.2/7.3/7.4，并接受 cargo-deny/SBOM 的 `Environment-blocked`（或在 CI 中实际跑通）。
判定权在用户。

## 8. 复现实据（供接手者核对）

```powershell
# 自动化门禁（本机已跑，退出 0）
cargo fmt --all -- --check
cargo test --workspace --offline
cargo check --workspace --all-targets --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo check --workspace --release --all-targets --offline
git diff --check

# 发布候选（本机已跑）
./packaging/scripts/build-msix.ps1 -DryRun     # 计划：panel_exe + helper_exe + output_dir=dist
./packaging/scripts/build-msix.ps1             # 构建未签名候选 + SHA-256 清单
./packaging/scripts/verify-release.ps1         # 正向：status=passed
# 负向：篡改 release-manifest.json 副本的 hash/identity/artifact 后 -Manifest 指向它，应退出 1
```

真机 HIL/UAC/人工 UI 验收请按 `tests/hardware/README.md` 与 `report-template.md` 逐项执行并回填四态结论。

## 9. 真机手工验证增量与托盘缺陷修复（2026-09-04 晚，用户本机）

用户在真实 Windows 本机对 release 构建做手工验证，发现并修复了三个原生 Win32 托盘缺陷
（该路径此前零自动化覆盖，测试只用内存假后端，故漏到真机才暴露）：

| 提交 | 缺陷 | 根因 | 用户复测 |
| --- | --- | --- | --- |
| `bfe554a` | 托盘右键无菜单、无法退出 | 申请了 `NOTIFYICON_VERSION_4` 却按旧约定匹配整个 `lParam`；v4 的事件在 `LOWORD(lParam)`、图标 id 在 `HIWORD`，右键/双击事件全部落空 | 菜单出现 |
| `d23870a` | 菜单文字乱码、点击无效 | `encode_text` 未加 NUL 结尾，`AppendMenuW` 越界读直到随机零字节 | 文字正常、点击生效、退出生效 |
| `d5792c9` | 隐藏到托盘后点"退出"无反应 | 窗口 `Visible(false)` 后 egui 停止重绘，`update()`（唯一轮询托盘事件处）不再运行，退出的队列指令无人读 | 曾报"隐藏后退出正常"，后续全新启动复测**再次失效**（见第 10 节） |

修复 `d5792c9` 采用跨线程唤醒：原生托盘工作线程排队事件后调用钩子，钩子执行
`Context::request_repaint()`。该唤醒只是**尽力而为**：在 eframe 0.29 中 `request_repaint()`
最终只是通过 `EventLoopProxy` 请求一次重绘，而对一个已被 `Visible(false)`（`ShowWindow(SW_HIDE)`）
隐藏的窗口，重绘请求无法产生新的一帧，`update()` 依旧不会运行。此外 `--autostart` 或
`start_minimized` 会让窗口自启动即不可见，此时 `update()` 可能从未运行过，连唤醒钩子都不会被安装。

因此原先据此得出的 HIL **C03（托盘显示/隐藏/退出）** `Passed` 结论**已作废**，
需按第 10 节的硬退出兜底重新手工验证。
**C01（多 DPI 无截断/中文无方框字）、C05（自启 drift）、C06（睡眠/唤醒/拔插）** 仍为 `Unexecuted`。

真机另发现一个**环境/硬件状态**（非代码缺陷，但影响显示准确性）：模块在位且 RNDIS 网络口
`Status OK`（可上网），但负责 AT 诊断的串口（驱动名 Baiwang，`MI_02~MI_05`）为 `Error/Unknown`
→ `select_at_port` 失败 → `pnp:no_safe_at_port`，面板无法完成蜂窝诊断。当前把该状态表面化为
"正在检测/未检测到"，**标签不够准确**（设备其实在位）。把标签改为"设备已识别，但 AT 端口不可用"
涉及 fail-closed 可用性分类器语义，需用户确认后再改；用户侧可先修复/重装 Baiwang 串口驱动使 AT 口恢复。

## 10. 托盘退出第四轮修复：工作线程硬退出兜底

### 根因

前三轮修复都停在"让 UI 线程有机会读到退出指令"，但**优雅退出路径的正确性依赖于 UI 回调还会再运行一次**，
而这恰恰是隐藏窗口无法保证的：

- `PanelApp::update()` 是唯一轮询托盘事件的地方（`poll_shell_events` → `try_recv`）。
- `handle_close_request` 用 `ViewportCommand::Visible(false)` 把窗口隐藏到托盘；winit 随即 `ShowWindow(SW_HIDE)`。
- eframe 0.29 把 `Context::request_repaint()` 接到 `EventLoopProxy::send_event(UserEvent::RequestRepaint{..})`，
  该事件的作用只是请求窗口重绘。**不可见的窗口拿不到新的一帧**，于是 `update()` 不再运行，
  排队中的 `Exit` 永远无人读取——进程既不退出也不报错。
- 更糟的是 `--autostart` 或配置 `start_minimized` 时，`main.rs` 直接以 `.with_visible(false)` 建窗
  （`start_hidden`），`update()` 可能**自始至终一次都没运行**，连 `d5792c9` 的唤醒钩子都不会被安装。
  这与用户"全新启动后再次失效"的现象一致。

结论：`request_repaint` 唤醒不是保证，只是运气（窗口可见时靠 `render()` 末尾的
`request_repaint_after(500ms)` 心跳兜住了）。

### 修复

把"必定退出"的保证从 UI 线程搬到**真正收到用户点击的那个线程**（托盘工作线程 `dji4g-tray`），
使其完全不依赖 egui 重绘：

| 文件 | 改动 |
| --- | --- |
| `crates/windows-platform/src/tray.rs` | 新增 `ExitBackstop` 看门狗状态机与 `arms_exit_backstop` 判定；`push_event` 在排队**之前**对退出项武装看门狗；`run_loop` 每 ~10ms 检查到期并调用新增的 `force_process_exit`（先删除自身托盘图标，再 `std::process::exit(0)`）；新增非阻塞 `WorkerCommand::AcknowledgeExit` / `NativeTray::acknowledge_exit`；`WM_COMMAND` 回退路径改为同样走 `push_event`；`show_menu` 进入模态菜单前先检查到期 |
| `apps/panel/src/tray.rs` | `TrayBackend` 新增默认空实现的 `acknowledge_exit`，`TrayController` 透传，`NativeTrayBackend` 转发到原生层 |
| `apps/panel/src/app.rs` | `TrayEventSource` 新增 `acknowledge_exit`；`handle_tray_command(Exit)` 在置 `explicit_exit` 并发 `ViewportCommand::Close` 后回执确认；修正 `tray_wake_installed` 无条件置真的潜在缺陷 |

关键语义：

- **只有**「退出」(menu id 4) 会武装看门狗；「打开面板」「立即刷新」「热点状态」与 `TaskbarCreated`
  恢复一律不会，未知 id 也不会（已有单元测试钉住）。
- 优雅路径仍然保留并且优先：UI 线程读到 `Exit` 后回执 `AcknowledgeExit`，工作线程据此把截止时间
  **一次性延长** 1.5s（`EXIT_BACKSTOP_ACKNOWLEDGED_GRACE`），使正在推进的正常关闭不被硬退出打断。
- 延长**永不等于取消**：若 UI 回执后 eframe 仍然没能关窗，工作线程到点照样终止进程。
- 窗口隐藏（收不到任何回执）时，原始 1.5s（`EXIT_BACKSTOP_GRACE`）到点即终止。
- 首次「退出」为准，重复点击不会把截止时间往后推，也不存在二次退出竞态
  （`run_loop` 收到 `Shutdown`/`WM_QUIT` 后不再检查看门狗）。
- 硬退出前会先 `NIM_DELETE` 自己的图标，避免通知区残留幽灵图标；`std::process::exit` 跳过析构在此可接受
  （日志每次 append 即 flush，句柄由系统在进程销毁时释放）。

### 自动化覆盖与必须人工验证的部分

`cargo test --workspace --offline` 覆盖纯判定逻辑：`only_the_exit_selection_arms_the_hard_exit_watchdog`、
`idle_backstop_is_never_due`、`armed_backstop_is_due_only_after_the_grace_period`、
`acknowledgement_extends_the_deadline_once_and_never_disarms`、
`acknowledgement_without_an_armed_backstop_is_ignored`、`repeated_exit_selections_cannot_postpone_termination`，
以及面板侧 `exit_acknowledges_the_watchdog_and_requests_a_real_close`、
`non_exit_tray_commands_never_touch_the_shutdown_watchdog`、`exit_without_an_attached_tray_still_requests_a_close`。

原生 Win32 消息泵、`TrackPopupMenu`、`Shell_NotifyIconW` 与 `process::exit` 本身**无法单元测试**，
必须由人工在真机确认（HIL C03 重测）：

1. **可见状态退出**：`target\x86_64-pc-windows-msvc\release\dji4g-panel.exe` 启动，窗口可见时右键托盘图标
   → 「退出」。期望：窗口与进程立即消失（远早于 1.5s，走的是优雅路径），通知区无残留图标。
2. **隐藏到托盘后退出**（本次缺陷的主场景）：启动后点窗口 X 关闭 → 窗口隐藏、托盘图标仍在 →
   右键托盘 → 「退出」。期望：进程在 ~1.5s 内必定消失，通知区无幽灵图标。
3. **自启动即隐藏**：以 `--autostart` 启动（或把配置 `start_minimized` 置真），窗口从不显示 →
   右键托盘 → 「退出」。期望：同上，进程必定消失。
4. **非退出项不得杀进程**：分别点「打开面板」「立即刷新」「热点状态」，确认面板正常响应且**不会**退出。
5. 用任务管理器或 `tasklist | findstr dji4g-panel` 确认第 2、3 步之后确实没有残留进程。

已知残留风险：`TrackPopupMenu` 的模态循环会挂起 `run_loop`，若用户在点「退出」后 1.5s 内**又**打开托盘菜单
并一直悬停不关，则硬退出会推迟到该菜单关闭为止（`show_menu` 入口已加检查，菜单一打开即刻退出）。

## 11. 「正在检测 / 未检测到」的真实根因与修复

第 9 节把该现象归因于**可用性分类器的标签语义**，这个归因**不准确**，需在此更正。
真实主因是：**release 构建一次刷新周期都没有跑过**，因此从未记录任何证据，面板永久停留在初始态。

证据链（全部可在代码中复核）：

1. `apps/panel/src/main.rs` 唯一的启动刷新被 `#[cfg(debug_assertions)]` 包住，release 构建里被编译掉。
2. `crates/application/src/monitor.rs::run` 只在合并后的 `RefreshSignal` 被置位时才 `run_refresh()`；
   全仓 5 处 `UiCommand::Refresh` 发送点中，4 处由用户动作触发（托盘"立即刷新"、二次启动激活、
   底部"刷新"按钮、诊断页刷新按钮），第 5 处即上述被编译掉的启动刷新，**没有任何周期性刷新**。
3. 零刷新 ⇒ `device_presence = None`、`phase = Startup`
   ⇒ `crates/domain/src/availability.rs::classify` 返回 `Detecting`
   ⇒ `crates/application/src/reducer.rs::app_snapshot` 判定 `Freshness::Unknown`
   ⇒ `apps/panel/src/ui/mod.rs::availability_vm` 渲染「正在检测」+「正在收集并校验当前设备的连接证据。」
   +「尚无有效的更新时间」，且 `app.device = None` ⇒ 概览页设备型号「未检测到」、蜂窝字段「未获取」。

第 9 节记录的真机现象（freshness 为「尚无有效的更新时间」而非「状态已过期」）只可能由
**从未成功记录过证据**产生，与"分类器把 AT 故障误显示为未检测到"不符。
另外两项曾被怀疑的成因也已排除：`pnp_spike --json` 报告 `device_count:1`（走的是与面板完全相同的
`WindowsDeviceInventory::scan_now()` / `devices()` 路径），因此既不是 `pnp:ambiguous_device`
多节点歧义，幻影/失效节点也已被 `is_proven_root` 正确排除，**不需要**做去重合并。

修复（AT 端口不可用属于第 9 节所述环境状态，此次一并把语义改准确）：

| 文件 | 变更 |
| --- | --- |
| `crates/application/src/monitor.rs` | 新增 `REFRESH_INTERVAL = 10s` 与纯函数 `periodic_refresh_due`；`run` 在收到用户信号**或**周期到期时刷新；首轮必然到期（`last_refresh_at = None`），故所有构建启动即扫描；无 ports 的测试/演示后端不参与定时 |
| `apps/panel/src/main.rs` | 启动刷新不再受 `debug_assertions` 限制，release 也会调度首次扫描 |
| `crates/application/src/controller.rs` | 新增 `interaction_in_flight(now)`：自动周期在该窗口内让位，避免每次扫描提升 `evidence_revision` 而把用户正要确认的修复计划作废；窗口以计划自身 `expires_at` 为界，被放弃的计划不会永久停掉监控 |
| `crates/domain/src/availability.rs` | 新增 `incomplete_evidence`：在已识别设备的前提下，若存在新鲜且来源正确的 `AtControlAvailability::Unavailable`，则以 `Limited(AtControlUnavailable)` 取代笼统的 `Limited(IncompleteEvidence)` |
| `apps/panel/src/localization.rs` | `LimitedReasonAtControlUnavailable` 文案改为「设备已识别，但 AT 端口不可用（串口异常），无法读取蜂窝状态。」 |
| `docs/architecture.md` | 同步周期刷新与分类器新规则 |

fail-closed 不变量未被放宽：新规则只在两个 `Limited` 原因之间替换，**永远不可能**产出 `Available`；
四个确定性 `Unavailable` 判定都在其之前返回，不会被掩盖；VID/PID 精确匹配、禁止 FriendlyName 与
系统默认路由兜底、封闭 AT/修复命令集、写操作三态结果、不自动重试均原样保留。
周期刷新只驱动只读观测端口，不涉及任何写操作或修复。

新增/调整的回归测试：`availability_matrix.rs`（AT 不可用在各证据缺失分支都要指名故障、永不产出
`Available`、不掩盖 `Unavailable`、错误来源证据不得用于解释）、`monitor_scenarios.rs`（真机场景端到端：
在位设备 + `pnp:no_safe_at_port` ⇒ 识别设备且 `Limited(AtControlUnavailable)` + `Fresh`；周期刷新
无需用户指令即可重扫；无 ports 不定时扫描）、`pnp_fixture_tests.rs`（复刻真机拓扑：一个在位复合根 +
`MI_00` 可用网卡 + `MI_02~MI_05` 串口异常 ⇒ 仍是**一个**设备、`com` 为空、`NoSafePort`）、
`ui_state.rs`（钉住用户可见中文文案，且该状态不得显示为「可用」）、
`confirmation_scenarios.rs`（`interaction_in_flight` 只在计划存活窗口内为真：空闲为假、待确认且未过期为真、
超过 `PLAN_LIFETIME` 为假、取消后为假、操作结束后为假）。

门禁：`cargo fmt --all -- --check` 通过；`cargo clippy --workspace --all-targets -- -D warnings`
（debug 与 release 两种 profile）均退出 0；`cargo test --workspace --offline` 294 项全部通过。

**仍需真机人工复验**（模块插好、用 release 构建）：启动后约 10 秒内表头应离开「正在检测」，
显示「受限」+「设备已识别，但 AT 端口不可用（串口异常），无法读取蜂窝状态。」，
freshness 显示「状态为最新 · 更新于 X秒前」；概览页设备型号应显示「DJI 一代 4G 模块」而非「未检测到」；
静置数分钟不应退回「正在检测」，也不应长期停在「状态已过期」。
若用户修复/重装 Baiwang 串口驱动使 AT 口恢复，则应进一步显示蜂窝信息并按证据升级为「可用」。

另需复验周期刷新引入的两项副作用：（1）在修复页准备好一个操作后，待确认状态应能跨越至少一个
10 秒自动周期而不被后台扫描作废（`interaction_in_flight` 让位生效），点击「确认」仍能正常走完
UAC/三态结果流程；（2）面板空闲（含最小化到托盘）时 CPU 占用应保持很低——`run` 仍然阻塞在
`recv_timeout(250ms)`，新增的只是到期判断，没有恢复忙等。
