# DJI 一代 4G 面板

面向 Windows 的大疆第一代 4G 模块管理工具。在一个界面中查看设备与网络状态、收发短信，并按引导排查连接问题。

[项目官网](https://lincodex.cn/index.php/archives/59/) · [下载](https://github.com/zhu-hailin/dji4g-gen1-panel/releases/latest) · [使用说明](docs/使用说明.txt) · [更新记录](https://github.com/zhu-hailin/dji4g-gen1-panel/releases)

## 主要功能

- **设备概览**：查看模块、SIM 卡、网络连接、温度和实时收发速率。
- **短信管理**：收发、搜索和阅读短信，可选择保存本地历史。
- **网络诊断**：检查模块连接、电脑网络与代理，导出诊断报告。
- **连接修复**：按引导检查驱动、网络配置和常见连接问题。
- **无线观测**：查看信号强度、服务小区及变化趋势。
- **个性化设置**：支持浅色、深色、跟随系统，以及简体中文、繁體中文和 English。

## 界面预览

以下预览使用模拟数据。

![设备与网络概览（模拟数据）](docs/screenshots/overview-013.png)

<details>
<summary>查看深色界面</summary>

![深色概览（模拟数据）](docs/screenshots/overview-dark-013.png)

</details>

## 快速上手

1. 从 [下载页面](https://github.com/zhu-hailin/dji4g-gen1-panel/releases/latest) 获取 Windows x64 便携 ZIP，完整解压并保留包内文件。
2. 插入 SIM 卡，用支持数据传输的 USB 线连接模块，运行 `dji4g-panel.exe`。
3. 按首次连接引导检查设备；进入面板后，可通过“诊断”排查联网问题，通过“短信”管理消息。

如需安装驱动，请按程序提示处理。详细操作见 [使用说明](docs/使用说明.txt)。

## 软件更新

程序在启动及运行期间后台检查更新。发现更新后，点击更新入口下载，并按提示安装。

也可以从 [下载页面](https://github.com/zhu-hailin/dji4g-gen1-panel/releases/latest) 手动获取更新。手动替换程序前，请从设置或托盘完全退出旧程序。

## 支持范围

支持 Windows x64 和大疆第一代 4G 模块，通用模块可查看只读信息。

修改设备或网络配置时，先查看影响说明，再确认执行。

## 从源码构建

需要 Rust MSVC 工具链及 Visual Studio C++ 构建工具。在项目根目录执行：

```powershell
cargo build --workspace --release --locked --target x86_64-pc-windows-msvc
```

构建产物位于 `target/x86_64-pc-windows-msvc/release`，打包脚本见 [packaging/scripts](packaging/scripts)。

## 反馈与许可

问题和建议请提交到 [Issues](https://github.com/zhu-hailin/dji4g-gen1-panel/issues)。分享日志或截图前，请隐藏号码及设备身份信息；安全漏洞报告方式见 [安全说明](SECURITY.md)。

项目采用 MIT OR Apache-2.0 双许可证，见 [LICENSE-MIT](LICENSE-MIT) 与 [LICENSE-APACHE](LICENSE-APACHE)。
