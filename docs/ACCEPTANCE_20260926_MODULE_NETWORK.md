# v0.1.7 模块网卡检查与逐项修复验收

## 基线与隔离

从已发布 v0.1.6 的 7eda3c6288b14e74bb929630adf82d9d0c104b90 建立独立工作树和 codex/module-network-check-017 分支。原开发工作树的全部未提交改动保持原样；没有 reset、clean、stash、批量暂存或子代理。

本地 docs/implementation-20260926/baseline 保存初始 Git 状态、HEAD、全量源码归档和逐文件 SHA-256。归档、完整日志与 BMP 截图留本地，不包含在公开源码提交中。

## 行为与回归证据

- 领域 module_network_check 分类：未执行/无法获取/过期不判为成功；无模块、网卡问题、断链、无地址/路由、网关、公网、DNS 分别归类，电脑出口不参与模块可用性判断。
- Controller/Runner：检查以请求 ID、设备代次和刷新周期绑定。重复点击合并，短信/工具/修复排队，设备变化与 30 秒过期阻止旧修复。
- 单次授权回归验证探测调用恰好一次、持久设置仍关闭、后续关闭探测周期不沿用旧“可用”证据；独立报告不被后台周期覆盖。最后一项先复现 Available 误用，再修正并验证。
- 修复建议：必须有明确 DHCP 或 DNS 配置证据；静态地址与未知配置不建议覆盖。准备命令携带检查 ID 与封闭动作类型，执行仍使用已有的目标身份、过期、回读与确认检查。
- 修复后只安排一次只读检查，保存独立操作结果；第二次确认拒绝、执行计数保持一次。结果未知也不会自动重复写入。
- 新排队回归先复现“短信读取排队尚未运行时刷新先行”，再改为使用权威 serial_work_busy 门禁；用例通过。
- Runtime 保留每个协议族针对测试目标选择的路由结果，测试断言 IPv4 模块出口与 IPv6 VPN 出口不被压成一个结果。原有平台 Fake 测试覆盖 GUID 识别、名称变化、断开与无地址/路由；没有更改本机网络。
- UI 在概览、诊断、首次引导复用同一组件。主要按钮至少 32 逻辑像素高，证据默认折叠；未知操作与复检分开说明，驱动入口只进入已有检查/匹配流程。

初始新功能用例在缺少 API 时编译失败，随后实现通过；未删除失败用例、扩大超时或跳过验证。

## 视觉验收

ui_capture 的 8 个模块场景（正常 VPN、DHCP、DNS、未识别、接口缺失、关闭探测、排队、未知操作后复检）分别生成 800×600 100% 和 150% 截图；另有 1100×760、1440×1000 正常场景，共 18 张。输出位置为本地 docs/implementation-20260926/screenshots-final。尺寸已核对。主要结论与按钮可见，正常首屏可见信号、上下行和温度。所有场景均为 Noop 模拟数据，不连接设备、不发送短信、不安装驱动。

README 图片为其中的正常 VPN 模拟场景，明确标注模拟数据。

## 验证与交付状态

以下命令均返回 0：

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --offline --locked -- -D warnings`
- `cargo test --workspace --all-targets --offline --locked --no-fail-fast`：67 组结果、964 项测试通过，0 失败、0 忽略。
- `git -c core.whitespace=cr-at-eol diff --check`
- `cargo build --workspace --release --offline --locked`

本地交付目录：`dist/20260926-module-network-145521/`。

- 独立版：`大疆4G面板-v0.1.7.exe`，SHA-256 `9B65DD0B3B28AA56940223B51130AE2F5D67B63934B6031642932AFEC029C9E6`。
- 面板 release 与 payload 均为 `DA754253CF0797DEDB7B8C26417C10DD6E42092183A924FCAFC0CB64E007E414`，产品版本回读为 0.1.7。
- 本地驱动来源仍为已有 `dist/local-driver-kit-20260918`；新安装器 `--check`、逐文件压缩包回读校验通过。没有下载或安装驱动。
- 新建进程级 `LOCALAPPDATA`，连续两次运行 `--verify-bundle`：退出码均为 0，均得到 17 个文件。没有启动真实面板。
- 交付目录的 `local-delivery-manifest.json` 保留 panel/helper/driver-setup 的 release 与 payload 对应哈希；`bundle-verification.json` 保存两次结果。

GitHub 发布由对应源码提交的 CI 门禁控制。工作流成功和附件实际存在后才对用户报告发布完成，具体结果另保存到本地验证记录。含厂商驱动的本地包不公开上传。没有使用自动测试代替实机验收。

待实机：不同电脑的驱动安装与接口匹配、真实模块公网/DNS、实际 DHCP 更新和网卡重启后的 USB 状态、双模块插拔、IPv4/IPv6 的真实出口组合。正常 VPN/代理节点或某网站的可达性不在模块通路通过的承诺范围内。
