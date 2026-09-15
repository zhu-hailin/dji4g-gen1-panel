# HIL 测试报告模板（每个测试项复制一份）

使用说明：复制本文件为 `tests/hardware/reports/<编号>-<简述>-<YYYYMMDD>.md`（编号见 `tests/hardware/README.md` 索引），**填写每一个字段，不得留空或删字段**。结论只允许四态之一：`Passed` / `Failed` / `Environment-blocked` / `Unexecuted`。未实际执行的项只能记 `Unexecuted`；工具/硬件/环境缺失记 `Environment-blocked`；绝不伪造通过。所有敏感值（APN、IMEI、ICCID、手机号码、SSID、设备序列号、nonce、pipe 名、可识别用户身份的 IP 地址）必须脱敏为 `<REDACTED>` 之类的占位。

---

## 空白模板（从此行以下复制）

- 测试项：
- 执行时间（含时区）：
- commit：
- Windows 版本：
- 安装方式（MSIX/便携/开发）：
- 模块硬件 ID（脱敏）：
- 驱动版本：
- 连接方式（直插/扩展坞/具体 USB 口）：
- Clash/Meta/TUN 状态：
- 前置授权确认（维护窗口/测试 SIM 已获用户授权：是/否）：
- 步骤：
  1.
  2.
  3.
- 预期：
- 实际：
- 截图/日志位置：
- 敏感值脱敏确认（已核对截图/日志/导出无未脱敏敏感值：是/否）：
- 四态结论（`Passed`/`Failed`/`Environment-blocked`/`Unexecuted`）：
- 备注：

---

## 示例（仅演示格式，全部为伪造/脱敏值，不是真实执行结果）

- 测试项：A24 公网绑定探测成功但模块 DNS 失败时显示“数据链路可达、DNS 失败”
- 执行时间（含时区）：2026-09-10 14:30（Asia/Shanghai, UTC+08:00）
- commit：`0000000`（示例占位）
- Windows 版本：Windows 11 x64 23H2（示例）
- 安装方式（MSIX/便携/开发）：MSIX（示例）
- 模块硬件 ID（脱敏）：`USB\VID_2CA3&PID_4006\<REDACTED>`
- 驱动版本：`0.0.0.0`（示例占位）
- 连接方式（直插/扩展坞/具体 USB 口）：直插，机身左侧第 1 个 USB-A 口（示例）
- Clash/Meta/TUN 状态：Clash 关闭，无 TUN 适配器（示例）
- 前置授权确认（维护窗口/测试 SIM 已获用户授权：是/否）：是（示例）
- 步骤：
  1. 在测试 SIM 上将 DNS 指向不可达地址 `<REDACTED>`（模拟模块 DNS 失败），保持公网绑定探测目标可达（示例）。
  2. 打开面板，等待一个完整探测周期，观察“是否可用”与原因文本。
  3. 打开诊断视图，核对 gateway/public/DNS 各项及 IPv4/IPv6 coverage 分列显示。
- 预期：面板不显示绿色可用；显示 Limited，原因为“数据链路可达、DNS 失败”（中文、具体），不出现“驱动故障”之类笼统表述；诊断视图中绑定公网探测为通过、绑定 DNS 为失败。
- 实际：与预期一致（示例）。
- 截图/日志位置：`tests/hardware/reports/evidence/A24-panel.png`、`%LOCALAPPDATA%\Dji4GPanel\logs\<REDACTED>.log`（示例，均已脱敏）
- 敏感值脱敏确认（已核对截图/日志/导出无未脱敏敏感值：是/否）：是（示例）
- 四态结论（`Passed`/`Failed`/`Environment-blocked`/`Unexecuted`）：`Passed`（示例）
- 备注：APN 为 `<REDACTED>`；本机 IP 等识别性地址已在截图中打码（示例）。
