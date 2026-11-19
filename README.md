# IPSwitcher 网卡 IP 快速切换工具

基于 **Tauri v2 + Vue 3 + TypeScript + Vite + Element Plus** 的 Windows 桌面工具，支持网卡 IP/DNS 快速切换、多网卡批量管理、方案保存与常用网络诊断。

## 技术栈

- 桌面框架：Tauri v2（Rust 后端，Windows 原生接口 / netsh / CIM-WMI）
- 前端：Vue 3 + TypeScript + Vite + Element Plus
- 适配系统：Windows 10 / Windows 11 x64

## 快速开始

### 环境要求

- Node.js 20.19+ / 22 LTS
- Rust 1.77+（MSVC 工具链，Visual Studio 生成工具）
- WebView2 运行时

### 命令

```bash
# 安装前端依赖
npm install

# 开发模式（启动 Vite 与 Tauri 窗口）
npm run app:dev

# 仅构建前端静态产物
npm run build

# 发布构建（编译 release 并生成 NSIS 安装包）
npm run app:build
```

## 目录结构

```
src/                      前端源码
├── api/                  Tauri IPC 封装
├── components/           界面组件（网卡列表 / IP 配置 / 网络诊断）
├── composables/         共享状态
├── styles/               全局样式
└── types/                类型定义

src-tauri/                Rust 后端
├── src/
│   ├── main.rs / lib.rs  入口、托盘、命令注册、关闭到托盘
│   ├── netcfg.rs         网卡读取、静态 IP / DHCP / DNS 切换
│   ├── diag.rs           诊断命令异步执行与流式输出
│   ├── elevate.rs         UAC 提权与待决操作执行
│   ├── config.rs         配置持久化（便携版同级目录 / 安装版 AppData）
│   └── types.rs          共享数据结构
├── icons/                多尺寸图标与 icon.ico
├── capabilities/         Tauri 权限配置
└── tauri.conf.json       应用与打包配置
```

## 核心后端命令

| 命令 | 说明 |
| --- | --- |
| `list_adapters` | 枚举网卡及当前配置（CIM/WMI + Get-NetAdapter） |
| `apply_static` | 应用静态 IP（可同步 DNS） |
| `apply_dhcp` | 切换 DHCP，可选同时恢复 DNS 自动获取 |
| `apply_dns` | 仅修改 DNS 服务器 |
| `run_diagnostic` | 异步执行诊断命令，逐行回流 `diag-line` 事件 |
| `elevate_action` | 落盘待决操作并以管理员身份重启执行（UAC） |
| `get_config` / `save_config_cmd` | 读取 / 保存本地配置 |
| `check_admin` | 判断当前是否具备管理员权限 |

## 配置持久化

- 便携版：可执行文件同级目录 `config.json`（存在 `portable.marker` 标记时启用）
- 安装版：`%APPDATA%\com.jamhoo.ipswitcher\config.json`

## 产物

| 产物 | 命名 |
| --- | --- |
| 便携版压缩包 | `IPSwitcher_v2.0.0_portable.zip` |
| NSIS 安装包 | `IPSwitcher_v2.0.0_setup.exe` |

详细使用说明见 [使用说明.md](./使用说明.md)。
