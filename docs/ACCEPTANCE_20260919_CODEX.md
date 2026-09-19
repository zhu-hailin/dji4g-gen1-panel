# 0.1.2 发布验收

日期：2026-09-19。验收对象为短信布局与三档设备工具交付，发布基线为 GitHub main `a064a3b350837ac216c34e1aeb49299319dff80d`。

## 验收中修复

1. 单字符 AT 输入会切片越界：新增测试先复现 panic，再增加长度校验。
2. `ATO0`、`ATO1`、`ATD`、`ATDL`、`ATDT...` 等基本交互命令未被拦截：扩展族级检查，并保留正常扩展查询。
3. 取消排队工具任务后忙碌状态不释放；取消运行中任务未通知 worker：补齐状态和控制信号，批量取消不再发剩余命令；写出后的未知结果保留。两条新流程测试先失败、修复后通过，涵盖写入前与写入后。
4. 异常提示符、CONNECT 和无法解析的响应没有最终码，不能推断固件不支持或操作无效果：结果统一为 OutcomeUnknown，不自动重试。新增测试先失败、修复后通过。
5. 同步 workspace、锁文件、MSIX identity、打包与校验脚本版本为 0.1.2；先前打包版本不匹配已由校验器检出并修复。

## 本地重新执行的检查

| 检查 | 结果 |
| --- | --- |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --workspace --all-targets --offline --locked -- -D warnings` | 通过，无警告 |
| `cargo test --workspace --all-targets --offline --locked` | 795 通过，0 失败，0 忽略 |
| `cargo build --workspace --release --offline --locked` | 通过 |
| `test-driver-completion.ps1` | 6 项通过，无设备修改 |
| `test-driver-install-command.ps1` | 3 项通过，模拟进程退出码 |
| `build-msix-dry-run.ps1` | 通过 |
| MSIX 构建与 `verify-release.ps1` | 通过，identity 与内含文件哈希一致 |
| 公开便携 ZIP 构建 | 通过，逐文件内容哈希核验 |

本地发布包：`dist/codex-0.1.2-final/`。ZIP SHA-256 为 `5dc8f20087f66f6bdc493a29eaf4493acf63c0a13dab03856db7c479dd347100`；MSIX SHA-256 为 `b825bb84e7427e3fd2a2ca6c7829a650bb504202e15b77ee6bf904c84367eb1e`。GitHub 将重新构建，远程包哈希以 Release 附件为准。

## 界面与证据边界

复核原交付宽屏短信列表、窄屏详情与专家确认截图，并重新构建及运行当前 ui_capture，生成 1100×1000 逻辑点设备工具五种状态。界面展示模拟数据，不接设备。列表增长与边界由布局测试覆盖。

本轮未执行真实模块查询、AT 写入、短信发送、驱动安装或网络配置变更。新电脑、不同固件、安全软件兼容性仍需实机确认。公开包不带厂商驱动；本地旧独立版 EXE 保留，未冒充本次修复后的产物。

GitHub 发布流程仍需对最终提交执行 CI、依赖检查、打包与 Release 上传；本文件只记录已完成的本地检查，不提前宣称远程成功。
