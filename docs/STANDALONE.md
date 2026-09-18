# 单文件独立版

交付文件为 `大疆4G面板独立版.exe`。Windows 10/11 x64 用户只需复制这个文件并双击，不需要解压 ZIP、安装应用、复制依赖或创建快捷方式。程序将内置资源准备到当前用户应用数据目录，再直接打开面板。缺少模块驱动时可从面板内启动附带的离线驱动安装器；安装系统驱动仍需要管理员授权和匹配的硬件 ID。

启动器使用 Rust 文件 API，不执行 PowerShell 安装脚本。它逐字节验证内置资源，拒绝启动被修改的文件；启动面板期间保留只读文件句柄。驱动安装步骤仍包含经校验的 PowerShell/PnPUtil 流程。

四个 EXE 均使用静态 CRT 构建，不依赖另装 `VCRUNTIME140.dll`。这不意味着支持任意操作系统、硬件或驱动配置。尚未通过全新 Windows 电脑和卡巴斯基实机验收，应用 EXE 仍未签名。

## 本地构建

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
