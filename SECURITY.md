# 安全策略（Security Policy）

本文档说明本项目的支持范围、漏洞报告方式、信任边界与隐私承诺。技术背景见 `docs/architecture.md` 与 `docs/superpowers/specs/2026-09-03-dji4g-gen1-panel-design.md`。

## 支持范围

- 仅 **Windows**（x86-64，`x86_64-pc-windows-msvc`）。
- 仅**大疆第一代 4G 模块**，精确 USB 身份 `USB\VID_2CA3&PID_4006`。其他设备不在支持范围内，程序也不会对其执行写操作。
- 当前只有**未签名开发候选**（0.1.0），没有正式发布版本。安全问题请针对本仓库主分支的最新状态报告与修复。

## 报告漏洞（请私密报告）

- 请**不要**为安全问题创建公开 issue、公开讨论或包含漏洞细节的公开 pull request。
- 本项目目前未公布专用安全邮箱。请通过本仓库托管平台提供的**私密渠道**联系仓库维护者（例如平台的私密漏洞报告功能，或维护者资料中公布的私密联系方式）。
- 报告请尽量包含：受影响的版本或 commit、复现步骤、影响面与前置条件、是否已公开披露；随附日志/截图前请先脱敏（APN、IMEI、ICCID、序列号等）。
- 在修复或公告发布之前，请给予合理的协调披露窗口。非安全类问题请使用仓库的常规反馈渠道。

## 信任边界与安全设计

### 1. 权限分离与 helper IPC

面板（`dji4g-panel.exe`）以 `asInvoker` 运行；需要特权的操作由一次性提权 helper（`dji4g-helper.exe`，`runas` 启动，每进程只执行一个封闭动作后退出）完成。面板与 helper 之间的每次操作使用一条独立的命名管道，并施加以下全部约束（实现见 `crates/ipc/` 与 `crates/windows-platform/src/privilege.rs`）：

- 管道名随机（私有语法 `\\.\pipe\dji4g-panel-<32 位十六进制>`），仅限本地客户端（`PIPE_REJECT_REMOTE_CLIENTS`）、首个实例标志、单实例、消息模式；显式 DACL 仅授予当前用户、Administrators 与 SYSTEM。
- 每次操作生成 32 字节随机 nonce 与随机 request id；协议版本固定 V1；请求携带签发/过期时间，操作生存期上限 60 秒，时钟偏移容忍 5 秒；帧大小上限 32 KiB；未知字段一律拒绝（`deny_unknown_fields`）；同一连接上的第二帧、第二个客户端都会导致拒绝。
- 双向对端校验：PID、进程创建时间、用户 SID 哈希、会话 ID、完整性级别（提权 helper 必须为 High，面板侧为 Medium）以及可执行镜像的 SHA-256。
- helper **不信任面板传来的任何目标描述**：它自行重新枚举设备，要求 `VID_2CA3&PID_4006` 唯一命中，并本地重新计算身份哈希、epoch 与操作前状态哈希，与请求中的证明逐一比对后才执行一次并回读。

### 2. 封闭命令集（无任意 AT、无任意 shell）

- AT 命令是封闭白名单枚举（`crates/at-protocol`）：只读查询 + 三个类型化写命令（模块重启、APN 修改、usbnet profile 切换）。不存在原始 AT 输入路径；只读命令最多在静默期后重试一次，**写命令绝不自动重试**。
- 修复动作是封闭类型化集合（`RepairAction` / `HelperActionV1`）：DHCP 续租、DNS 配置、精确网卡重启、设备重新枚举、usbnet `0/1` 切换、非活动 IP PDP 上下文的 APN 修改、模块重启、热点开关。helper 不接受任意命令行、脚本、注册表路径、文件路径、设备或原始 AT 字符串。
- 每个写操作的结果必为 `Applied / Failed / OutcomeUnknown` 三态之一；超时记为 `OutcomeUnknown`，不会自动重发。
- 检测与写入只绑定精确设备身份与精确网卡（GUID/LUID）；不存在 FriendlyName 匹配或系统默认路由兜底，避免把手机热点、普通 Wi‑Fi 或 Meta/Clash TUN 误认为模块。

