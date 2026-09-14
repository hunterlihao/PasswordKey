# 密码管理器 (Password Manager)

一款本地离线密码管理工具，基于 Go + Wails v2 构建，使用 AES-256-GCM 加密存储。

## 功能特性

- 本地加密存储，数据不上传云端
- AES-256-GCM 加密 + Argon2id 密钥派生
- 支持 7 种凭证类型：浏览器密码、API Key、SSH 密钥、安全笔记、数据库连接、邮箱账号、银行卡
- 密码强度检测与随机密码生成
- 收藏置顶、搜索过滤
- 批量选择与删除
- JSON / Excel 导入导出（含模板下载）
- 修改主密钥（自动重新加密所有凭证）
- 自定义数据库存储路径（设置中可更改）
- 便携版：数据存放在程序同级的 `data/` 目录，整个文件夹可拷到 U 盘直接运行
- 闲置自动锁定，时长可在设置中调整（1/5/10/15/30 分钟或永不）
- 安全体检：扫描弱密码与重复使用的密码，给出安全评分
- 关闭窗口最小化到系统托盘，后台常驻；托盘菜单支持显示窗口、立即锁定、退出
- 单实例运行：重复启动时自动唤回已运行的窗口

## 技术栈

| 组件 | 技术 |
|------|------|
| 后端 | Go + Wails v2 |
| 数据库 | SQLite (modernc.org/sqlite, 纯Go) |
| 加密 | AES-256-GCM + Argon2id |
| 前端 | Vanilla JS + Vite |
| Excel | excelize/v2 |
| 打包 | Wails build + NSIS |

## 快速开始

### 前置要求

- Go 1.23+
- Node.js 18+
- Wails CLI: `go install github.com/wailsapp/wails/v2/cmd/wails@latest`

### 开发

```bash
wails dev
```

### 构建

```bash
wails build
```

输出: `build/bin/password.exe`

### 生成安装包

需要安装 [NSIS](https://nsis.sourceforge.io/Download)

```bash
makensis build/windows/installer/project.nsi
```

## 项目结构

```
password/
├── main.go                         # 程序入口（窗口选项、单实例锁）
├── app.go                          # 业务逻辑层
├── storage.go                      # 便携版数据目录解析与旧数据迁移
├── storage_test.go                 # 迁移逻辑单元测试
├── tray.go                         # 系统托盘（Windows）
├── tray_other.go                   # 非 Windows 平台空实现
├── lock.go                         # 锁定并广播事件到前端
├── autolock.go                     # 闲置自动锁定监控
├── idle_windows.go                 # Windows 系统级空闲时长
├── idle_other.go                   # 非 Windows 平台空实现
├── audit.go                        # 安全体检（弱密码 / 重复密码）
├── go.mod
├── internal/
│   ├── crypto/
│   │   └── crypto.go               # AES-256-GCM 加密/解密 + Argon2id
│   ├── model/
│   │   ├── model.go                # 数据结构定义
│   │   └── audit.go                # 体检报告模型与评分
│   ├── store/
│   │   └── store.go                # SQLite CRUD 操作
│   ├── settings/
│   │   └── settings.go             # 配置文件读写
│   └── export/
│       └── export.go               # JSON/Excel 导入导出
├── frontend/
│   ├── index.html
│   ├── src/
│   │   ├── main.js                 # 前端逻辑
│   │   └── style.css               # 样式
│   └── wailsjs/                    # Wails 自动生成的 Go 绑定
└── build/
    ├── appicon.png                 # 应用图标
    └── windows/
        ├── icon.ico
        ├── wails.exe.manifest
        └── installer/
            └── project.nsi         # NSIS 安装脚本
```

## 安全设计

- **密钥派生**: 用户主密钥经 Argon2id 派生为 AES-256 密钥（32字节）
- **加密存储**: 每条凭证的密码独立加密，使用随机 12 字节 nonce
- **内存安全**: 锁屏时密钥内存清零，剪贴板 30 秒后自动清除；改密/改路径时密钥副本使用后即清零
- **暴力破解防护**: 连续输错密码 5 次后锁定 30 秒，锁定状态持久化到数据库（重启应用不重置）
- **事务保护**: 修改主密钥全程在数据库事务中执行，失败自动回滚
- **数据路径**: 便携版默认 `<exe 同级>/data/`，可在设置中自定义；迁移时连同 `-wal`/`-shm` 一并复制，副本权限 0600

## 便携版说明

数据目录优先选在 **exe 同级的 `data/`**，配置（`settings.json`）与密码库（`data.db`）都在其中。
把 exe 和 `data/` 一起拷走即可在另一台机器上继续使用，不写注册表、不留系统目录残留。

- **首次启用**：若 `data/` 尚不存在数据，而旧版 `%APPDATA%\PasswordManager` 有数据，
  会自动把配置与数据库（含 `-wal`/`-shm`）复制一份过来。旧目录保持不动，可自行删除。
- **自动回退**：若程序装在 `C:\Program Files` 等只读位置，会自动改回
  `%APPDATA%\PasswordManager`，并在 `data/` 留下 `system_dir` 标记，
  之后不再反复尝试。设置页会显示当前处于「便携版」还是「系统目录」。
- **恢复便携**：把 exe 移到可写目录并删除 `data/system_dir` 标记即可。

## 许可证

MIT License
