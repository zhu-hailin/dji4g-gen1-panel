# 0.1.6 电脑网络与代理诊断验收

## 本轮结果

- 模块公网及 DNS 验证已通过时，仅有 VPN/TUN 默认路由不会再把模块状态判为“连接受限”。领域和应用回归测试先复现旧实现失败，再通过修复。
- 新增独立的电脑网卡、IPv4/IPv6 默认路由、系统代理和 Clash Verge Rev 固定出口诊断。模块未连接、网卡断开、目标网卡消失与读取证据不足分别呈现；不把代理端口或虚拟路由当作互联网连通证明。
- Clash Verge Rev v2.5.5 简单扩展配置的来源核验、单字段编辑、备份、写后核验与撤销代码已实现。隔离文件测试覆盖注释及 CRLF 保留、重复键/脚本歧义拒绝、备份恢复和外部修改冲突。**官方客户端隔离端到端验收尚未完成，发布版的自动修改入口保持关闭**；目前只提供诊断和处理步骤。
- 诊断页在 800×600 的 100% 和 150% 缩放下可看到结论与重新检查；模拟修复预览的主要按钮也在首屏内。示例只使用 Noop 命令接收器及模拟快照，不连接模块或修改代理。
- README 已加入作者提供的[项目官网](https://lincodex.cn/index.php/archives/59/)，并把设备工具中的模糊“专家终端”入口统一命名为“AT 命令（高级）”。

## 自动检查

在以 GitHub 0.1.5 为基线的独立发布工作树中执行：

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --offline --locked -- -D warnings` | 通过 |
| `cargo test --workspace --all-targets --offline --locked --no-fail-fast` | 通过 |
| `git -c core.whitespace=cr-at-eol diff --check` | 通过 |
| `packaging/tests/build-msix-dry-run.ps1` | 通过 |
| 现有驱动完成、安装命令和计划脚本测试 | 通过 |
| 本机 `cargo-deny` | 未安装；GitHub CI 保留该硬门槛 |

`apps/panel/examples/ui_capture.rs` 在 800×600（100% 与 150%）、1100×760 和 1440×1000（100%）下为正常 TUN、失效出口、不支持客户端、修复预览、待重启、撤销冲突、无模块共生成 28 张模拟截图。文件位于本地 `docs/implementation-20260925/network-proxy-20260925-113238/screenshots/`；调整预览布局后另存 `screenshots-after-layout/`。

首次 GitHub CI 在现有串口 actor 测试中暴露了模拟输入缺失：测试提交第二条监测查询，却只排队了一条工具响应，导致模拟串口读空。补齐第二条模拟响应并核对查询结果后，该测试连续运行 20 次通过，全工作区测试再次通过；没有扩大超时或跳过用例。

第二次 GitHub CI 中，两项支持报告测试在共享 runner 上启动 Windows PowerShell 超过原有 8 秒期限。它们验证的是子进程输出、退出码和超时处理，现改由测试程序自身作为固定子进程提供成功、失败与挂起场景；生产探测命令和期限均未改变。相关测试及全工作区测试在本地重新验证。

## 本地交付

交付目录：`dist/20260925-network-assistance-122511/`。

| 文件 | SHA-256 |
|---|---|
| 公开便携 ZIP `public/dji4g-panel-windows-x64-portable.zip` | `C4C3EAADE9A33A39B1F0336EA31DCEA0420B943A81921599DFE4C8BDD8F030C4` |
| 本地含已校验资源 ZIP `local/dji4g-panel-windows-x64-local-offline.zip` | `9A4EB298BB47EC8BE38DCFA080256DBB8E1A693E8E5BEB15001C9B6D19E17DE0` |
| 本地单文件 `local/大疆4G模块管理-0.1.6-本地验收版.exe` | `C1DCA2EB867564FD005FE0F95CB3A86B7D74C67AAC14BCD89ECC472FC654B6F4` |

release 面板、helper、驱动检查程序与 payload 对应文件的 SHA-256 分别为 `EB51CA77693A2DC82FDD4E4565E6C31BE40C847FA1672B0266A2B799D66164A8`、`E79EE1D987E8DD2ABA9430647151C9841C21DD4DD1DEDA21E452D573D3ED37CE`、`9D7FD6E06E179D54066C6D5067974B498182B93F2AA571B267E2C57C84616F75`。安装器 `--check` 通过，没有执行安装。隔离进程级 `LOCALAPPDATA` 下两次等待进程退出的 `--verify-bundle` 均以退出码 0 校验 17 项资源；未启动真实面板。桌面旧 0.1.5 已备份到交付目录，仅保留一个 `大疆4G面板.exe`，其版本和哈希为 0.1.6 及上述单文件值。

公开 GitHub 包仍不包含厂商驱动；本地含驱动版不对外上传。真实网络恢复、官方客户端“修复→重启→复检→撤销”、真实短信/模块操作和新电脑驱动兼容性均未由这些自动测试证明。
