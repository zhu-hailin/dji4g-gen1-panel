# 设备属性检测修订

用户诊断报告（2026-09-18 08:20 UTC）显示：USB 父设备和五个子接口均为 Code 0；MI_02 是 Quectel AT 串口 COM6，MI_04 是 Baiwang 网卡。面板在 USB 识别阶段返回 pnp:property_invalid，其余阶段为 app:stage_missing。历史安装日志的四个异常接口不能当作当前状态。

代码核查发现，设备枚举在关联 DJI 拓扑之前读取了全部设备的可选属性，并把只有终止符的空字符串判为损坏。Windows 字符串属性契约为以 NULL 终止的 Unicode 字符串：https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devprop-type-string 。回归测试复现了空字符串导致的同一错误码，但报告没有记录失败属性的名称和原始字节，不能断言远端触发点已被完全复现。

本次修订先通过 ConfigMgr 获取设备身份与祖先链，再仅为 DJI 相关设备读取属性；合法空字符串转为缺省值。类型、UTF-16、终止符、硬件 ID 和拓扑归属校验保留，目标设备属性损坏仍返回失败。没有更改驱动、USB 模式、APN 或网络配置。

验证：平台单元测试 167 项通过；平台所有 targets 的 clippy -D warnings 通过；本机只读设备枚举成功。远端 AT 通信、短信及面板识别恢复仍待复测。

使用：退出旧面板（包括托盘），插着模块打开“设备识别修复”独立 EXE，刷新即可，不需重装驱动。如仍未识别，导出新版日志。
