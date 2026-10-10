# 密码管理器 · PasswordKey

> 纯本地、离线运行的桌面密码管理工具。数据加密后只保存在本机，不联网、不上云。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Go](https://img.shields.io/badge/Go-1.25%2B-00ADD8?logo=go&logoColor=white)](https://go.dev/)
[![Platform](https://img.shields.io/badge/Platform-Windows-0078D6?logo=windows&logoColor=white)](#环境要求)

基于 **Go + Wails v2** 构建，后端用 **AES-256-GCM** 加密、**Argon2id** 派生密钥，前端是零框架的原生 JavaScript。
程序目录可以整体拷到 U 盘，插到另一台电脑上直接运行。

---

## 目录

- [特性](#特性)
- [技术栈](#技术栈)
- [环境要求](#环境要求)
- [快速开始](#快速开始)
- [使用说明](#使用说明)
- [数据存储与便携模式](#数据存储与便携模式)
- [安全设计](#安全设计)
- [项目结构](#项目结构)
- [开发与测试](#开发与测试)
- [常见问题](#常见问题)
- [参与贡献](#参与贡献)
- [许可证](#许可证)

---

## 特性

**凭证管理**

- 支持 7 种凭证类型：浏览器密码、API Key、SSH 密钥、安全笔记、数据库连接、邮箱账号、银行卡
- 按名称 / 网址 / 用户名模糊搜索（`%` `_` `\` 会被转义，不会被当成通配符）
- 收藏置顶、批量选择与删除
- 密码强度检测（按字符集大小估算熵值分级）
- 随机密码生成器（16 位，包含大小写字母、数字、符号，并保证每类至少出现一次）

**安全**

- 每条凭证独立加密，AES-256-GCM + 随机 nonce
- Argon2id 密钥派生，抵抗 GPU 暴力破解
- 连续 5 次解锁失败锁定 30 秒，状态持久化在数据库，重启不重置
- 闲置自动锁定，时长可选 1 / 5 / 10 / 15 / 30 分钟或永不（默认 5 分钟）
- 查看明文密码、删除凭证需再次验证主密钥
- 复制的密码 30 秒后自动清除剪贴板
- 锁定与退出时清零内存中的密钥
- 安全体检：扫描弱密码与重复使用的密码，输出 0–100 评分

**数据**

- 便携模式：配置与密码库放在程序同级的 `data/` 目录
- 程序目录不可写时自动回退到 `%APPDATA%\PasswordManager`
- 支持自定义数据库路径，并可一键恢复默认
- JSON / Excel 导入导出，附带可下载的导入模板
- 修改主密钥：在单个事务内重新加密全部凭证，失败自动回滚

**桌面集成**

- 系统托盘菜单：显示主窗口 / 立即锁定 / 退出
- 关闭窗口即最小化到托盘，后台常驻
- 单实例运行：重复启动会唤回已有窗口，而不是开第二个进程

## 技术栈

| 层 | 技术 |
|---|---|
| 语言 | Go 1.25+ |
| 桌面框架 | [Wails v2.12](https://wails.io) |
| 前端 | 原生 JavaScript + Vite 3（无框架） |
| 数据库 | SQLite（[modernc.org/sqlite](https://gitlab.com/cznic/sqlite)，纯 Go 实现，`CGO_ENABLED=0`） |
| 加密 | AES-256-GCM + Argon2id（`golang.org/x/crypto`） |
| 表格导出 | [excelize v2](https://github.com/qax-os/excelize) |
| 系统托盘 | [energye/systray](https://github.com/energye/systray) |
| 打包 | Wails build + NSIS |

## 环境要求

| 依赖 | 版本 | 说明 |
|---|---|---|
| Windows | 10 / 11 x64 | 当前只面向 Windows 发行 |
| Go | 1.25 或更高 | 见 `go.mod` |
| Node.js | 16 或更高 | 前端使用 Vite 3 |
| Wails CLI | v2.12 | `go install github.com/wailsapp/wails/v2/cmd/wails@latest` |
| NSIS | 任意近期版本 | 可选，仅生成安装包时需要 |
| WebView2 运行时 | — | Win11 自带；Win10 安装包会自动安装 |

> 非 Windows 平台可以编译通过（`tray_other.go` / `idle_other.go` 提供了空实现），
> 但托盘与闲置自动锁定不会生效，因此不作为支持目标。

## 快速开始

```bash
git clone https://github.com/hunterlihao/PasswordKey.git
cd PasswordKey

# 安装 Wails CLI
go install github.com/wailsapp/wails/v2/cmd/wails@latest

# 开发模式（前端热重载）
wails dev

# 生产构建 → build/bin/password.exe
wails build

# 生成 Windows 安装包（需要 NSIS）
makensis build/windows/installer/project.nsi
```

也可以直接用 Makefile：

```bash
make dev        # wails dev
make build      # wails build
make installer  # build + makensis
make release    # clean + build + installer
```

## 使用说明

### 首次使用

1. 启动程序，设置主密钥（建议 12 位以上、混合字符类型）。
2. **主密钥不会被保存到任何地方，也没有找回渠道。** 忘记它就只能删除密码库重新开始，请务必自行记录。
3. 之后每次启动输入主密钥解锁即可。

### 导入导出格式

**JSON** —— 对象数组：

```json
[
  {
    "name": "GitHub",
    "type": "browser",
    "url": "https://github.com",
    "username": "user@example.com",
    "secret": "your_password",
    "notes": "个人账号"
  }
]
```

`type` 可选值：`browser`、`api_key`、`ssh_key`、`secure_note`、`database`、`email`、`bank_card`。
留空或无法识别时按 `browser` 处理。

**Excel** —— 首行是表头，数据从第二行开始：

| name | type | url | username | secret | notes |
|---|---|---|---|---|---|
| GitHub | browser | https://github.com | user@example.com | your_password | 个人账号 |

导入时会跳过 `name` 或 `secret` 为空的记录（这两项必填）。

> ⚠️ 导出文件中的密码是**明文**。请把导出文件放在安全位置，用完及时删除。

## 数据存储与便携模式

启动时按以下顺序确定数据目录：

1. **便携模式** —— 若程序同级的 `data/` 目录可写，就用它。
2. **回退模式** —— 若该目录不可写（典型情况是程序装在 `C:\Program Files`），
   则改用 `%APPDATA%\PasswordManager`，并在 `data/` 下留一个 `system_dir` 标记文件，
   之后启动直接走回退路径，不再反复尝试写程序目录。

`data/` 目录内容：

| 文件 | 说明 |
|---|---|
| `settings.json` | 数据库路径、自动锁定时长 |
| `data.db` | SQLite 密码库（加密后） |
| `startup.log` | 启动诊断日志，只记录启动阶段节点，**不含任何凭证信息** |

数据库使用 SQLite **WAL** 模式。更改或恢复数据库路径时，会连同 `-wal` / `-shm` 文件一起复制，
避免丢失尚未 checkpoint 的写入；复制出的副本以 `0600` 权限落盘。

> **注意 `0600` 在 Windows 上的实际含义**
>
> `0600` 是 Unix 语义。在 Windows 上 Go 只会把其中的「所有者可写」位映射成文件的只读属性，
> 并不设置访问控制列表 —— 文件的实际权限来自所在目录继承的 ACL。
> 因此不要把程序放在所有人可读的目录里。真正的机密性由数据库加密本身保证，
> 文件权限只是额外一层（在 `%APPDATA%` 回退模式下，用户配置目录默认已限制其他用户访问）。

> **关于"便携"的准确说法**
>
> 密码库与配置完全随 `data/` 目录走，这部分是真便携。
> 但 Wails 使用的 WebView2 内核会在 `%APPDATA%\password.exe\EBWebView\` 下建立自己的
> 缓存与 profile（Wails 的默认行为）。该目录只有渲染引擎缓存，**不包含任何凭证数据**。
> 若追求零残留，把 `password.exe` 和 `data/` 拷走后手动删掉这个目录即可。

## 安全设计

### 加密方案

| 环节 | 做法 |
|---|---|
| 密钥派生 | Argon2id：time cost 3、memory 64 MiB、parallelism 4，16 字节随机盐 → 32 字节密钥 |
| 凭证加密 | 每条凭证的密码**独立**加密：AES-256-GCM，随机 12 字节 nonce，密文布局为 `nonce ‖ ciphertext ‖ tag` |
| 主密钥校验 | 不直接比较密钥，而是单独保存一段 32 字节随机 token 的密文；解锁时尝试解密，成功即密钥正确 |
| 修改主密钥 | 在单个 SQLite 事务内重新加密全部凭证并替换盐与校验值，任何一条失败则整体回滚 |

### 运行时防护

- 解锁失败累计 5 次锁定 30 秒，计数与锁定时间持久化在数据库，重启进程不会重置。
- 闲置自动锁定按**系统级**空闲时间（`GetLastInputInfo`）计算：看视频、在别的程序里操作、
  鼠标移出窗口都会被正确计入，而不是只看窗口内是否有事件。
- 锁定、退出时清零内存中的主密钥字节。
- 查看明文密码、删除凭证需要再次输入主密钥验证。
- 复制密码后 30 秒自动清空剪贴板（若剪贴板内容仍是本次复制的值）。
- 安全体检全程只在内存中处理明文，单条处理完立即清零；重复密码检测用 SHA-256 摘要分组，
  不用明文做 map key；返回给前端的只有掩码（形如 `a***z`，长度归一化，防止通过长度猜密码）。

### 威胁模型与边界

**可以防**

- 密码库文件被单独拷走、被同步进网盘、被误发给别人 —— 没有主密钥就解不开，这是核心防线
- 程序目录被整个复制后，在另一台机器上离线分析

**不能防 / 需要注意**

- 本机恶意软件、键盘记录器、屏幕录制
- 解锁状态下对进程内存的抓取或调试
- 主密钥被钓鱼、肩窥或社工获取
- 你自己导出的明文 JSON / Excel 文件泄露
- **文件系统层面的访问控制**：程序用 `0600` 创建数据文件，但如前所述这在 Windows 上不生效，
  实际权限取决于所在目录的 ACL。请把程序放在受控目录，不要放在所有人可读的位置；
  只要主密钥足够强，即使 db 文件被拿走也难以破解

> ⚠️ **本项目是个人作品，未经过任何第三方安全审计。**
> 用于保存关键凭据前请自行评估风险，并保留独立的加密备份。

## 项目结构

```
PasswordKey/
├── main.go                      # 程序入口：窗口选项、单实例锁、关闭即隐藏
├── app.go                       # 业务逻辑：解锁、凭证 CRUD、导入导出、改主密钥
├── storage.go                   # 数据目录解析（便携 / 回退）与可写探测
├── lock.go                      # 锁定并向前端广播事件
├── autolock.go                  # 闲置自动锁定监控
├── idle_windows.go              # Windows 系统级空闲时长（GetLastInputInfo）
├── idle_other.go                # 非 Windows 平台空实现
├── tray.go                      # 系统托盘（消息泵绑定 OS 线程）
├── tray_other.go                # 非 Windows 平台空实现
├── audit.go                     # 安全体检：弱密码 + 重复密码 + 评分
├── storage_test.go              # 数据目录与降级逻辑测试
├── update_credential_test.go    # 凭证更新与导出回归测试
├── internal/
│   ├── crypto/crypto.go         # AES-256-GCM 加解密、Argon2id 派生、随机密码生成
│   ├── store/store.go           # SQLite 建表、CRUD、解锁状态持久化
│   ├── model/model.go           # 凭证模型、类型枚举、密码强度检测
│   ├── model/audit.go           # 体检报告模型、掩码、评分算法
│   ├── settings/settings.go     # settings.json 读写
│   └── export/export.go         # JSON / Excel 导入导出与模板
├── frontend/
│   ├── index.html               # 入口页面（含启动动画）
│   ├── vite.config.js
│   ├── src/main.js              # 前端全部逻辑（原生 JS，无框架）
│   ├── src/style.css
│   └── wailsjs/                 # Wails 自动生成的绑定，请勿手改
├── build/
│   ├── appicon.png
│   └── windows/                 # 图标、应用清单、版本信息、NSIS 安装脚本
├── winres/winres.json           # Windows 资源定义（当前构建未引用，保留备用）
├── wails.json                   # Wails 项目配置
├── Makefile
├── go.mod
└── LICENSE
```

## 开发与测试

```bash
gofmt -l .        # 应无任何输出
go vet ./...
go test ./...
```

现有测试覆盖：

- `storage_test.go` —— 目录可写探测、不可写时的降级判定、降级标记短路、便携目录名与 `.gitignore` 一致性
- `update_credential_test.go` —— 编辑保存时留空密码不会覆盖原密码（密文逐字节保留）、显式改密码生效、
  收藏状态不被编辑操作清掉、编辑后导出仍带密码

提交前请确保上面三条命令全部通过。

## 常见问题

**忘记主密钥了怎么办？**

无法找回。程序不保存主密钥，也没有后门或恢复码 —— 这正是它安全的前提。
只能删除 `data.db`（或 `%APPDATA%\PasswordManager\data.db`）重新初始化，原有凭证将全部丢失。

**数据文件到底在哪？**

打开设置页，「数据库存储位置」会显示当前实际路径。默认是程序同级 `data\data.db`，
回退模式下是 `%APPDATA%\PasswordManager\data.db`。设置页也会标明当前处于「便携版」还是系统目录。

**怎么迁移到另一台电脑？**

把 `password.exe` 和 `data/` 整个目录一起拷过去即可。
如果之前自定义过数据库路径，记得把那个 db 文件也一并拷走。

**为什么数据跑到了 `%APPDATA%`？**

程序所在目录不可写（通常是装在了 `C:\Program Files`）。
把程序移到可写目录，并删除 `data/system_dir` 标记文件，即可恢复便携模式。

**导入 JSON 提示部分记录失败？**

`name` 或 `secret` 为空的记录会被跳过，这两项是必填的。另外无法识别的 `type` 会按 `browser` 处理，不会报错。

**托盘图标在，但点击没反应？**

这是 Wails 托盘消息泵的线程绑定问题，已在 `tray.go` 中通过 `runtime.LockOSThread()` 修复。
如果你基于旧代码构建，请更新到最新版本。

## 参与贡献

欢迎提交 Issue 和 Pull Request。

- 提交前请确保 `gofmt -l .` 无输出，且 `go vet ./...`、`go test ./...` 通过
- 新增功能请附带对应的测试
- 改动涉及加密、密钥管理或数据存储时，请在 PR 描述中说明安全影响
- 请保持现有的代码风格：中文注释、说明「为什么」而非「做了什么」

## 许可证

[MIT License](LICENSE) © 2025 liyuanhao