### 3. 无可信签名 helper 时失败关闭（含已知缺口）

- `TrustedHelper` 只接受“规范路径位于 Program Files 之下且 `signature_verified` 为真”的 helper；否则以 `privilege:helper_untrusted` 拒绝，用户可写目录或便携目录中的同名可执行文件绝不会被提权。
- **已知缺口（有意为之）**：`TrustedHelper::installed()`（`crates/windows-platform/src/privilege.rs`）当前把 `signature_verified` 硬编码为 `false`，因此**总是失败关闭**——即使 helper 已安装到受保护路径，特权修复路径也无法成功，直到实现真正的 Authenticode 签名验证。这是刻意的保守姿态，不是疏漏：宁可暂无修复能力，也不对未验证签名的镜像执行特权操作。请勿将该缺口本身作为新漏洞重复报告；但如果你发现**绕过**该失败关闭检查、或在缺口补齐后能绕开签名/路径校验的途径，请务必私密报告。

### 4. 隐私与脱敏

- 日志、调试输出与诊断导出**绝不**泄露以下明文值：APN、IMEI、ICCID、电话号码、SSID、设备序列号、nonce、管道名。它们在日志落盘层（`apps/panel/src/logging.rs`）、AT 文本层（`crates/at-protocol/src/redact.rs`）与协议类型的 `Debug` 实现（输出 `[REDACTED]`）分别脱敏。
- 号码与 ICCID 在 UI 中**默认掩码显示**：仅在用户显式点击 [显示]/[复制] 时，明文短暂出现在屏幕或剪贴板（且要求该记录与当前快照的 SIM 会话/观测周期相关）；日志、调试输出与诊断导出仍保持不含明文。
- 短信正文与发送方属于 `MessageContent` 敏感级：相关 AT 交易（CMGF/CMGL/CMGR/CMGD/CMGS）在传输层整行 `[REDACTED]`，`SmsMessage` 的 `Debug`/`Serialize` 均脱敏，诊断导出只包含计数与状态；UI 中发送方默认掩码，正文仅在用户点击消息后展开，[复制] 为显式动作。删除与发送均需用户二次确认；失败或超时不会自动重试。
- 无遥测、无分析、无云端账户、无自动更新；诊断数据不会自动上传。
- 配置：`%APPDATA%\Dji4GPanel\config.toml`；日志：`%LOCALAPPDATA%\Dji4GPanel\logs\`（有界滚动保留）。

## 我们特别关注的问题类型

- 任何能对非 `VID_2CA3&PID_4006` 设备或错误网卡执行写入的路径。
- 绕过确认、epoch、身份哈希或操作前状态哈希校验的路径；使不可重复写操作被自动重试的路径。
- helper IPC 的重放、伪造、跨用户/跨会话/跨完整性级别攻击，以及确认与执行之间的 TOCTOU 竞态。
- 上述敏感值在日志、UI、错误信息或诊断导出中的泄露。

---

**English summary:** Report security vulnerabilities privately to the repository maintainers via the hosting platform's private channel; do not open public issues for security bugs. Scope: Windows x86-64 and the DJI first-generation 4G module (`USB\VID_2CA3&PID_4006`) only. Trust boundaries: a one-shot elevated helper over a per-operation random named pipe with a random nonce, bounded lifetime, and peer/image/integrity/user/session validation; closed AT and repair command sets; three-state outcomes with no automatic retries; fail-closed without an installed, signature-verified helper. Logs, debug output and diagnostics exports redact APN, IMEI, ICCID, phone numbers, SSIDs, serials, nonces, and pipe names; phone numbers and ICCIDs are shown masked in the UI by default and appear in plaintext only on an explicit user display/copy action.
