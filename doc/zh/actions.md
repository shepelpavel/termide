# 自定义脚本

脚本系统允许您将自定义脚本添加到 TermIDE 的菜单栏。脚本在新的终端面板中执行，方便您直接从 TermIDE 运行构建命令、部署脚本或任何自动化任务。

## 快速入门

### 脚本目录位置

将您的脚本放置在脚本目录中：

| 平台 | 路径 |
|----------|------|
| Linux | `~/.local/share/termide/scripts/` |
| macOS | `~/Library/Application Support/termide/scripts/` |
| Windows | `%APPDATA%\termide\scripts\` |

您也可以通过菜单栏中的 `选项 → 管理脚本` 访问此文件夹。

### 创建第一个脚本

```bash
# 创建脚本目录
mkdir -p ~/.local/share/termide/scripts

# 创建一个简单脚本
cat > ~/.local/share/termide/scripts/hello.sh << 'EOF'
#!/bin/bash
echo "Hello from TermIDE!"
echo "Current directory: $(pwd)"
read -p "Press Enter to close..."
EOF

# 设置可执行权限（Unix 系统必需）
chmod +x ~/.local/share/termide/scripts/hello.sh
```

创建脚本后，重启 TermIDE 或使用 `选项 → 管理脚本` 刷新。您的脚本将出现在**脚本**菜单中。

## 脚本命名

菜单中的显示名称由文件名生成：

| 文件名 | 显示名称 |
|----------|--------------|
| `build.sh` | build |
| `deploy.sh` | deploy |
| `run-tests.py` | run-tests |
| `my.cool.script.sh` | my |

显示名称是文件名中第一个点号之前的部分。

## 目录结构（分组）

您可以使用子目录将脚本组织成分组。每个子目录成为一个子菜单：

```
~/.local/share/termide/scripts/
├── build.sh              # 显示在脚本菜单根目录
├── deploy.sh             # 显示在脚本菜单根目录
├── docker/               # 创建 "docker" 子菜单
│   ├── up.sh
│   ├── down.sh
│   └── logs.sh
└── git/                  # 创建 "git" 子菜单
    ├── pull.sh
    ├── push.sh
    └── status.sh
```

**注意：** 仅支持一级子目录。嵌套子目录将被忽略。

## 执行模式

脚本根据文件名后缀支持不同的执行模式：

### 后台执行 (`.bg.`)

对于希望在后台运行的长时间进程，在文件名中添加 `.bg.`：

| 文件名 | 执行模式 |
|----------|----------------|
| `server.sh` | 前台（新终端面板） |
| `server.bg.sh` | 后台（无终端面板） |
| `deploy.bg.sh` | 后台 |

后台脚本运行时不打开终端面板，适用于：
- 启动开发服务器
- 运行监视进程
- 启动后台服务

### 报告脚本 (`.report.`)

对于应在后台运行并在模态对话框中显示输出的脚本，在文件名中添加 `.report.`：

| 文件名 | 执行模式 |
|----------|----------------|
| `check.sh` | 前台（新终端面板） |
| `check.report.sh` | 后台并显示模态输出 |
| `status.report.sh` | 后台并显示模态输出 |

报告脚本：
- 在后台运行，不阻塞 UI
- 捕获 stdout 和 stderr
- 完成时在信息模态框中显示输出
- 在模态框标题中显示成功（✓）或失败（✗）指示器

**使用场景示例：**
- 快速状态检查（`git status`、`docker ps`）
- 代码检查或验证脚本
- 系统健康检查
- 任何希望查看结果的短时间运行脚本

**示例：**
```bash
# 创建一个报告脚本
cat > ~/.local/share/termide/scripts/check.report.sh << 'EOF'
#!/bin/bash
echo "Checking system status..."
echo "Date: $(date)"
echo "User: $(whoami)"
echo "PWD: $(pwd)"
EOF
chmod +x ~/.local/share/termide/scripts/check.report.sh
```

## 平台特定说明

### Unix (Linux/macOS)

脚本必须具有可执行权限：

```bash
chmod +x ~/.local/share/termide/scripts/myscript.sh
```

任何具有可执行位的文件都会出现在菜单中，与扩展名无关。

### Windows

在 Windows 上，以下文件扩展名被识别为可执行文件：
- `.sh`（需要 WSL 或 Git Bash）
- `.bat`
- `.cmd`
- `.ps1`（PowerShell）
- `.py`（Python）
- `.rb`（Ruby）
- `.pl`（Perl）

## 工作目录

脚本以当前会话的根目录作为工作目录执行。这通常是您启动 TermIDE 时所在的目录，或通过 `会话 → 更改根路径` 选择的目录。

## 使用技巧

1. **添加 shebang 行**：始终以 shebang 开头（例如 `#!/bin/bash`），以确保使用正确的解释器运行。

2. **保持输出可见**：对于前台脚本，在末尾添加 `read -p "Press Enter..."` 以在终端关闭前查看输出。

3. **使用描述性名称**：第一个点号之前的文件名成为菜单标签，请使用清晰、描述性的名称。

4. **使用分组组织**：使用子目录对相关脚本进行分组（例如 `docker/`、`npm/`、`git/`）。

5. **服务器用后台模式**：对于启动长时间运行进程的脚本，在文件名中使用 `.bg.`。

6. **快速检查用报告模式**：对于希望在模态框中查看结果的脚本，在文件名中使用 `.report.`。

## 故障排除

### 脚本未出现在菜单中

1. 检查文件是否具有可执行权限（`chmod +x`）
2. 确保文件在正确的目录中
3. 重启 TermIDE 以刷新脚本列表

### 脚本运行失败

1. 先在终端中手动测试脚本
2. 检查 shebang 行是否正确
3. 确保所有必需的工具/解释器已安装

### 后台脚本似乎不工作

后台脚本静默运行。请检查：
1. 脚本确实启动了预期的进程
2. 进程没有立即退出
3. 如需查看进程输出，请检查系统日志

### 报告脚本的模态框未出现

报告脚本在完成时显示模态框：
1. 等待脚本执行完毕
2. 检查脚本是否退出太快（如需要可添加小延迟）
3. 确保脚本向 stdout 或 stderr 产生输出
