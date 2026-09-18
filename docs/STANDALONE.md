# 单文件独立版

交付文件为 `大疆4G面板独立版.exe`。Windows 10/11 x64 用户只需复制这个文件并双击，不需要解压 ZIP、安装应用、复制依赖或创建快捷方式。程序将内置资源准备到当前用户应用数据目录，再直接打开面板。缺少模块驱动时可从面板内启动附带的离线驱动安装器；安装系统驱动仍需要管理员授权和匹配的硬件 ID。

启动器使用 Rust 文件 API，不执行 PowerShell 安装脚本。它逐字节验证内置资源，拒绝启动被修改的文件；启动面板期间保留只读文件句柄。驱动安装步骤仍包含经校验的 PowerShell/PnPUtil 流程。

四个 EXE 均使用静态 CRT 构建，不依赖另装 `VCRUNTIME140.dll`。这不意味着支持任意操作系统、硬件或驱动配置。尚未通过全新 Windows 电脑和卡巴斯基实机验收，应用 EXE 仍未签名。

## 使用与排查

1. 双击独立 EXE，首页“开始使用模块”按 USB、网卡、AT 串口、SIM/蜂窝、公网、DNS 显示当前实测结果和下一步。
2. 先使用数据线直连电脑并刷新。无法识别 USB 或无法上网不自动等于缺少驱动。
3. 确认缺驱动时进入“修复”，点击“退出面板并安装驱动”。确认后面板正常退出，安装器等待该进程退出再请求管理员授权；不强制关闭其他程序。发送短信、执行/确认修复、导出日志期间不允许此退出流程。
4. 安装窗口显示成功、失败、取消或需要重启的实际结果。重新打开独立 EXE 并刷新，分别验证 AT 与网络；不以安装器结束代替设备验收。
5. 顶部“导出详细日志”不依赖设备识别成功。后台导出一个 UTF-8 TXT；进度和完成后的“打开所在文件夹”“复制日志路径”保留在底部。包含检测阶段/时间/错误码、USB 硬件 ID、驱动版本与签名、串口、IP/DNS/路由、安全软件状态、SetupAPI 相关记录及历史安装日志。每节有独立超时，权限不足也保留完整错误并继续其他节。文件包含本机设备和网络信息，不采集短信正文、电话号码或 SIM 标识。

导出末尾 `REPORT_COMPLETE` 仅表示收集结束，不表示各检查通过。历史日志有数量和大小上限，截断会明确标记。主程序没有运行时监测到的历史不能补造。

## 本地构建步骤

先准备一个已验证、含驱动的便携包目录，保留 `portable-manifest.json`。厂商驱动文件不进源码仓库；须有相应分发授权才能公开发布含驱动的二进制。

在仓库根目录为主程序、helper 和驱动安装器生成静态 CRT 版本。以下输出目录需预先创建，路径也可替换为绝对路径：

```powershell
cargo rustc -p dji4g-panel --bin dji4g-panel --release --offline -- -C target-feature=+crt-static --emit=link=dist/static/dji4g-panel.exe
cargo rustc -p dji4g-helper --bin dji4g-helper --release --offline -- -C target-feature=+crt-static --emit=link=dist/static/dji4g-helper.exe
cargo rustc -p dji4g-panel --bin dji4g-driver-setup --release --offline -- -C target-feature=+crt-static --emit=link=dist/static/dji4g-driver-setup.exe
```

用这三个 EXE 更新便携包目录，重新生成其文件 SHA-256 清单，再运行：

```powershell
./packaging/scripts/build-standalone.ps1 -PayloadDirectory <便携包目录> -Destination <新输出文件.exe>
```

该脚本校验清单后将资源直接嵌入启动器，并以静态 CRT 构建启动器。`--verify-bundle` 只准备和校验资源，不打开面板、不提权、不安装驱动。测试可临时将进程的 `LOCALAPPDATA` 指向隔离目录，验证首次启动和重复启动；不要修改系统的环境变量。

## 本次验证

- 单文件资源首次准备、重复使用、中文路径、篡改拒绝、目录穿越拒绝。
- 驱动安装失败/取消阻止自动打开面板；接口状态异常拒绝报成功。
- 原始驱动 SHA-256/CAT 校验和篡改拒绝；未改动本机正常驱动。
- 未完成：卡巴斯基复测、新电脑实际驱动安装、短信送达和网络验收。
